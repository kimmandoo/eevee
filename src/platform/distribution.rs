//! Explicit lifecycle operations for checksum-owned native installations.
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

mod lifecycle;
pub use lifecycle::{LifecycleReview, Operation, apply_review, prepare_rollback, prepare_update};

const LATEST_URL: &str = "https://github.com/kimmandoo/eevee/releases/latest";
const TAG_PREFIX: &str = "https://github.com/kimmandoo/eevee/releases/tag/";
const MAX_BINARY: u64 = 256 * 1024 * 1024;
const MAX_METADATA: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct Version([u64; 3]);

impl Version {
    fn parse(text: &str) -> Result<Self, String> {
        let mut parts = text.split('.');
        let mut version = [0; 3];
        for number in &mut version {
            let part = parts.next().ok_or("Expected a stable X.Y.Z version")?;
            if part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
            {
                return Err("Expected a stable X.Y.Z version without leading zeroes".into());
            }
            *number = part.parse().map_err(|_| "Version component is too large")?;
        }
        if parts.next().is_some() {
            return Err("Expected exactly three version components".into());
        }
        Ok(Self(version))
    }

    fn tag(text: &str) -> Result<Self, String> {
        Self::parse(
            text.strip_prefix('v')
                .ok_or("Release tag must start with v")?,
        )
    }
}

#[derive(Debug)]
struct Marker<'a> {
    version: Version,
    binary_hash: &'a str,
    metadata_hashes: (&'a str, &'a str),
}

fn marker_hash<'a>(line: Option<&'a str>, prefix: &str) -> Result<&'a str, String> {
    let hash = line
        .and_then(|line| line.strip_prefix(prefix))
        .ok_or("Invalid ownership hash field")?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("Ownership hashes must be lowercase SHA-256".into());
    }
    Ok(hash)
}

fn parse_marker(text: &str) -> Result<Marker<'_>, String> {
    if text.len() > 4096 || !text.ends_with('\n') {
        return Err("Invalid or oversized native installation marker".into());
    }
    let mut lines = text.lines();
    let format = lines.next().ok_or("Missing native installation marker")?;
    if format != "ev-native-install-v2" {
        return Err("Unknown native installation marker format".into());
    }
    let version = Version::parse(
        lines
            .next()
            .and_then(|line| line.strip_prefix("version=ev "))
            .ok_or("Invalid installed version field")?,
    )?;
    let binary_hash = marker_hash(lines.next(), "sha256=")?;
    let metadata_hashes = (
        marker_hash(lines.next(), "license-sha256=")?,
        marker_hash(lines.next(), "notices-sha256=")?,
    );
    if lines.next().is_some() {
        return Err("Unexpected fields in native installation marker".into());
    }
    Ok(Marker {
        version,
        binary_hash,
        metadata_hashes,
    })
}

fn linked(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn inspect(path: &Path, directory: bool) -> Result<Metadata, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("Cannot inspect {}: {error}", path.display()))?;
    if linked(&metadata) || (directory && !metadata.is_dir()) || (!directory && !metadata.is_file())
    {
        return Err(format!(
            "Refusing linked or non-regular installation path: {}",
            path.display()
        ));
    }
    #[cfg(unix)]
    if !directory {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(format!(
                "Refusing multiply linked installation file: {}",
                path.display()
            ));
        }
    }
    Ok(metadata)
}

fn inspect_ancestry(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(
            "Installation directory must be an absolute path without dot components".into(),
        );
    }
    for ancestor in path.ancestors() {
        inspect(ancestor, true)?;
    }
    Ok(())
}

