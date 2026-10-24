//! Offline, byte-preserving full-directory snapshots. Never opens the source with SQLite.
use super::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Seek};

const MANIFEST: &str = "snapshot-manifest.json";
pub(super) const INCOMPLETE: &str = ".ev-snapshot-incomplete";
const FORMAT: u32 = 1;
const MAX_MANIFEST: u64 = 16 * 1024 * 1024;
const MAX_MEMBERS: usize = 65536;

pub const DISCLOSURE: &str = "Offline snapshot format 1 copies ALL app-owned data, including private task/draft text, journals, outputs, artifacts, archived tasks, preferences and any secrets or saved app credentials present in the data directory. It is NOT encrypted or redacted. External credential stores are never accessed. Source bytes are not migrated, repaired or changed. Destinations must not exist; failures may leave a marked incomplete directory. Restore validates the backup with this reader before making a fresh directory usable; it preserves project identities and historical unknown outcomes, not active requests, provider sessions or usable approvals. No merge, project relocation, credential import, dispatch or automatic execution.";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub path: String,
    pub directory: bool,
    pub bytes: u64,
    pub sha256: String,
    /// Original filesystem identity: used only to rebind matching retained evidence.
    pub identity: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub application: String,
    pub created_by: String,
    pub creator_reader_identity: String,
    pub origin: String,
    pub platform: String,
    pub created_at_ms: i64,
    pub privacy: String,
    pub members: Vec<Member>,
}

/// Directory handles anchor member access. Windows additionally pins all ancestors.
struct Directory {
    path: PathBuf,
    anchor: SpoolDirectory,
}
impl Directory {
    fn open(path: &Path) -> Result<Self, String> {
        ensure_spool_directory(&fs::symlink_metadata(path).map_err(|e| e.to_string())?)?;
        let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
        let anchor = SpoolDirectory::open(&path)?;
        Ok(Self { path, anchor })
    }
    fn bound(&self) -> Result<(), String> {
        let current = Directory::open(&self.path)?;
        if directory_identity(&self.anchor.root)? != directory_identity(&current.anchor.root)? {
            return Err("Snapshot directory identity changed".into());
        }
        Ok(())
    }
    fn read(&self, name: &str) -> Result<File, String> {
        validate_member_name(name)?;
        self.bound()?;
        #[cfg(unix)]
        let file = self.anchor.open_file(name, libc::O_RDONLY)?;
        #[cfg(windows)]
        let file = read_regular(&self.path.join(name))?;
        file_identity(&file)?; // Also refuses hard links, on Unix and Windows.
        Ok(file)
    }
    fn child(&self, name: &str) -> Result<Self, String> {
        validate_member_name(name)?;
        self.bound()?;
        let child = Self::open(&self.path.join(name))?;
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            let name = std::ffi::CString::new(name).map_err(|e| e.to_string())?;
            // SAFETY: validated component and owned parent; descriptor adopted once.
            let fd = unsafe {
                libc::openat(
                    self.anchor.root.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(io::Error::last_os_error().to_string());
            }
            // SAFETY: openat returned an owned descriptor.
            let anchored = unsafe { File::from_raw_fd(fd) };
            if directory_identity(&anchored)? != directory_identity(&child.anchor.root)? {
                return Err("Snapshot child directory identity changed".into());
            }
        }
        self.bound()?;
        Ok(child)
    }
    fn create_child(&self, name: &str) -> Result<Self, String> {
        validate_member_name(name)?;
        self.bound()?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let name = std::ffi::CString::new(name).map_err(|e| e.to_string())?;
            // SAFETY: single validated component, anchored live parent, private mode.
            if unsafe { libc::mkdirat(self.anchor.root.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                return Err(format!(
                    "Cannot create fresh private destination (must not exist): {}",
                    io::Error::last_os_error()
                ));
            }
        }
        #[cfg(windows)]
        private_directory(&self.path.join(name))?;
        let child = self.child(name)?;
        self.anchor.sync()?;
        Ok(child)
    }
    fn names(&self) -> Result<Vec<String>, String> {
        self.bound()?;
        let mut names = Vec::new();
        for entry in fs::read_dir(&self.path).map_err(|e| e.to_string())? {
            let name = entry
                .map_err(|e| e.to_string())?
                .file_name()
                .into_string()
                .map_err(|_| "Snapshot refuses non-Unicode member names")?;
            validate_member_name(&name)?;
            names.push(name);
        }
        self.bound()?;
        names.sort();
        Ok(names)
    }
}

fn directory_identity(file: &File) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: owned live handle and writable ABI-correct result.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(format!(
            "{}:{}:{}",
            information.dwVolumeSerialNumber, information.nFileIndexHigh, information.nFileIndexLow
        ))
    }
}

