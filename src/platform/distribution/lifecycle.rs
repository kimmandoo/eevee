//! Immutable lifecycle reviews and one bounded, checksum-owned prior package.
use super::*;
#[cfg(windows)]
use std::io::Write;

const SLOT: &str = ".ev-rollback";
const MANIFEST: &str = "retained-package";
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;

#[cfg(all(test, unix))]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Update,
    Rollback,
}

#[derive(Clone, Debug)]
pub struct LifecycleReview {
    pub operation: Operation,
    pub current_version: String,
    pub target_version: String,
    pub source: String,
    pub installation_directory: PathBuf,
    pub can_apply: bool,
    pub reason: String,
    installation: Installation,
    tag: String,
    apply_allowed: bool,
    rollback: Option<Retained>,
    data_dir: Option<PathBuf>,
}

impl LifecycleReview {
    /// This is the exact owned installation path, not a PATH lookup.
    pub fn executable_path(&self) -> PathBuf {
        executable(&self.installation.directory)
    }
}

fn executable(directory: &Path) -> PathBuf {
    directory.join(if cfg!(windows) { "ev.exe" } else { "ev" })
}

fn tag(version: Version) -> String {
    let [major, minor, patch] = version.0;
    format!("v{major}.{minor}.{patch}")
}

/// Explicit discovery only: no package, installation, task or cache is written.
pub fn prepare_update(no_animation: bool) -> Result<LifecycleReview, String> {
    let installation = current_installation()?;
    let (tag, explicit) = release_tag(!no_animation)?;
    update_review(installation, tag, explicit)
}

pub(super) fn update_review(
    installation: Installation,
    selected: String,
    explicit: bool,
) -> Result<LifecycleReview, String> {
    let target = Version::tag(&selected)?;
    let can_apply = target > installation.version;
    let reason = if target == installation.version {
        "The exact release is already installed."
    } else if !can_apply {
        if explicit {
            "The selected release is older than this installation. Updates cannot downgrade; use the verified retained-package rollback review."
        } else {
            "This installation is newer than the latest published stable release. Updates cannot downgrade; use the verified retained-package rollback review."
        }
    } else {
        "Approval replaces only the verified owned package and retains one prior package."
    };
    Ok(LifecycleReview {
        operation: Operation::Update,
        current_version: tag(installation.version),
        target_version: selected.clone(),
        source: format!("{TAG_PREFIX}{selected}"),
        installation_directory: installation.directory.clone(),
        can_apply,
        apply_allowed: can_apply,
        reason: reason.into(),
        installation,
        tag: selected,
        rollback: None,
        data_dir: None,
    })
}

/// A missing/unknown/incompatible prior package is a refusal, never a downgrade probe.
pub fn prepare_rollback(data_dir: &Path) -> Result<LifecycleReview, String> {
    rollback_review(current_installation()?, data_dir)
}

fn rollback_review(installation: Installation, data_dir: &Path) -> Result<LifecycleReview, String> {
    let retained = inspect_retained(&installation.directory.join(SLOT))?;
    compatible(&retained, data_dir)?;
    let can_apply = retained.installation.version != installation.version;
    Ok(LifecycleReview {
        operation: Operation::Rollback,
        current_version: tag(installation.version),
        target_version: tag(retained.installation.version),
        source: retained.directory.display().to_string(),
        installation_directory: installation.directory.clone(),
        can_apply,
        apply_allowed: can_apply,
        reason: if can_apply { "The retained package has known identical readers and current data passed their read-only inspection." } else { "The retained package is already installed." }.into(),
        tag: tag(retained.installation.version),
        installation,
        rollback: Some(retained),
        data_dir: Some(data_dir.to_path_buf()),
    })
}

/// Caller owns explicit consent, committed idle state and terminal lending.
/// Always returns so the caller can drop workers and restart the reviewed path.
pub fn apply_review(review: &LifecycleReview, no_animation: bool) -> Result<(), String> {
    apply_with(
        review,
        no_animation,
        |directory, selected, owner, package| {
            run_installer(
                directory,
                Some(selected),
                no_animation,
                Some(owner),
                package,
            )
        },
    )
}