fn hash_file(path: &Path, limit: u64) -> Result<String, String> {
    if inspect(path, false)?.len() > limit {
        return Err(format!(
            "Installation file exceeds its size limit: {}",
            path.display()
        ));
    }
    let mut file = File::open(path)
        .map_err(|error| format!("Cannot open installation file: {error}"))?
        .take(limit + 1);
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("Cannot hash installation file: {error}"))?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > limit {
            return Err("Installation file grew beyond its size limit".into());
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Installation {
    directory: PathBuf,
    version: Version,
    owner: String,
}

fn owned_installation(executable: &Path) -> Result<Installation, String> {
    let expected_name = if cfg!(windows) { "ev.exe" } else { "ev" };
    if executable
        .file_name()
        .is_none_or(|name| name != expected_name)
    {
        return Err("This executable is not an installer-owned ev binary".into());
    }
    let directory = executable
        .parent()
        .ok_or("Cannot determine installation directory")?;
    inspect_ancestry(directory)?;
    inspect(executable, false)?;
    let metadata_dir = directory.join(".ev-install");
    inspect(&metadata_dir, true)?;
    let mut count = 0;
    for entry in fs::read_dir(&metadata_dir)
        .map_err(|error| format!("Cannot read installation ownership: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("Cannot inspect installation metadata: {error}"))?;
        if ![".native-install", "LICENSE", "THIRD_PARTY_NOTICES"]
            .iter()
            .any(|name| entry.file_name() == *name)
        {
            return Err(
                "Installation metadata contains unrecognized files; refusing changes".into(),
            );
        }
        inspect(&entry.path(), false)?;
        count += 1;
    }
    if count != 3 {
        return Err("Native installation ownership metadata is incomplete".into());
    }
    let marker_path = metadata_dir.join(".native-install");
    if inspect(&marker_path, false)?.len() > 4096 {
        return Err("Oversized installation marker".into());
    }
    let mut text = String::new();
    File::open(&marker_path)
        .map_err(|error| format!("Cannot open installation marker: {error}"))?
        .take(4097)
        .read_to_string(&mut text)
        .map_err(|error| format!("Cannot read installation marker: {error}"))?;
    let marker = parse_marker(&text)?;
    if hash_file(executable, MAX_BINARY)? != marker.binary_hash {
        return Err("Executable no longer matches its ownership marker; refusing changes".into());
    }
    let (license, notices) = marker.metadata_hashes;
    if hash_file(&metadata_dir.join("LICENSE"), MAX_METADATA)? != license
        || hash_file(&metadata_dir.join("THIRD_PARTY_NOTICES"), MAX_METADATA)? != notices
    {
        return Err("Installation metadata was modified; refusing changes".into());
    }
    Ok(Installation {
        directory: directory.to_path_buf(),
        version: marker.version,
        owner: hash_file(&marker_path, 4096)?,
    })
}

fn current_installation() -> Result<Installation, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("Cannot locate running executable: {error}"))?;
    let installation = owned_installation(&executable)
        .map_err(|error| format!("Native lifecycle operation refused: {error}. Source builds and unowned/copied binaries cannot update or uninstall themselves."))?;
    if installation.version != Version::parse(env!("CARGO_PKG_VERSION"))? {
        return Err(
            "Running version does not match installation ownership; refusing changes".into(),
        );
    }
    Ok(installation)
}

fn tag_from_location(location: &str) -> Result<String, String> {
    let tag = location
        .strip_prefix(TAG_PREFIX)
        .ok_or("Latest release redirected outside the exact Eevee repository")?;
    Version::tag(tag)?;
    Ok(tag.to_owned())
}

fn bounded_resolve(address: &str) -> std::io::Result<Vec<std::net::SocketAddr>> {
    use std::net::ToSocketAddrs;
    let address = address.to_owned();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    // libc DNS cannot be interrupted. Never let it hold up the CLI indefinitely;
    // a timed-out worker is detached and cannot publish or mutate anything.
    std::thread::Builder::new()
        .name("ev-release-dns".into())
        .spawn(move || {
            let result = address
                .to_socket_addrs()
                .map(|addresses| addresses.collect());
            let _ = sender.send(result);
        })?;
    receiver.recv_timeout(Duration::from_secs(8)).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::TimedOut, "Release DNS lookup timed out")
    })?
}