#[cfg(windows)]
fn private_directory(path: &Path) -> Result<(), String> {
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        };
        use windows_sys::Win32::Storage::FileSystem::CreateDirectoryW;
        let sddl: Vec<u16> = "D:P(A;OICI;GA;;;OW)"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: NUL-terminated strings and writable descriptor pointer.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error().to_string());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        // SAFETY: path and descriptor remain live for the call.
        let result = unsafe { CreateDirectoryW(path.as_ptr(), &attributes) };
        let error = io::Error::last_os_error();
        // SAFETY: conversion allocated with LocalAlloc.
        unsafe {
            LocalFree(descriptor);
        }
        if result == 0 {
            Err(format!(
                "Cannot create fresh private destination (must not exist): {error}"
            ))
        } else {
            Ok(())
        }
    }
}

fn fresh(path: &Path, forbidden: &Path) -> Result<Directory, String> {
    let path = canonical_parent(path)?;
    if path.starts_with(forbidden) || forbidden.starts_with(&path) {
        return Err("Snapshot destination must be outside its source directory".into());
    }
    let parent = Directory::open(path.parent().ok_or("Destination has no parent")?)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Destination basename must be Unicode")?;
    let directory = parent.create_child(name)?;
    let mut marker = directory.anchor.create_member(INCOMPLETE)?;
    marker
        .write_all(b"Incomplete offline copy; not usable app data or a complete backup.\n")
        .map_err(|e| e.to_string())?;
    marker.sync_all().map_err(|e| e.to_string())?;
    directory.anchor.sync()?;
    parent.bound()?;
    Ok(directory)
}

fn digest(source: &File, mut destination: Option<File>) -> Result<(u64, String), String> {
    let result = digest_to(source, destination.as_mut())?;
    if let Some(output) = destination {
        output
            .sync_all()
            .map_err(|e| format!("Incomplete copy: {e}"))?;
    }
    Ok(result)
}

fn digest_to<W: Write>(
    mut source: &File,
    mut destination: Option<&mut W>,
) -> Result<(u64, String), String> {
    source.rewind().map_err(|e| e.to_string())?;
    let initial = source.metadata().map_err(|e| e.to_string())?;
    let identity = file_identity(source)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = source.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n as u64)
            .ok_or("Snapshot member is too large")?;
        if total > initial.len() {
            return Err("Snapshot member changed while copying".into());
        }
        hash.update(&buffer[..n]);
        if let Some(output) = &mut destination {
            output
                .write_all(&buffer[..n])
                .map_err(|e| format!("Incomplete copy: {e}"))?;
        }
    }
    let final_metadata = source.metadata().map_err(|e| e.to_string())?;
    if total != initial.len()
        || final_metadata.len() != initial.len()
        || final_metadata.modified().ok() != initial.modified().ok()
        || file_identity(source)? != identity
    {
        return Err("Snapshot member changed while copying".into());
    }
    Ok((total, format!("{:x}", hash.finalize())))
}

fn payload(index: usize) -> String {
    format!("member-{index:08}.data")
}

fn inventory(
    directory: &Directory,
    prefix: &str,
    members: &mut Vec<Member>,
    copy: Option<&Directory>,
    owned_lock: Option<&File>,
) -> Result<(), String> {
    if prefix.split('/').count() > 64 {
        return Err("Snapshot directory nesting exceeds 64".into());
    }
    for name in directory.names()? {
        if name == INCOMPLETE {
            return Err("Source is an incomplete restore or snapshot".into());
        }
        if members.len() >= MAX_MEMBERS {
            return Err("Snapshot exceeds 65536 members".into());
        }
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let metadata =
            fs::symlink_metadata(directory.path.join(&name)).map_err(|e| e.to_string())?;
        if metadata.is_dir() {
            ensure_spool_directory(&metadata)?;
            let child = directory.child(&name)?;
            members.push(Member {
                path: path.clone(),
                directory: true,
                bytes: 0,
                sha256: String::new(),
                identity: String::new(),
            });
            inventory(&child, &path, members, copy, None)?;
        } else {
            ensure_regular_metadata(&metadata)?;
            let output = copy
                .map(|copy| copy.anchor.create_member(&payload(members.len())))
                .transpose()?;
            let opened = directory.read(&name)?;
            let identity = file_identity(&opened)?;
            // Windows byte-range ownership locks also prohibit reads through a
            // separately opened handle. Read every lock-file byte through the
            // actual owner, without releasing its lock or modifying the source.
            let source = if prefix.is_empty() && name == LOCK {
                if let Some(lock) = owned_lock {
                    if file_identity(lock)? != identity {
                        return Err("Source writer lock identity changed".into());
                    }
                    lock
                } else {
                    &opened
                }
            } else {
                &opened
            };
            let (bytes, sha256) = digest(source, output)?;
            members.push(Member {
                path,
                directory: false,
                bytes,
                sha256,
                identity,
            });
        }
    }
    directory.bound()
}

