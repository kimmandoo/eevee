//! Bounded, no-follow, owner-private native credential files with atomic replacement.
use super::*;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) type FileIdentity = (u64, u64);

fn file_identity(file: &File) -> Result<FileIdentity, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata().map_err(|_| ERROR.to_string())?;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        super::super::windows::file_identity(file)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        Err(ERROR.into())
    }
}

#[cfg(unix)]
struct MutationLock<'a>(&'a File);
#[cfg(unix)]
impl Drop for MutationLock<'_> {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

pub(super) struct Directory {
    root: File,
    #[cfg(windows)]
    path: PathBuf,
    #[cfg(windows)]
    _ancestors: Vec<File>,
}

fn safe_file(file: &File, bounded: bool) -> Result<(), String> {
    let metadata = file.metadata().map_err(|_| ERROR.to_string())?;
    if !metadata.is_file() || bounded && metadata.len() > FILE_LIMIT {
        return Err(ERROR.into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err(ERROR.into());
        }
        super::super::posix::check_acl(file, true)?;
    }
    #[cfg(windows)]
    {
        super::super::windows::check_private_file(file)?;
    }
    Ok(())
}

impl Directory {
    #[cfg(unix)]
    fn mutation_lock(&self) -> Result<MutationLock<'_>, String> {
        use std::os::fd::AsRawFd;
        self.check_root()?;
        if unsafe { libc::flock(self.root.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(
                "Private connection storage is busy; no changes made. Try again deliberately."
                    .into(),
            );
        }
        Ok(MutationLock(&self.root))
    }

    fn check_root(&self) -> Result<(), String> {
        let metadata = self.root.metadata().map_err(|_| ERROR.to_string())?;
        if !metadata.is_dir() {
            return Err(ERROR.into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
                return Err(ERROR.into());
            }
            super::super::posix::check_acl(&self.root, true)?;
        }
        #[cfg(windows)]
        {
            super::super::windows::check_private_directory(&self.root)?;
        }
        Ok(())
    }