fn release_tag(animate: bool) -> Result<(String, bool), String> {
    if let Some(value) = std::env::var_os("EV_VERSION") {
        let tag = value
            .into_string()
            .map_err(|_| "EV_VERSION must contain a UTF-8 release tag")?;
        Version::tag(&tag)?;
        return Ok((tag, true));
    }
    if animate {
        return animated_release_tag().map(|tag| (tag, false));
    }
    // Do not follow redirects: validate the exact HTTPS repository/tag before
    // passing it to the embedded installer. No response body is needed or read.
    let response = ureq::AgentBuilder::new()
        .https_only(true)
        .resolver(bounded_resolve)
        .redirects(0)
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(15))
        .timeout_write(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .get(LATEST_URL)
        .set("User-Agent", concat!("eevee/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|_| "Could not resolve the latest Eevee release over HTTPS; no changes made")?;
    if response.status() != 302 {
        return Err(format!(
            "Latest release returned unexpected HTTP {}; no changes made",
            response.status()
        ));
    }
    let locations = response.all("Location");
    let [location] = locations.as_slice() else {
        return Err("Latest release must provide exactly one tag location".into());
    };
    Ok((tag_from_location(location)?, false))
}

#[cfg(windows)]
struct ReleaseLookup(PathBuf);

#[cfg(windows)]
impl ReleaseLookup {
    fn new() -> Result<Self, String> {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random)
            .map_err(|error| format!("Cannot create private release lookup: {error}"))?;
        let path =
            std::env::temp_dir().join(format!("ev-release-{:032x}", u128::from_le_bytes(random)));
        fs::create_dir(&path).map_err(|error| format!("Cannot stage release lookup: {error}"))?;
        Ok(Self(path))
    }
}

#[cfg(windows)]
impl Drop for ReleaseLookup {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0.join("tag"));
        let _ = fs::remove_dir(&self.0);
    }
}

fn animated_release_tag() -> Result<String, String> {
    #[cfg(windows)]
    use std::io::Write;
    use std::process::Stdio;

    // Reuse the installer's exact artwork, timing, terminal checks and cleanup
    // during the initial release lookup as well as the subsequent download.
    #[cfg(unix)]
    let mut command = {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            include_str!("../../install.sh"),
            "ev-installer",
            "--resolve-release",
        ]);
        command
    };
    #[cfg(windows)]
    let lookup = ReleaseLookup::new()?;
    #[cfg(windows)]
    let mut command = {
        let root = std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
        let powershell = PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        inspect(&powershell, false)?;
        let mut command = Command::new(powershell);
        let output = lookup.0.join("tag");
        let output = output
            .to_str()
            .ok_or("Release lookup path is not valid Unicode")?;
        let expression = format!(
            "[Console]::InputEncoding=[Text.UTF8Encoding]::new($false); & ([scriptblock]::Create([Console]::In.ReadToEnd())) -ResolveRelease -ResolveOutput '{}'",
            output.replace('\'', "''")
        );
        command.args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &expression,
        ]);
        command.stdin(Stdio::piped());
        command
    };
    #[cfg(not(any(unix, windows)))]
    return Err("Native installation lifecycle is unsupported on this platform".into());

    installer_environment(&mut command);
    #[cfg(unix)]
    command.stdout(Stdio::piped());
    let mut child = command
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("Cannot start release lookup: {error}"))?;
    #[cfg(windows)]
    {
        let result = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("Installer stdin unavailable"))
            .and_then(|mut input| input.write_all(include_str!("../../install.ps1").as_bytes()));
        if let Err(error) = result {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Cannot supply release lookup: {error}"));
        }
    }
    let mut output = String::new();
    #[cfg(unix)]
    let read = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("Release lookup stdout unavailable"))
        .and_then(|pipe| pipe.take(1024).read_to_string(&mut output));
    let status = child
        .wait()
        .map_err(|error| format!("Cannot wait for release lookup: {error}"))?;
    #[cfg(unix)]
    read.map_err(|error| format!("Cannot read release lookup: {error}"))?;
    if !status.success() {
        return Err("Could not resolve the latest Eevee release; no changes made".into());
    }
    #[cfg(windows)]
    File::open(lookup.0.join("tag"))
        .and_then(|file| file.take(1024).read_to_string(&mut output))
        .map_err(|error| format!("Cannot read release lookup: {error}"))?;
    let tag = output.trim();
    Version::tag(tag)?;
    Ok(tag.to_owned())
}