struct Owner(File);

impl Drop for Owner {
    fn drop(&mut self) {
        // Match normal store ownership: a concurrent fork must not extend the
        // lock merely because its child briefly inherited this descriptor.
        let _ = self.0.unlock();
    }
}

fn owner(source: &Directory) -> Result<Owner, String> {
    let file = source.read(LOCK).map_err(|e| format!("Offline snapshot requires an existing writer lock; no lock will be created in the source: {e}"))?;
    file.try_lock().map_err(|e| {
        format!("Live owner: stop Eevee and wait for all writers before snapshot: {e}")
    })?;
    Ok(Owner(file))
}

pub fn create(source: &Path, destination: &Path) -> Result<Manifest, String> {
    let source = Directory::open(source)?;
    let lock = owner(&source)?;
    let destination = fresh(destination, &source.path)?;
    let mut members = Vec::new();
    inventory(&source, "", &mut members, Some(&destination), Some(&lock.0))?;
    if !members.iter().any(|m| m.path == DATABASE && !m.directory) {
        return Err("Source contains no owned database; incomplete copy retained".into());
    }
    let mut checked = Vec::new();
    inventory(&source, "", &mut checked, None, Some(&lock.0))?;
    if checked != members || file_identity(&source.read(LOCK)?)? != file_identity(&lock.0)? {
        return Err("Source changed; copy remains incomplete".into());
    }
    let manifest = Manifest {
        format: FORMAT,
        application: "eevee".into(),
        created_by: env!("CARGO_PKG_VERSION").into(),
        creator_reader_identity: env!("EV_LIFECYCLE_FORMAT_SHA256").into(),
        origin: source
            .path
            .to_str()
            .ok_or("Source path must be Unicode")?
            .into(),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        created_at_ms: timestamp()?,
        privacy: DISCLOSURE.into(),
        members,
    };
    let mut file = destination.anchor.create_member(MANIFEST)?;
    serde_json::to_writer_pretty(&mut file, &manifest).map_err(|e| e.to_string())?;
    file.write_all(b"\n")
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_MANIFEST {
        return Err("Snapshot manifest exceeds 16 MiB; copy remains incomplete".into());
    }
    destination.anchor.sync()?;
    finish(&destination)?;
    Ok(manifest)
}

fn finish(directory: &Directory) -> Result<(), String> {
    directory.bound()?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let name = std::ffi::CString::new(INCOMPLETE).map_err(|e| e.to_string())?;
        // SAFETY: remove only the marker in this owned directory, never a path ancestor.
        if unsafe { libc::unlinkat(directory.anchor.root.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(io::Error::last_os_error().to_string());
        }
    }
    #[cfg(windows)]
    fs::remove_file(directory.path.join(INCOMPLETE)).map_err(|e| e.to_string())?;
    directory.anchor.sync()
}

fn member_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.split('/').count() > 64 {
        return Err("Invalid snapshot member path".into());
    }
    for part in path.split('/') {
        validate_member_name(part)?;
    }
    if path.split('/').any(|part| part == INCOMPLETE) {
        return Err("Snapshot contains an incomplete-copy marker".into());
    }
    Ok(())
}

