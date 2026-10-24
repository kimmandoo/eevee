//! POSIX descriptor-relative namespace operations. Advisory locking and repeated
//! identity checks do not exclude arbitrary uncooperative same-user writers.
use std::ffi::{CString, OsStr, OsString};
use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path};
use std::sync::Arc;

use super::{MAX_BYTES, NamePolicy, WriteFailure};

#[derive(Clone, Debug, PartialEq, Eq)]
struct DirectoryId {
    device: u64,
    inode: u64,
    owner: u32,
    group: u32,
    mode: u32,
}
impl DirectoryId {
    fn capture(metadata: &Metadata) -> Result<Self, String> {
        if !metadata.is_dir() {
            return Err("Selected ancestor is not a directory".into());
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
            group: metadata.gid(),
            mode: metadata.mode(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FileStamp {
    device: u64,
    inode: u64,
    owner: u32,
    group: u32,
    mode: u32,
    pub(super) size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
pub(super) fn file_stamp(file: &File) -> Result<FileStamp, String> {
    let stamp = observation_stamp(file)?;
    if stamp.size > MAX_BYTES as u64 {
        return Err(
            "Selected target must be a regular single-link UTF-8 file at most 64 KiB".into(),
        );
    }
    Ok(stamp)
}

pub(super) fn observation_stamp(file: &File) -> Result<FileStamp, String> {
    let metadata = file
        .metadata()
        .map_err(|_| "Cannot inspect selected file handle")?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err("Source must be a regular single-link file".into());
    }
    Ok(FileStamp {
        device: metadata.dev(),
        inode: metadata.ino(),
        owner: metadata.uid(),
        group: metadata.gid(),
        mode: metadata.mode(),
        size: metadata.len(),
        modified: (metadata.mtime(), metadata.mtime_nsec()),
        changed: (metadata.ctime(), metadata.ctime_nsec()),
    })
}

pub(super) fn same_file(left: &FileStamp, right: &FileStamp) -> bool {
    left.device == right.device
        && left.inode == right.inode
        && left.owner == right.owner
        && left.group == right.group
        && left.mode == right.mode
        && left.size == right.size
        && left.modified == right.modified
}

#[derive(Clone, Debug)]
pub(super) struct Directory(Arc<DirectoryHandle>);
#[derive(Debug)]
struct DirectoryHandle {
    file: File,
    identity: DirectoryId,
    parent: Option<Directory>,
    name: OsString,
}

fn name(value: &OsStr) -> Result<CString, String> {
    if value.is_empty() || value.as_bytes().contains(&b'/') || value == "." || value == ".." {
        return Err("Invalid literal write component".into());
    }
    CString::new(value.as_bytes()).map_err(|_| "Invalid literal write component".into())
}

fn named_regular_matches(named: &libc::stat, opened: &Metadata) -> bool {
    // Match MetadataExt::dev's unsigned identity on macOS's signed dev_t.
    #[cfg(target_os = "macos")]
    let device = named.st_dev as u64;
    #[cfg(not(target_os = "macos"))]
    let device = named.st_dev;
    device == opened.dev()
        && named.st_ino == opened.ino()
        && named.st_mode & libc::S_IFMT == libc::S_IFREG
}
fn last_error(context: &str) -> String {
    format!("{context}: {}", io::Error::last_os_error())
}
fn failure(message: String, uncertain: bool) -> WriteFailure {
    WriteFailure { message, uncertain }
}

fn publish_link(directory: &File, source: &CString, destination: &CString) -> io::Result<()> {
    if unsafe {
        libc::linkat(
            directory.as_raw_fd(),
            source.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
            0,
        )
    } == 0
    {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

impl Directory {
    pub(super) fn open_root(root: &Path) -> Result<Self, String> {
        if !root.is_absolute() {
            return Err("Write root must be canonical and absolute".into());
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")
            .map_err(|_| "Cannot safely open filesystem root")?;
        let identity = DirectoryId::capture(
            &file
                .metadata()
                .map_err(|_| "Cannot inspect filesystem root")?,
        )?;
        let mut directory = Self(Arc::new(DirectoryHandle {
            file,
            identity,
            parent: None,
            name: OsString::from("/"),
        }));
        for component in root.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(value) => {
                    directory = directory
                        .child(value)?
                        .ok_or("Reviewed write root is missing")?
                }
                _ => return Err("Write root must be canonical and absolute".into()),
            }
        }
        directory.verify()?;
        Ok(directory)
    }

    #[cfg(target_os = "macos")]
    pub(super) fn relative_path(&self, root: &Self) -> Result<std::path::PathBuf, String> {
        self.verify()?;
        root.verify()?;
        let native_path = |file: &File| -> Result<std::path::PathBuf, String> {
            let mut buffer = [0u8; libc::PATH_MAX as usize];
            // SAFETY: the retained live handle and PATH_MAX destination satisfy F_GETPATH.
            if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) } == -1
            {
                return Err("Native source directory spelling could not be resolved".into());
            }
            let length = buffer
                .iter()
                .position(|byte| *byte == 0)
                .ok_or("Native source directory path exceeded its bound")?;
            Ok(Path::new(OsStr::from_bytes(&buffer[..length])).to_path_buf())
        };
        let relative = native_path(&self.0.file)?
            .strip_prefix(native_path(&root.0.file)?)
            .map_err(|_| "Native source directory escaped the retained project root")?
            .to_path_buf();
        self.verify()?;
        root.verify()?;
        Ok(relative)
    }

    pub(super) fn names(&self, limit: usize) -> Result<Vec<OsString>, String> {
        self.verify()?;
        let descriptor = unsafe {
            libc::openat(
                self.0.file.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(last_error("Cannot enumerate project directory"));
        }
        let stream = unsafe { libc::fdopendir(descriptor) };
        if stream.is_null() {
            unsafe { libc::close(descriptor) };
            return Err(last_error("Cannot enumerate project directory"));
        }
        struct Entries(*mut libc::DIR);
        impl Drop for Entries {
            fn drop(&mut self) {
                unsafe { libc::closedir(self.0) };
            }
        }
        let entries = Entries(stream);
        let mut names = Vec::new();
        loop {
            let mut entry = std::mem::MaybeUninit::<libc::dirent>::zeroed();
            let mut result = std::ptr::null_mut();
            #[allow(deprecated)]
            let error = unsafe { libc::readdir_r(entries.0, entry.as_mut_ptr(), &mut result) };
            if error != 0 {
                return Err("Project directory enumeration failed".into());
            }
            if result.is_null() {
                break;
            }
            let entry = unsafe { entry.assume_init() };
            let bytes = unsafe { std::ffi::CStr::from_ptr(entry.d_name.as_ptr()) }.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            if names.len() == limit {
                return Err(
                    "Project discovery exceeds 10000 entries; choose a narrower directory scope"
                        .into(),
                );
            }
            names.push(OsStr::from_bytes(bytes).to_owned());
        }
        self.verify()?;
        names.sort();
        Ok(names)
    }

    pub(super) fn same_object(&self, other: &Self) -> bool {
        self.0.identity.device == other.0.identity.device
            && self.0.identity.inode == other.0.identity.inode
    }

    pub(super) fn name_policy(&self) -> NamePolicy {
        #[cfg(target_os = "macos")]
        {
            // APFS/HFS may normalize Unicode even on case-sensitive volumes.
            match unsafe { libc::fpathconf(self.0.file.as_raw_fd(), libc::_PC_CASE_SENSITIVE) } {
                0 => NamePolicy::AsciiInsensitive,
                1 => NamePolicy::AsciiSensitive,
                _ => NamePolicy::Unknown,
            }
        }
        #[cfg(target_os = "linux")]
        {
            let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
            if unsafe { libc::fstatfs(self.0.file.as_raw_fd(), info.as_mut_ptr()) } != 0 {
                return NamePolicy::Unknown;
            }
            let kind = unsafe { info.assume_init() }.f_type;
            if kind == libc::EXT4_SUPER_MAGIC || kind == libc::F2FS_SUPER_MAGIC {
                let mut flags: libc::c_long = 0;
                if unsafe {
                    libc::ioctl(self.0.file.as_raw_fd(), libc::FS_IOC_GETFLAGS, &mut flags)
                } != 0
                {
                    return NamePolicy::Unknown;
                }
                // FS_CASEFOLD_FL is inherited by newly created directories.
                return if flags & 0x4000_0000 != 0 {
                    NamePolicy::AsciiInsensitive
                } else {
                    NamePolicy::ByteSensitive
                };
            }
            // tmpfs is byte-sensitive; overlayfs rejects backing directories
            // with custom d_compare/d_hash. XFS (which has an ASCII-CI format),
            // network/FUSE and other unrecognized filesystems remain unknown.
            if matches!(kind, libc::TMPFS_MAGIC | libc::OVERLAYFS_SUPER_MAGIC) {
                NamePolicy::ByteSensitive
            } else {
                NamePolicy::Unknown
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        NamePolicy::Unknown
    }

    pub(super) fn verify(&self) -> Result<(), String> {
        if DirectoryId::capture(
            &self
                .0
                .file
                .metadata()
                .map_err(|_| "Cannot inspect retained ancestor handle")?,
        )? != self.0.identity
        {
            return Err("Retained write ancestor identity or permissions changed".into());
        }
        let reopened = if let Some(parent) = &self.0.parent {
            parent.verify()?;
            parent
                .child(&self.0.name)?
                .ok_or("Reviewed write ancestor disappeared")?
        } else {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open("/")
                .map_err(|_| "Cannot revalidate filesystem root")?;
            let identity = DirectoryId::capture(
                &file
                    .metadata()
                    .map_err(|_| "Cannot inspect filesystem root")?,
            )?;
            Self(Arc::new(DirectoryHandle {
                file,
                identity,
                parent: None,
                name: OsString::from("/"),
            }))
        };
        if reopened.0.identity != self.0.identity {
            return Err("Reviewed write ancestor was replaced".into());
        }
        Ok(())
    }

    pub(super) fn child(&self, value: &OsStr) -> Result<Option<Self>, String> {
        let component = name(value)?;
        let descriptor = unsafe {
            libc::openat(
                self.0.file.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENOENT) {
                return Ok(None);
            }
            return Err(format!(
                "Write ancestor is linked, non-directory or inaccessible: {error}"
            ));
        }
        let file = unsafe { File::from_raw_fd(descriptor) };
        let identity = DirectoryId::capture(
            &file
                .metadata()
                .map_err(|_| "Cannot inspect opened write ancestor")?,
        )?;
        Ok(Some(Self(Arc::new(DirectoryHandle {
            file,
            identity,
            parent: Some(self.clone()),
            name: value.to_os_string(),
        }))))
    }

    pub(super) fn absent(&self, value: &OsStr) -> Result<bool, String> {
        let component = name(value)?;
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                self.0.file.as_raw_fd(),
                component.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
        {
            return Ok(false);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ENOENT) {
            Ok(true)
        } else {
            Err(format!("Cannot establish selected name absence: {error}"))
        }
    }

    pub(super) fn open_file(&self, value: &OsStr, write: bool) -> Result<File, String> {
        let component = name(value)?;
        let descriptor = unsafe {
            libc::openat(
                self.0.file.as_raw_fd(),
                component.as_ptr(),
                (if write { libc::O_RDWR } else { libc::O_RDONLY })
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK,
            )
        };
        if descriptor < 0 {
            return Err(last_error(
                "Selected file is missing, linked or inaccessible",
            ));
        }
        let file = unsafe { File::from_raw_fd(descriptor) };
        if write {
            file_stamp(&file)?;
        } else {
            observation_stamp(&file)?;
        }
        if write && unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Selected file is busy; leaf replacement refused".into());
        }
        Ok(file)
    }

    pub(super) fn mutation_file(&self, value: &OsStr) -> Result<File, String> {
        self.verify()?;
        let file = self.open_file(value, false)?;
        file_stamp(&file)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Selected file is busy; namespace mutation refused".into());
        }
        Ok(file)
    }

    pub(super) fn same_volume(&self, other: &Self) -> bool {
        self.0.identity.device == other.0.identity.device
    }

    fn mutation_name(&self, value: &OsStr, file: &File) -> Result<CString, String> {
        self.verify()?;
        file_stamp(file)?;
        let component = name(value)?;
        let mut named = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                self.0.file.as_raw_fd(),
                component.as_ptr(),
                named.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(last_error("Reviewed mutation source disappeared"));
        }
        let opened = file
            .metadata()
            .map_err(|_| "Cannot inspect retained mutation source")?;
        if !named_regular_matches(&unsafe { named.assume_init() }, &opened) {
            return Err("Reviewed mutation source name changed; no effect".into());
        }
        Ok(component)
    }

    pub(super) fn remove(&self, value: &OsStr, file: &File) -> Result<(), WriteFailure> {
        let component = self.mutation_name(value, file)?;
        // Like all descriptor-relative POSIX namespace operations, this is not
        // an identity-CAS against an uncooperative same-user name substitution.
        if unsafe { libc::unlinkat(self.0.file.as_raw_fd(), component.as_ptr(), 0) } != 0 {
            let error = io::Error::last_os_error();
            return Err(failure(
                format!("Exact regular-file deletion failed: {error}"),
                error.kind() == io::ErrorKind::Interrupted
                    || error.raw_os_error() == Some(libc::EIO),
            ));
        }
        self.0.file.sync_all().map_err(|error| {
            failure(
                format!("Deletion occurred but parent sync failed; no rollback claimed: {error}"),
                true,
            )
        })
    }

    pub(super) fn rename(
        &self,
        value: &OsStr,
        file: &File,
        destination: &Self,
        destination_value: &OsStr,
    ) -> Result<(), WriteFailure> {
        if !self.same_volume(destination) {
            return Err("Cross-volume rename is unsupported; no move occurred"
                .to_owned()
                .into());
        }
        let source = self.mutation_name(value, file)?;
        destination.verify()?;
        let target = name(destination_value)?;
        if !destination.absent(destination_value)? {
            return Err("Rename destination appeared; nothing was overwritten"
                .to_owned()
                .into());
        }
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                self.0.file.as_raw_fd(),
                source.as_ptr(),
                destination.0.file.as_raw_fd(),
                target.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                self.0.file.as_raw_fd(),
                source.as_ptr(),
                destination.0.file.as_raw_fd(),
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        return Err(
            "This platform has no supported anchored no-overwrite rename; no move occurred"
                .to_owned()
                .into(),
        );
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            if result != 0 {
                let error = io::Error::last_os_error();
                return Err(failure(
                    format!("Anchored no-overwrite rename refused: {error}"),
                    error.kind() == io::ErrorKind::Interrupted
                        || error.raw_os_error() == Some(libc::EIO),
                ));
            }
            self.0
                .file
                .sync_all()
                .and_then(|_| destination.0.file.sync_all())
                .map_err(|error| {
                    failure(
                        format!(
                            "Rename occurred but parent sync failed; no rollback claimed: {error}"
                        ),
                        true,
                    )
                })
        }
    }

    pub(super) fn create_directory(&self, value: &OsStr) -> Result<Self, WriteFailure> {
        self.verify()?;
        let component = name(value)?;
        if unsafe { libc::mkdirat(self.0.file.as_raw_fd(), component.as_ptr(), 0o755) } != 0 {
            let error = io::Error::last_os_error();
            return Err(failure(
                format!(
                    "Reviewed absent parent could not be exclusively created; appeared entries are never adopted: {error}"
                ),
                error.kind() == io::ErrorKind::Interrupted,
            ));
        }
        // mkdirat cannot return a directory handle. No-follow open plus parent/
        // child identity revalidation refuses observable substitution; it is not
        // protection against arbitrary uncooperative same-user namespace races.
        let directory = self
            .child(value)
            .and_then(|child| child.ok_or("Created parent disappeared".into()))
            .map_err(|message| {
                failure(
                    format!("Approved parent was created but cannot be bound: {message}"),
                    true,
                )
            })?;
        self.verify()
            .and_then(|_| directory.verify())
            .and_then(|_| {
                self.0
                    .file
                    .sync_all()
                    .map_err(|error| format!("Cannot sync created parent entry: {error}"))
            })
            .map_err(|message| {
                failure(
                    format!("Approved parent creation occurred; inspect selected scope: {message}"),
                    true,
                )
            })?;
        Ok(directory)
    }

    fn remove_stage(&self, component: &CString, expected: &File) -> Result<(), String> {
        let opened = expected
            .metadata()
            .map_err(|_| "Cannot inspect owned staging handle")?;
        let mut named = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                self.0.file.as_raw_fd(),
                component.as_ptr(),
                named.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(last_error(
                "Owned staging name disappeared; cleanup cannot be established",
            ));
        }
        let named = unsafe { named.assume_init() };
        if !named_regular_matches(&named, &opened) {
            return Err("Staging name identity changed; unrelated entry was not removed".into());
        }
        // POSIX has no identity-conditional unlink. Only our unpredictable,
        // exclusively-created staging name is eligible for this cleanup.
        if unsafe { libc::unlinkat(self.0.file.as_raw_fd(), component.as_ptr(), 0) } != 0 {
            return Err(last_error("Cannot remove exact owned staging name"));
        }
        self.0
            .file
            .sync_all()
            .map_err(|error| format!("Cannot sync staging cleanup: {error}"))
    }

    pub(super) fn publish(&self, value: &OsStr, content: &[u8]) -> Result<File, WriteFailure> {
        self.verify()?;
        let destination = name(value)?;
        if !self.absent(value)? {
            return Err("Reviewed absent target appeared; no publication"
                .to_owned()
                .into());
        }
        let mut random = [0_u8; 16];
        getrandom::getrandom(&mut random).map_err(|_| {
            "Cannot choose an unpredictable staging name; target not published".to_owned()
        })?;
        let mut staging = String::with_capacity(49);
        staging.push_str(".eevee-write-");
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in random {
            staging.push(HEX[usize::from(byte >> 4)] as char);
            staging.push(HEX[usize::from(byte & 15)] as char);
        }
        staging.push_str(".tmp");
        let component = name(OsStr::new(&staging))?;
        let descriptor = unsafe {
            libc::openat(
                self.0.file.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if descriptor < 0 {
            return Err(
                last_error("Cannot exclusively create content staging; no publication").into(),
            );
        }
        let mut file = unsafe { File::from_raw_fd(descriptor) };
        let prepared = (|| -> Result<(), String> {
            file.write_all(content)
                .and_then(|_| file.sync_all())
                .map_err(|error| format!("Cannot prepare complete new-file content: {error}"))?;
            file_stamp(&file)?;
            self.verify()?;
            // Bind the source namespace entry to the exclusively created handle
            // before hard-link publication. linkat never replaces any target.
            let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe {
                libc::fstatat(
                    self.0.file.as_raw_fd(),
                    component.as_ptr(),
                    metadata.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(last_error("Cannot revalidate staged content name"));
            }
            let metadata = unsafe { metadata.assume_init() };
            let opened = file
                .metadata()
                .map_err(|_| "Cannot revalidate staged content handle")?;
            if !named_regular_matches(&metadata, &opened) || metadata.st_nlink != 1 {
                return Err("Staged content name identity changed; no publication".into());
            }
            Ok(())
        })();
        if let Err(message) = prepared {
            return match self.remove_stage(&component, &file) {
                Ok(()) => Err(failure(
                    format!("{message}; owned staging removed, target not published"),
                    false,
                )),
                Err(cleanup) => Err(failure(
                    format!("{message}; staging may remain at {staging}: {cleanup}"),
                    true,
                )),
            };
        }
        if let Err(publication_error) = publish_link(&self.0.file, &component, &destination) {
            return match self.remove_stage(&component, &file) {
                Ok(()) => Err(failure(
                    format!(
                        "No-overwrite publication failed; appeared target was not replaced: {publication_error}"
                    ),
                    publication_error.kind() == io::ErrorKind::Interrupted,
                )),
                Err(cleanup) => Err(failure(
                    format!(
                        "Publication failed: {publication_error}; staging may remain at {staging}: {cleanup}"
                    ),
                    true,
                )),
            };
        }
        self.remove_stage(&component, &file)
            .map_err(|message| failure(format!("Complete target was published, but staging cleanup/durability is uncertain ({staging}): {message}"), true))?;
        self.verify().map_err(|message| {
            failure(
                format!(
                    "Complete target was published, but its parent identity changed: {message}"
                ),
                true,
            )
        })?;
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn native_publication_and_mkdir_refuse_entries_that_appeared_after_review() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "eevee-posix-publication-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let directory = Directory::open_root(&root).unwrap();
        std::fs::write(root.join("staged"), "complete reviewed bytes\n").unwrap();
        for (index, kind) in ["regular", "directory", "dangling"].iter().enumerate() {
            let leaf = format!("target-{index}");
            assert!(directory.absent(OsStr::new(&leaf)).unwrap());
            match *kind {
                "regular" => std::fs::write(root.join(&leaf), "unrelated user bytes").unwrap(),
                "directory" => std::fs::create_dir(root.join(&leaf)).unwrap(),
                _ => symlink("outside-missing", root.join(&leaf)).unwrap(),
            }
            let error = publish_link(
                &directory.0.file,
                &CString::new("staged").unwrap(),
                &CString::new(leaf.clone()).unwrap(),
            )
            .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(libc::EEXIST));
            assert!(
                !directory
                    .create_directory(OsStr::new(&leaf))
                    .unwrap_err()
                    .uncertain
            );
            match *kind {
                "regular" => assert_eq!(
                    std::fs::read_to_string(root.join(&leaf)).unwrap(),
                    "unrelated user bytes"
                ),
                "directory" => assert!(root.join(&leaf).is_dir()),
                _ => assert!(
                    std::fs::symlink_metadata(root.join(&leaf))
                        .unwrap()
                        .file_type()
                        .is_symlink()
                ),
            }
        }
        publish_link(
            &directory.0.file,
            &CString::new("staged").unwrap(),
            &CString::new("published").unwrap(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("published")).unwrap(),
            "complete reviewed bytes\n"
        );
        drop(directory);
        std::fs::remove_dir_all(root).unwrap();
    }
}
