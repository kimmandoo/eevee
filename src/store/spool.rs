//! Anchored spool directory for retained run output.
use super::*;

/// A directory handle anchors POSIX operations. On Windows, deny-delete handles
/// retain every ancestor while an operation uses its path; final files are no-follow.
pub(super) struct SpoolDirectory {
    root: File,
    #[cfg(windows)]
    path: PathBuf,
    #[cfg(windows)]
    _ancestors: Vec<File>,
}

impl SpoolDirectory {
    pub(super) fn open(path: &Path) -> Result<Self, String> {
        if fs::canonicalize(path).map_err(|error| error.to_string())? != path {
            return Err("Storage directory identity changed; refusing artifact access".into());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options
                .share_mode(3)
                .custom_flags(0x0200_0000 | 0x0020_0000);
        }
        #[cfg(windows)]
        let ancestors = {
            let mut handles = Vec::new();
            for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
                if ancestor == path || ancestor.as_os_str().is_empty() {
                    continue;
                }
                let file = options.open(ancestor).map_err(|error| error.to_string())?;
                ensure_spool_directory(&file.metadata().map_err(|error| error.to_string())?)?;
                handles.push(file);
            }
            handles
        };
        let root = options.open(path).map_err(|error| error.to_string())?;
        ensure_spool_directory(&root.metadata().map_err(|error| error.to_string())?)?;
        if fs::canonicalize(path).map_err(|error| error.to_string())? != path {
            return Err("Storage directory was replaced while opening".into());
        }
        Ok(Self {
            root,
            #[cfg(windows)]
            path: path.to_path_buf(),
            #[cfg(windows)]
            _ancestors: ancestors,
        })
    }

    pub(super) fn sync(&self) -> Result<(), String> {
        ensure_spool_directory(&self.root.metadata().map_err(|error| error.to_string())?)?;
        #[cfg(unix)]
        self.root.sync_all().map_err(|error| error.to_string())?;
        Ok(())
    }

    #[cfg(unix)]
    pub(super) fn open_file(&self, name: &str, flags: i32) -> Result<File, String> {
        use std::os::fd::{AsRawFd, FromRawFd};
        validate_spool_name(name)?;
        let name = std::ffi::CString::new(name).map_err(|error| error.to_string())?;
        // SAFETY: root is owned and live, the filename is a single validated component,
        // and the returned descriptor is adopted exactly once.
        let fd = unsafe {
            libc::openat(
                self.root.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        // SAFETY: successful openat returned a new owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        ensure_regular_metadata(&file.metadata().map_err(|error| error.to_string())?)?;
        Ok(file)
    }

    pub(super) fn create(&self, name: &str) -> Result<File, String> {
        #[cfg(unix)]
        {
            self.open_file(name, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use std::os::windows::io::FromRawHandle;
            use windows_sys::Win32::Foundation::{INVALID_HANDLE_VALUE, LocalFree};
            use windows_sys::Win32::Security::{
                Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SECURITY_ATTRIBUTES,
            };
            use windows_sys::Win32::Storage::FileSystem::{
                CREATE_NEW, CreateFileW, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
                FILE_GENERIC_WRITE,
            };
            validate_spool_name(name)?;
            let path: Vec<u16> = self
                .path
                .join(name)
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            // A protected owner-rights-only DACL, not permissions inherited from an
            // arbitrary --data-dir. The owner remains the creating user's token.
            let sddl: Vec<u16> = "D:P(A;;GA;;;OW)".encode_utf16().chain(Some(0)).collect();
            let mut descriptor = std::ptr::null_mut();
            // SAFETY: both strings are NUL-terminated and the descriptor out pointer is valid.
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
            // SAFETY: attributes and path remain live until CreateFileW returns.
            let handle = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                    7,
                    &attributes,
                    CREATE_NEW,
                    FILE_FLAG_OPEN_REPARSE_POINT,
                    std::ptr::null_mut(),
                )
            };
            let error = io::Error::last_os_error();
            // SAFETY: the conversion routine allocated descriptor with LocalAlloc.
            unsafe {
                LocalFree(descriptor);
            }
            if handle == INVALID_HANDLE_VALUE {
                return Err(error.to_string());
            }
            // SAFETY: the successful file handle is owned exactly once.
            let file = unsafe { File::from_raw_handle(handle) };
            ensure_regular_metadata(&file.metadata().map_err(|error| error.to_string())?)?;
            Ok(file)
        }
    }

    pub(super) fn read(&self, name: &str, identity: &str) -> Result<File, String> {
        validate_spool_name(name)?;
        #[cfg(unix)]
        let file = self.open_file(name, libc::O_RDONLY)?;
        #[cfg(windows)]
        let file = {
            let mut options = OpenOptions::new();
            options.read(true);
            no_follow(&mut options);
            options
                .open(self.path.join(name))
                .map_err(|error| error.to_string())?
        };
        ensure_regular_metadata(&file.metadata().map_err(|error| error.to_string())?)?;
        if file_identity(&file)? != identity {
            return Err("Output file identity changed; retained evidence is unavailable".into());
        }
        Ok(file)
    }

    pub(super) fn remove(&self, name: &str, identity: &str) -> Result<(), String> {
        validate_spool_name(name)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let name = std::ffi::CString::new(name).map_err(|error| error.to_string())?;
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: root and name are live; stat points to writable initialized-on-success memory.
            if unsafe {
                libc::fstatat(
                    self.root.as_raw_fd(),
                    name.as_ptr(),
                    stat.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                let error = io::Error::last_os_error();
                return if error.kind() == io::ErrorKind::NotFound {
                    Ok(())
                } else {
                    Err(error.to_string())
                };
            }
            // SAFETY: fstatat succeeded.
            let stat = unsafe { stat.assume_init() };
            // Match MetadataExt::dev's unsigned identity on macOS's signed dev_t.
            #[cfg(target_os = "macos")]
            let device = stat.st_dev as u64;
            #[cfg(not(target_os = "macos"))]
            let device = stat.st_dev;
            if stat.st_mode & libc::S_IFMT != libc::S_IFREG
                || format!("{}:{}", device, stat.st_ino) != identity
            {
                // A link/replacement is not this artifact. In particular never traverse
                // it to remove an external target or destroy another task's reference.
                return Ok(());
            }
            // SAFETY: unlinkat removes only this anchored directory entry, never a
            // link target even if another process swaps it after the metadata check.
            if unsafe { libc::unlinkat(self.root.as_raw_fd(), name.as_ptr(), 0) } != 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::NotFound {
                    return Err(error.to_string());
                }
            }
            self.sync()
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                FILE_DISPOSITION_INFO, FileDispositionInfo, SetFileInformationByHandle,
            };
            let mut options = OpenOptions::new();
            options.access_mode(0x0001_0000 | 0x80).share_mode(7);
            no_follow(&mut options);
            let file = match options.open(self.path.join(name)) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.to_string()),
            };
            if ensure_regular_metadata(&file.metadata().map_err(|error| error.to_string())?)
                .is_err()
                || file_identity(&file)? != identity
            {
                return Ok(());
            }
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            // SAFETY: this exact no-follow handle was identity-checked; disposition is
            // live and has the native ABI size. Deletion follows this handle, not a path.
            if unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle(),
                    FileDispositionInfo,
                    (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                    std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            } == 0
            {
                return Err(io::Error::last_os_error().to_string());
            }
            Ok(())
        }
    }
}

pub(super) fn validate_spool_name(name: &str) -> Result<(), String> {
    let id = name
        .strip_prefix("ev-run-")
        .and_then(|name| name.strip_suffix(".output"))
        .ok_or("Invalid owned output filename")?;
    if valid_run_id(id) {
        Ok(())
    } else {
        Err("Invalid owned output filename".into())
    }
}

pub(super) fn ensure_spool_directory(metadata: &fs::Metadata) -> Result<(), String> {
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Artifact directory is not an unlinked directory".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Artifact directory is a Windows reparse point".into());
        }
    }
    Ok(())
}

pub(super) fn file_identity(file: &File) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        if metadata.nlink() != 1 {
            return Err("Output must have exactly one owned filesystem link".into());
        }
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut information = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: file owns a live handle; information is writable and ABI-correct.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        if information.nNumberOfLinks != 1 {
            return Err("Output must have exactly one owned filesystem link".into());
        }
        Ok(format!(
            "{}:{}:{}",
            information.dwVolumeSerialNumber, information.nFileIndexHigh, information.nFileIndexLow
        ))
    }
}