fn verified(directory: &Directory) -> Result<Manifest, String> {
    let names = directory.names()?;
    if names.iter().any(|name| name == INCOMPLETE) {
        return Err("Snapshot is incomplete; it is not a complete backup".into());
    }
    let file = directory.read(MANIFEST)?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_MANIFEST {
        return Err("Snapshot manifest exceeds 16 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err("Snapshot manifest exceeds 16 MiB".into());
    }
    // Parse and fingerprint the same frozen bytes, not separately opened files.
    let manifest_digest = (bytes.len() as u64, format!("{:x}", Sha256::digest(&bytes)));
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid snapshot manifest: {e}"))?;
    drop(bytes);
    if manifest.format != FORMAT || manifest.application != "eevee" {
        return Err("Unknown snapshot manifest format/application; no changes made".into());
    }
    if manifest.members.len() > MAX_MEMBERS {
        return Err("Snapshot has too many members".into());
    }
    let mut paths = BTreeMap::new();
    let mut expected = BTreeSet::from([MANIFEST.to_owned()]);
    for (index, member) in manifest.members.iter().enumerate() {
        member_path(&member.path)?;
        if paths
            .insert(member.path.clone(), member.directory)
            .is_some()
        {
            return Err("Duplicate snapshot member path".into());
        }
        if let Some((parent, _)) = member.path.rsplit_once('/')
            && paths.get(parent) != Some(&true)
        {
            return Err("Snapshot member has no preceding directory parent".into());
        }
        if member.directory {
            if member.bytes != 0 || !member.sha256.is_empty() || !member.identity.is_empty() {
                return Err("Invalid snapshot directory metadata".into());
            }
        } else {
            if member.identity.is_empty()
                || member.identity.len() > 128
                || !member
                    .identity
                    .bytes()
                    .all(|b| b.is_ascii_digit() || b == b':')
            {
                return Err("Invalid original snapshot member identity".into());
            }
            let name = payload(index);
            expected.insert(name.clone());
            let (bytes, sha256) = digest(&directory.read(&name)?, None)?;
            if bytes != member.bytes || sha256 != member.sha256 {
                return Err(format!(
                    "Snapshot member integrity mismatch: {:?}",
                    member.path
                ));
            }
        }
    }
    if paths.get(DATABASE) != Some(&false) || paths.get(LOCK) != Some(&false) {
        return Err("Snapshot lacks the owned database/writer lock".into());
    }
    if names.into_iter().collect::<BTreeSet<_>>() != expected {
        return Err("Snapshot contains missing or unexpected members".into());
    }
    if directory.names()?.into_iter().collect::<BTreeSet<_>>() != expected
        || digest(&directory.read(MANIFEST)?, None)? != manifest_digest
    {
        return Err("Snapshot changed during verification".into());
    }
    directory.bound()?;
    Ok(manifest)
}

/// Integrity inspection reads only ordinary files, never SQLite or credential stores.
pub fn verify(source: &Path) -> Result<Manifest, String> {
    verified(&Directory::open(source)?)
}

fn destination_parent(root: &Directory, path: &str) -> Result<(Directory, String), String> {
    member_path(path)?;
    let mut components = path.split('/').collect::<Vec<_>>();
    let name = components
        .pop()
        .ok_or("Missing member filename")?
        .to_owned();
    let mut directory = Directory::open(&root.path)?;
    for component in components {
        directory = directory.child(component)?;
    }
    Ok((directory, name))
}

fn restore_database(root: &Directory, manifest: &Manifest) -> Result<(), String> {
    // Recover only this fresh, private, marked-incomplete copy. SQLite may consume
    // rollback/WAL journals here; incompatible data never becomes usable, and the
    // original source and complete byte-exact backup are never opened by SQLite.
    let database = root.path.join(DATABASE);
    let mut connection = Connection::open_with_flags(
        &database,
        OpenFlags::SQLITE_OPEN_READ_WRITE | sqlite_no_follow(),
    )
    .map_err(|e| e.to_string())?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|e| e.to_string())?;
    inspect(&connection)?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|e| e.to_string())?;
    initialize(&mut connection)?;
    // OS file identities cannot be portable. Rebind only verified owned retained outputs.
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    for table in ["executions", "tool_artifacts"] {
        let references = {
            let mut statement = transaction
                .prepare(&format!(
                    "SELECT rowid,output_name,output_identity FROM {table}"
                ))
                .map_err(|e| e.to_string())?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| e.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| e.to_string())?
        };
        for (rowid, name, old_identity) in references {
            validate_spool_name(&name)?;
            if manifest
                .members
                .iter()
                .any(|m| !m.directory && m.path == name && m.identity == old_identity)
            {
                let identity = file_identity(&root.read(&name)?)?;
                transaction
                    .execute(
                        &format!("UPDATE {table} SET output_identity=?1 WHERE rowid=?2"),
                        params![identity, rowid],
                    )
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    transaction.commit().map_err(|e| e.to_string())?;
    reconcile_runs(&mut connection, &root.path)?; // Unknown stays unknown; releases all stale active claims.
    inspect(&connection)?;
    // FULL commits above use SQLite's writable VFS handles to durably sync the
    // database/journal, as in normal store recovery. A read-only Windows handle
    // cannot FlushFileBuffers; do not reopen one for a redundant sync.
    connection.close().map_err(|(_, e)| e.to_string())?;
    root.anchor.sync()
}

pub fn restore(source: &Path, destination: &Path) -> Result<Manifest, String> {
    let source = Directory::open(source)?;
    let manifest = verified(&source)?;
    let root = fresh(destination, &source.path)?;
    for (index, member) in manifest.members.iter().enumerate() {
        let (parent, name) = destination_parent(&root, &member.path)?;
        if member.directory {
            parent.create_child(&name)?;
        } else {
            let (bytes, sha256) = digest(
                &source.read(&payload(index))?,
                Some(parent.anchor.create_member(&name)?),
            )?;
            if bytes != member.bytes || sha256 != member.sha256 {
                return Err(
                    "Backup changed while restoring; destination remains incomplete".into(),
                );
            }
        }
    }
    // All compatibility/schema/row admission precedes usability; no normal app worker is started.
    restore_database(&root, &manifest)?;
    if verified(&source)? != manifest {
        return Err(
            "Backup manifest changed while restoring; destination remains incomplete".into(),
        );
    }
    finish(&root)?;
    Ok(manifest)
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