/// Check or explicitly update the original native installation, never task data.
pub fn update(check_only: bool, no_animation: bool) -> Result<(), String> {
    let installation = current_installation()?;
    if no_animation {
        eprintln!("ev updater: Checking release");
    }
    let (tag, explicit) = release_tag(!check_only && !no_animation)?;
    let target = Version::tag(&tag)?;
    if target == installation.version {
        println!(
            "ev {} is already up to date ({tag}).",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    if target < installation.version {
        if explicit {
            return Err("The selected release is older than this installation. ev update cannot downgrade, including through EV_VERSION; use the verified retained-package rollback review.".into());
        }
        println!(
            "ev {} is newer than the latest published release ({tag}); no changes made.",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }
    println!(
        "ev {} → {tag}{}",
        env!("CARGO_PKG_VERSION"),
        if check_only {
            " available; run ev update to install."
        } else {
            ""
        }
    );
    if check_only {
        return Ok(());
    }
    let review = lifecycle::update_review(installation, tag, explicit)?;
    apply_review(&review, no_animation)
}

/// Remove only the verified owned executable and its exact license metadata.
pub fn uninstall() -> Result<(), String> {
    let installation = current_installation()?;
    run_installer(&installation.directory, None, false, None, None)
}

fn installer_environment(command: &mut Command) {
    // Provider credentials/configuration are not forwarded to installer helpers.
    command.env_clear();
    for name in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "LOCALAPPDATA",
        "APPDATA",
        "TMPDIR",
        "TMP",
        "TEMP",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "TERM",
        "COLORTERM",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "NO_COLOR",
        "CI",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
        "no_proxy",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

#[cfg(unix)]
fn run_installer(
    directory: &Path,
    tag: Option<&str>,
    no_animation: bool,
    expected_owner: Option<&str>,
    package: Option<(&Path, &Path, &str)>,
) -> Result<(), String> {
    // Wait rather than exec: a terminal-loan caller must resume its exact task.
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(include_str!("../../install.sh"))
        .arg("ev-installer");
    if no_animation {
        command.arg("--no-animation");
    }
    if let Some(tag) = tag {
        command.arg("--version").arg(tag);
    } else {
        command.arg("--uninstall");
    }
    command.arg("--install-dir").arg(directory);
    if let Some(owner) = expected_owner {
        command.arg("--expected-owner").arg(owner);
    }
    if let Some((archive, checksums, hash)) = package {
        command
            .arg("--archive")
            .arg(archive)
            .arg("--checksums")
            .arg(checksums);
        command.arg("--expected-archive").arg(hash);
    }
    installer_environment(&mut command);
    let status = command
        .status()
        .map_err(|error| format!("Cannot execute embedded installer: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Embedded installer failed ({status}); review its recovery details above"
        ))
    }
}

#[cfg(windows)]
fn run_installer(
    directory: &Path,
    tag: Option<&str>,
    no_animation: bool,
    expected_owner: Option<&str>,
    package: Option<(&Path, &Path, &str)>,
) -> Result<(), String> {
    use std::io::Write;
    use std::process::Stdio;
    let directory = directory
        .to_str()
        .ok_or("Installation directory is not valid Unicode")?;
    if directory.chars().any(char::is_control) {
        return Err("Installation directory contains control characters".into());
    }
    let system_root = std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
    let powershell =
        PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    inspect(&powershell, false)?;
    let mut operation = tag.map_or_else(
        || "-Uninstall".to_owned(),
        |tag| format!("-Version '{tag}'"),
    );
    if no_animation {
        operation.push_str(" -NoAnimation");
    }
    if let Some(owner) = expected_owner {
        operation.push_str(&format!(" -ExpectedOwner '{owner}'"));
    }
    if let Some((archive, checksums, hash)) = package {
        operation.push_str(&format!(" -ExpectedArchive '{hash}'"));
        for (flag, path) in [("-Archive", archive), ("-Checksums", checksums)] {
            let path = path
                .to_str()
                .ok_or("Retained package path is not valid Unicode")?;
            if path.chars().any(char::is_control) {
                return Err("Retained package path contains control characters".into());
            }
            operation.push_str(&format!(" {flag} '{}'", path.replace('\'', "''")));
        }
    }
    let expression = format!(
        "$ErrorActionPreference='Stop'; try {{ [Console]::InputEncoding=[Text.UTF8Encoding]::new($false); $s=[Console]::In.ReadToEnd(); & ([scriptblock]::Create($s)) {operation} -InstallDir '{}' -OwnerProcessId {} }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}",
        directory.replace('\'', "''"),
        std::process::id()
    );
    let mut command = Command::new(powershell);
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &expression,
        ])
        .stdin(Stdio::piped());
    installer_environment(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("Cannot start embedded installer: {error}"))?;
    let result = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("Installer stdin unavailable"))
        .and_then(|mut input| input.write_all(include_str!("../../install.ps1").as_bytes()));
    if let Err(error) = result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("Cannot supply embedded installer: {error}"));
    }
    let status = child
        .wait()
        .map_err(|error| format!("Cannot wait for installer result: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Embedded installer failed ({status}); review its recovery details above"
        ))
    }
}