fn apply_with(
    review: &LifecycleReview,
    _no_animation: bool,
    install: impl FnOnce(&Path, &str, &str, Option<(&Path, &Path, &str)>) -> Result<(), String>,
) -> Result<(), String> {
    if !review.apply_allowed {
        return Err(review.reason.clone());
    }
    Version::tag(&review.tag)?;
    let actual = owned_installation(&review.executable_path())?;
    if actual != review.installation {
        return Err(
            "Installation changed after review; prepare a new review. No replacement performed."
                .into(),
        );
    }
    if let Some(retained) = &review.rollback {
        let current = inspect_retained(&retained.directory)?;
        if current != *retained {
            return Err("Retained package changed after review; no replacement performed.".into());
        }
        compatible(
            &current,
            review
                .data_dir
                .as_deref()
                .ok_or("Missing rollback data scope")?,
        )?;
        install(
            &actual.directory,
            &review.tag,
            &actual.owner,
            Some((
                &current.directory.join(archive_name()?),
                &current.directory.join("SHA256SUMS"),
                &current.archive_hash,
            )),
        )
    } else {
        // Attest only our own running, hash-owned executable. Never label an
        // arbitrary installed older executable as having this build's readers.
        let running = std::env::current_exe().map_err(|error| error.to_string())?;
        if hash_file(&running, MAX_BINARY)? != hash_file(&review.executable_path(), MAX_BINARY)? {
            return Err(
                "The reviewed package is not the running executable; restart before updating."
                    .into(),
            );
        }
        let stage = stage_retained(&actual)?;
        install(&actual.directory, &review.tag, &actual.owner, None)?;
        if let Err(error) = publish_retained(&stage, &actual.directory) {
            let recovery = stage.0.display().to_string();
            std::mem::forget(stage);
            return Err(format!(
                "The new package was installed, but the prior-package slot could not be published: {error}. Verified prior package retained at {recovery}."
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Retained {
    directory: PathBuf,
    installation: Installation,
    format: String,
    archive_hash: String,
    manifest_hash: String,
    checksum_hash: String,
}

fn archive_name() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("ev-darwin-arm64.tar.gz"),
        ("macos", "x86_64") => Ok("ev-darwin-x86_64.tar.gz"),
        ("linux", "aarch64") => Ok("ev-linux-arm64.tar.gz"),
        ("linux", "x86_64") => Ok("ev-linux-x86_64.tar.gz"),
        ("windows", "x86_64") => Ok("ev-windows-x86_64.zip"),
        _ => Err("No native retained package format for this platform".into()),
    }
}

// Generated from the complete recursive Rust source tree, dependency identities
// and compiled configuration. A version-only release retains the same contract;
// any unknown child-reader change conservatively refuses rollback.
fn format_identity() -> &'static str {
    env!("EV_LIFECYCLE_FORMAT_SHA256")
}

fn compatible(retained: &Retained, data_dir: &Path) -> Result<(), String> {
    if retained.format != format_identity() {
        return Err("Retained executable has an unknown or different strict serialized format identity. SQL schema equality is insufficient; restore a complete compatible data-directory backup instead. No changes made.".into());
    }
    crate::store::inspect_lifecycle_data(data_dir)
        .map_err(|error| format!("Rollback data compatibility refused: {error}. No changes made."))
}

fn inspect_retained(directory: &Path) -> Result<Retained, String> {
    inspect_ancestry(directory).map_err(|error| format!("No safe retained package: {error}"))?;
    let installation = owned_installation(&executable(directory))?;
    let archive = archive_name()?;
    let expected = [
        if cfg!(windows) { "ev.exe" } else { "ev" },
        ".ev-install",
        MANIFEST,
        "SHA256SUMS",
        archive,
    ];
    let mut count = 0;
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        if !expected.iter().any(|name| entry.file_name() == *name) {
            return Err("Retained package contains unknown files; refusing rollback".into());
        }
        count += 1;
    }
    if count != expected.len() {
        return Err("Retained package is incomplete".into());
    }
    let path = directory.join(MANIFEST);
    if inspect(&path, false)?.len() > 4096 {
        return Err("Oversized retained identity".into());
    }
    let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let mut lines = text.lines();
    if lines.next() != Some("ev-retained-package-v1") || !text.ends_with('\n') {
        return Err("Unknown retained package identity".into());
    }
    let format = marker_hash(lines.next(), "format-sha256=")?.to_owned();
    let owner = marker_hash(lines.next(), "owner-sha256=")?;
    let archive_hash = marker_hash(lines.next(), "archive-sha256=")?.to_owned();
    if lines.next().is_some() || owner != installation.owner {
        return Err("Retained executable ownership differs from its format attestation".into());
    }
    if hash_file(&directory.join(archive), MAX_ARCHIVE)? != archive_hash {
        return Err("Retained archive checksum changed; refusing rollback".into());
    }
    let sums = directory.join("SHA256SUMS");
    if inspect(&sums, false)?.len() > 4096
        || fs::read_to_string(&sums).map_err(|error| error.to_string())?
            != format!("{archive_hash}  {archive}\n")
    {
        return Err("Retained checksum manifest changed; refusing rollback".into());
    }
    Ok(Retained {
        directory: directory.into(),
        installation,
        format,
        archive_hash,
        manifest_hash: hash_file(&path, 4096)?,
        checksum_hash: hash_file(&sums, 4096)?,
    })
}

struct PrivateDirectory(PathBuf);
impl PrivateDirectory {
    fn create(parent: &Path) -> Result<Self, String> {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).map_err(|error| error.to_string())?;
        let path = parent.join(format!(".ev-retain-{:032x}", u128::from_le_bytes(random)));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&path)
            .map_err(|error| format!("Cannot stage retained package: {error}"))?;
        Ok(Self(path))
    }
}
impl Drop for PrivateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn stage_retained(installation: &Installation) -> Result<PrivateDirectory, String> {
    let expanded = inspect(&executable(&installation.directory), false)?.len()
        + inspect(&installation.directory.join(".ev-install/LICENSE"), false)?.len()
        + inspect(
            &installation
                .directory
                .join(".ev-install/THIRD_PARTY_NOTICES"),
            false,
        )?
        .len();
    if expanded > 96 * 1024 * 1024 {
        return Err("Prior package exceeds the verified installer's 96 MiB expanded limit; no installation changes made".into());
    }
    match fs::symlink_metadata(installation.directory.join(SLOT)) {
        Ok(_) => {
            inspect_retained(&installation.directory.join(SLOT))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    let stage = PrivateDirectory::create(&installation.directory)?;
    let metadata = stage.0.join(".ev-install");
    fs::create_dir(&metadata).map_err(|error| error.to_string())?;
    fs::copy(executable(&installation.directory), executable(&stage.0))
        .map_err(|error| error.to_string())?;
    for name in [".native-install", "LICENSE", "THIRD_PARTY_NOTICES"] {
        fs::copy(
            installation.directory.join(".ev-install").join(name),
            metadata.join(name),
        )
        .map_err(|error| error.to_string())?;
    }
    let copy = owned_installation(&executable(&stage.0))?;
    if copy.owner != installation.owner
        || owned_installation(&executable(&installation.directory))? != *installation
    {
        return Err(
            "Installation changed while retaining prior package; no replacement performed".into(),
        );
    }
    package(&stage.0)?;
    let archive = archive_name()?;
    let archive_hash = hash_file(&stage.0.join(archive), MAX_ARCHIVE)?;
    fs::write(
        stage.0.join("SHA256SUMS"),
        format!("{archive_hash}  {archive}\n"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(stage.0.join(MANIFEST), format!("ev-retained-package-v1\nformat-sha256={}\nowner-sha256={}\narchive-sha256={archive_hash}\n", format_identity(), installation.owner)).map_err(|error| error.to_string())?;
    inspect_retained(&stage.0)?;
    Ok(stage)
}

#[cfg(test)]
fn retain(installation: &Installation) -> Result<(), String> {
    let stage = stage_retained(installation)?;
    publish_retained(&stage, &installation.directory)
}

fn publish_retained(stage: &PrivateDirectory, directory: &Path) -> Result<(), String> {
    let slot = directory.join(SLOT);
    let previous = match fs::symlink_metadata(&slot) {
        Ok(_) => {
            inspect_retained(&slot)?;
            Some(PrivateDirectory::create(directory)?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    if let Some(previous) = &previous {
        fs::rename(&slot, previous.0.join("package")).map_err(|error| error.to_string())?;
    }
    if let Err(error) = fs::rename(&stage.0, &slot) {
        if let Some(previous) = previous
            && let Err(recovery) = fs::rename(previous.0.join("package"), &slot)
        {
            let preserved = previous.0.display().to_string();
            std::mem::forget(previous);
            return Err(format!(
                "Retaining package failed: {error}; prior slot recovery failed: {recovery}. Previous slot retained at {preserved}"
            ));
        }
        return Err(format!("Cannot publish retained package: {error}"));
    }
    Ok(())
}

#[cfg(unix)]
fn package(directory: &Path) -> Result<(), String> {
    let mut command = Command::new("tar");
    command
        .arg("--format=ustar")
        .arg("-czf")
        .arg(directory.join(archive_name()?))
        .arg("-C")
        .arg(directory)
        .arg("ev")
        .arg("-C")
        .arg(directory.join(".ev-install"))
        .args(["LICENSE", "THIRD_PARTY_NOTICES"]);
    installer_environment(&mut command);
    command.env("COPYFILE_DISABLE", "1");
    let status = command
        .status()
        .map_err(|error| format!("Cannot package prior executable: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Cannot retain prior package ({status}); no installation changes made"
        ))
    }
}

#[cfg(windows)]
fn package(directory: &Path) -> Result<(), String> {
    use std::process::Stdio;
    let root = std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?;
    let powershell = PathBuf::from(root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    inspect(&powershell, false)?;
    let path = directory
        .to_str()
        .ok_or("Package directory is not valid Unicode")?;
    if path.chars().any(char::is_control) {
        return Err("Unsafe retained package directory".into());
    }
    let script = format!(
        "$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.IO.Compression; Add-Type -AssemblyName System.IO.Compression.FileSystem; $d='{}'; $z=[IO.Compression.ZipFile]::Open([IO.Path]::Combine($d,'{}'),[IO.Compression.ZipArchiveMode]::Create); try {{ foreach($n in @('ev.exe','LICENSE','THIRD_PARTY_NOTICES')) {{ $p=if($n -eq 'ev.exe'){{[IO.Path]::Combine($d,$n)}}else{{[IO.Path]::Combine($d,'.ev-install',$n)}}; [void][IO.Compression.ZipFileExtensions]::CreateEntryFromFile($z,$p,$n) }} }} finally {{ $z.Dispose() }}",
        path.replace('\'', "''"),
        archive_name()?
    );
    let mut command = Command::new(powershell);
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "& ([scriptblock]::Create([Console]::In.ReadToEnd()))",
        ])
        .stdin(Stdio::piped());
    installer_environment(&mut command);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    if let Err(error) = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::other("Package stdin unavailable"))
        .and_then(|mut input| input.write_all(script.as_bytes()))
    {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.to_string());
    }
    let status = child.wait().map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Cannot retain prior package ({status})"))
    }
}

#[cfg(not(any(unix, windows)))]
fn package(_directory: &Path) -> Result<(), String> {
    Err("Native retained packages are unsupported on this platform".into())
}