    pub(super) fn open(home: &Path, project: Option<&Path>, create: bool) -> Result<Self, String> {
        if !home.is_absolute() {
            return Err(ERROR.into());
        }
        let canonical = home.canonicalize().map_err(|_| ERROR.to_string())?;
        let destination = canonical.join(".eevee-connections");
        if project.is_some_and(|root| destination.starts_with(root)) {
            return Err(ERROR.into());
        }
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            use std::os::unix::fs::MetadataExt;
            let current = super::super::posix::open_directory(&canonical)?;
            let metadata = current.metadata().map_err(|_| ERROR.to_string())?;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
                return Err(ERROR.into());
            }
            super::super::posix::check_acl(&current, false)?;
            let name = c".eevee-connections";
            let created = if create {
                if unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o700) } == 0 {
                    true
                } else if std::io::Error::last_os_error().kind()
                    == std::io::ErrorKind::AlreadyExists
                {
                    false
                } else {
                    return Err(ERROR.into());
                }
            } else {
                false
            };
            let descriptor = unsafe {
                libc::openat(
                    current.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if descriptor < 0 {
                return Err(ERROR.into());
            }
            let root = unsafe { File::from_raw_fd(descriptor) };
            if created {
                super::super::posix::clear_new_acl(&root)?;
            }
            let metadata = root.metadata().map_err(|_| ERROR.to_string())?;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
                return Err(ERROR.into());
            }
            super::super::posix::check_acl(&root, true)?;
            Ok(Self { root })
        }
        #[cfg(windows)]
        {
            let (root, ancestors) = super::super::windows::private_directory(&canonical, create)?;
            Ok(Self {
                root,
                path: destination,
                _ancestors: ancestors,
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = create;
            Err(
                "Private credential persistence is unsupported on this platform; use session-only."
                    .into(),
            )
        }
    }

    pub(super) fn read(&self, name: &str) -> Result<Option<File>, String> {
        self.open_leaf(name, true)
    }

    fn open_leaf(&self, name: &str, bounded: bool) -> Result<Option<File>, String> {
        self.check_root()?;
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            let name = std::ffi::CString::new(name).map_err(|_| ERROR.to_string())?;
            let descriptor = unsafe {
                libc::openat(
                    self.root.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            if descriptor < 0 {
                return if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                    Ok(None)
                } else {
                    Err(ERROR.into())
                };
            }
            let file = unsafe { File::from_raw_fd(descriptor) };
            safe_file(&file, bounded)?;
            Ok(Some(file))
        }
        #[cfg(windows)]
        {
            let path = self.path.join(name);
            if fs::symlink_metadata(&path)
                .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound)
            {
                return Ok(None);
            }
            let canonical = path.canonicalize().map_err(|_| ERROR.to_string())?;
            let file = super::super::windows::open_user_file(&path, &canonical, true)?;
            safe_file(&file, bounded)?;
            Ok(Some(file))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = name;
            Err(ERROR.into())
        }
    }

    pub(super) fn identity(&self, name: &str) -> Result<Option<FileIdentity>, String> {
        self.open_leaf(name, false)?
            .as_ref()
            .map(file_identity)
            .transpose()
    }

    /// Remove only the reviewed own leaf. No credential bytes or provider paths
    /// are read. An identity change refuses; cooperating writers use this lock.
    pub(super) fn remove(&self, name: &str, expected: FileIdentity) -> Result<bool, String> {
        #[cfg(unix)]
        let _lock = self.mutation_lock()?;
        let file = self
            .open_leaf(name, false)?
            .ok_or("Private connection changed; review deletion again.")?;
        if file_identity(&file)? != expected {
            return Err("Private connection changed; review deletion again.".into());
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let name = std::ffi::CString::new(name).map_err(|_| ERROR.to_string())?;
            if unsafe { libc::unlinkat(self.root.as_raw_fd(), name.as_ptr(), 0) } != 0 {
                return Err(ERROR.into());
            }
            Ok(self.root.sync_all().is_ok())
        }
        #[cfg(windows)]
        {
            drop(file);
            super::super::windows::remove_private_file(&self.path.join(name), expected)?;
            Ok(true)
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(ERROR.into())
        }
    }

    /// True means replacement and durability confirmation both completed.
    pub(super) fn write(&self, name: &str, bytes: &[u8]) -> Result<bool, String> {
        if bytes.len() as u64 > FILE_LIMIT {
            return Err(ERROR.into());
        }
        #[cfg(unix)]
        let _lock = self.mutation_lock()?;
        // Do not silently overwrite a foreign, linked or unsafe existing destination.
        drop(self.read(name)?);
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).map_err(|_| ERROR.to_string())?;
        let random = u128::from_le_bytes(random);
        let temporary = format!(
            ".pending-{}-{}-{random:x}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        #[cfg(unix)]
        {
            use std::os::fd::{AsRawFd, FromRawFd};
            let temporary = std::ffi::CString::new(temporary).map_err(|_| ERROR.to_string())?;
            let name = std::ffi::CString::new(name).map_err(|_| ERROR.to_string())?;
            let descriptor = unsafe {
                libc::openat(
                    self.root.as_raw_fd(),
                    temporary.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if descriptor < 0 {
                return Err(ERROR.into());
            }
            let mut file = unsafe { File::from_raw_fd(descriptor) };
            let result = (|| {
                super::super::posix::clear_new_acl(&file)?;
                safe_file(&file, true)?;
                file.write_all(bytes).map_err(|_| ERROR.to_string())?;
                file.sync_all().map_err(|_| ERROR.to_string())?;
                if unsafe {
                    libc::renameat(
                        self.root.as_raw_fd(),
                        temporary.as_ptr(),
                        self.root.as_raw_fd(),
                        name.as_ptr(),
                    )
                } != 0
                {
                    return Err(ERROR.into());
                }
                Ok(self.root.sync_all().is_ok())
            })();
            if result.is_err()
                && unsafe { libc::unlinkat(self.root.as_raw_fd(), temporary.as_ptr(), 0) } != 0
                && std::io::Error::last_os_error().kind() != std::io::ErrorKind::NotFound
            {
                return Err("Private save failed and a private pending plaintext file could not be removed. Inspect ~/.eevee-connections; no request was sent.".into());
            }
            result
        }
        #[cfg(windows)]
        {
            super::super::windows::write_private_file(&self.path, &temporary, name, bytes)?;
            Ok(true)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = temporary;
            Err(ERROR.into())
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    struct Temporary(PathBuf);
    impl Temporary {
        fn new() -> Self {
            let path = env::temp_dir().join(format!(
                "eevee-private-config-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path.canonicalize().unwrap())
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn private_atomic_storage_is_bounded_and_rejects_links_and_project_paths() {
        let home = Temporary::new();
        assert!(Directory::open(&home.0, Some(&home.0), true).is_err());
        let directory = Directory::open(&home.0, None, true).unwrap();
        directory.write("openai.json", b"first").unwrap();
        directory.write("openai.json", b"second").unwrap();
        let mut file = directory.read("openai.json").unwrap().unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"second");
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert!(
            directory
                .write("openai.json", &vec![0; FILE_LIMIT as usize + 1])
                .is_err()
        );
        symlink(
            home.0.join(".eevee-connections/openai.json"),
            home.0.join(".eevee-connections/google.json"),
        )
        .unwrap();
        assert!(directory.read("google.json").is_err());
        assert!(directory.write("google.json", b"no").is_err());
        fs::set_permissions(
            home.0.join(".eevee-connections/openai.json"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(directory.read("openai.json").is_err());
    }
    #[test]
    fn linked_private_directory_is_refused() {
        let home = Temporary::new();
        let other = Temporary::new();
        symlink(&other.0, home.0.join(".eevee-connections")).unwrap();
        assert!(Directory::open(&home.0, None, true).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn inherited_home_acl_is_removed_only_from_new_owned_objects() {
        use crate::providers::direct::config::posix;
        let home = Temporary::new();
        let home_handle = posix::open_directory(&home.0).unwrap();
        posix::grant_acl_for_test(&home_handle, (1 << 1) | (1 << 3), true);
        assert!(posix::check_acl(&home_handle, false).is_ok());
        assert!(posix::check_acl(&home_handle, true).is_err());
        let directory = Directory::open(&home.0, None, true).unwrap();
        assert!(posix::check_acl(&directory.root, true).is_ok());
        directory
            .write("openai.json", b"isolated-fake-key")
            .unwrap();
        let file = directory.read("openai.json").unwrap().unwrap();
        assert!(posix::check_acl(&file, true).is_ok());
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(
            directory.root.metadata().unwrap().permissions().mode() & 0o777,
            0o700
        );
        // The inherited ACL on the parent belongs to the user, never to this writer.
        assert!(posix::check_acl(&home_handle, true).is_err());
        assert!(posix::check_acl(&home_handle, false).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn existing_store_directory_and_file_acl_grants_are_never_repaired() {
        use crate::providers::direct::config::posix;
        let home = Temporary::new();
        let directory = Directory::open(&home.0, None, true).unwrap();
        directory
            .write("openai.json", b"isolated-fake-key")
            .unwrap();
        let file = directory.read("openai.json").unwrap().unwrap();
        posix::grant_acl_for_test(&file, 1 << 1, false);
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert!(directory.read("openai.json").is_err());
        assert!(directory.write("openai.json", b"replacement").is_err());
        assert!(posix::check_acl(&file, true).is_err());
        posix::grant_acl_for_test(&directory.root, (1 << 1) | (1 << 3), true);
        assert_eq!(
            directory.root.metadata().unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(Directory::open(&home.0, None, false).is_err());
        assert!(directory.write("google.json", b"replacement").is_err());
        assert!(posix::check_acl(&directory.root, true).is_err());
    }
}