#[cfg(not(any(unix, windows)))]
fn run_installer(
    _directory: &Path,
    _tag: Option<&str>,
    _no_animation: bool,
    _expected_owner: Option<&str>,
    _package: Option<(&Path, &Path, &str)>,
) -> Result<(), String> {
    Err("Native installation lifecycle is unsupported on this platform".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_versions_are_numeric_and_canonical() {
        assert!(Version::parse("1.10.0").unwrap() > Version::parse("1.9.99").unwrap());
        for invalid in [
            "",
            "v1.2.3",
            "1.2",
            "1.2.3.4",
            "01.2.3",
            "1.2.3-rc1",
            "1.2.3+meta",
            "1.2.3\n",
            "1.2.18446744073709551616",
            "1.2.-3",
        ] {
            assert!(Version::parse(invalid).is_err(), "{invalid:?}");
        }
        assert!(Version::tag("v0.1.0").is_ok());
        assert!(Version::tag("0.1.0").is_err());
    }

    #[test]
    fn latest_redirect_must_be_the_exact_https_repository_tag() {
        assert_eq!(
            tag_from_location(&format!("{TAG_PREFIX}v0.1.0")).unwrap(),
            "v0.1.0"
        );
        for location in [
            "http://github.com/kimmandoo/eevee/releases/tag/v1.2.3",
            "https://github.com/other/eevee/releases/tag/v1.2.3",
            "https://github.com.evil.test/kimmandoo/eevee/releases/tag/v1.2.3",
            "https://user@github.com/kimmandoo/eevee/releases/tag/v1.2.3",
            "https://github.com/kimmandoo/eevee/releases/tag/v1.2.3?x=y",
            "https://github.com/kimmandoo/eevee/releases/tag/v1.2.3/../evil",
            "https://github.com/kimmandoo/eevee/releases/tag/v1.2.3#fragment",
            "/kimmandoo/eevee/releases/tag/v1.2.3",
        ] {
            assert!(tag_from_location(location).is_err(), "{location}");
        }
    }

    #[test]
    fn ownership_marker_is_exact_and_requires_all_v2_hashes() {
        let hash = "a".repeat(64);
        let marker = format!(
            "ev-native-install-v2\nversion=ev 0.1.0\nsha256={hash}\nlicense-sha256={hash}\nnotices-sha256={hash}\n"
        );
        assert_eq!(
            parse_marker(&marker).unwrap().metadata_hashes,
            (hash.as_str(), hash.as_str())
        );
        assert!(parse_marker(&marker.replace('\n', "\r\n")).is_ok());
        assert!(parse_marker(marker.trim_end()).is_err());
        assert!(parse_marker(&format!("{marker}\n")).is_err());
        assert!(parse_marker(&marker.replace("license-sha256=", "sha256=")).is_err());
        assert!(parse_marker(&marker.replace(&hash, &"A".repeat(64))).is_err());
        assert!(parse_marker(&marker.replace("ev 0.1.0", "ev 0.1.0\rextra")).is_err());
        let legacy = format!("ev-native-install-v1\nversion=ev 0.1.0\nsha256={hash}\n");
        assert!(parse_marker(&legacy).is_err());
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let parent = fs::canonicalize(std::env::temp_dir()).unwrap();
            let path = parent.join(format!("ev-ownership-test-{}-{stamp}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn executable(&self) -> PathBuf {
            self.0.join(if cfg!(windows) { "ev.exe" } else { "ev" })
        }
        fn install(&self) {
            fs::write(self.executable(), b"owned executable bytes").unwrap();
            let metadata = self.0.join(".ev-install");
            fs::create_dir(&metadata).unwrap();
            fs::write(metadata.join("LICENSE"), b"license").unwrap();
            fs::write(metadata.join("THIRD_PARTY_NOTICES"), b"notices").unwrap();
            fs::write(metadata.join(".native-install"), format!("ev-native-install-v2\nversion=ev 0.1.0\nsha256={}\nlicense-sha256={}\nnotices-sha256={}\n", hash_file(&self.executable(), MAX_BINARY).unwrap(), hash_file(&metadata.join("LICENSE"), MAX_METADATA).unwrap(), hash_file(&metadata.join("THIRD_PARTY_NOTICES"), MAX_METADATA).unwrap())).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn ownership_rejects_unowned_copied_tampered_and_extra_files() {
        let fixture = Fixture::new();
        fs::write(fixture.executable(), b"unowned").unwrap();
        assert!(owned_installation(&fixture.executable()).is_err());
        fixture.install();
        assert!(owned_installation(&fixture.executable()).is_ok());
        let copy = fixture.0.join("copy");
        fs::create_dir(&copy).unwrap();
        let copied_executable = copy.join(fixture.executable().file_name().unwrap());
        fs::copy(fixture.executable(), &copied_executable).unwrap();
        assert!(owned_installation(&copied_executable).is_err());
        fs::write(fixture.0.join(".ev-install/extra"), b"keep").unwrap();
        assert!(owned_installation(&fixture.executable()).is_err());
        fs::remove_file(fixture.0.join(".ev-install/extra")).unwrap();
        fs::write(fixture.0.join(".ev-install/LICENSE"), b"changed license").unwrap();
        assert!(owned_installation(&fixture.executable()).is_err());
        fs::write(fixture.0.join(".ev-install/LICENSE"), b"license").unwrap();
        fs::write(fixture.executable(), b"modified binary").unwrap();
        assert!(owned_installation(&fixture.executable()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn ownership_rejects_symbolic_and_multiple_hard_links() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        fixture.install();
        let binary = fixture.executable();
        let moved = fixture.0.join("other");
        fs::rename(&binary, &moved).unwrap();
        symlink(&moved, &binary).unwrap();
        assert!(owned_installation(&binary).is_err());
        fs::remove_file(&binary).unwrap();
        fs::hard_link(&moved, &binary).unwrap();
        assert!(owned_installation(&binary).is_err());
    }
}
