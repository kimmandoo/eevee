//! Windows writes retain every non-reparse ancestor without delete sharing.
//! Child lookup/creation and publication are relative to retained native handles;
//! a pathname is never used to re-traverse an unpinned ancestor during a write.
use super::{MAX_BYTES, NamePolicy, WriteFailure};
use std::ffi::{OsStr, OsString, c_void};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_CASE_SENSITIVE_INFO,
    FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_FULL_DIR_INFO, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_ID_INFO, FILE_LIST_DIRECTORY,
    FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, FILE_TYPE_DISK, FileBasicInfo,
    FileCaseSensitiveInfo, FileDispositionInfo, FileFullDirectoryInfo,
    FileFullDirectoryRestartInfo, FileIdInfo, GetFileInformationByHandle,
    GetFileInformationByHandleEx, GetFileType, GetFinalPathNameByHandleW, SYNCHRONIZE,
    SetFileInformationByHandle, VOLUME_NAME_DOS,
};

// These NT entry points provide the directory-relative exclusive create that
// CreateDirectoryW cannot provide. Keep their small ABI surface here rather
// than enabling unrelated WDK dependencies or substituting a path-based mkdir.
#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root_directory: HANDLE,
    object_name: *mut UnicodeString,
    attributes: u32,
    security_descriptor: *mut c_void,
    security_quality_of_service: *mut c_void,
}

#[repr(C)]
struct IoStatusBlock {
    // The native union is NTSTATUS or a pointer, hence pointer-sized/aligned.
    status_or_pointer: usize,
    information: usize,
}

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtCreateFile(
        handle: *mut HANDLE,
        desired_access: u32,
        object_attributes: *mut ObjectAttributes,
        io_status: *mut IoStatusBlock,
        allocation_size: *const i64,
        file_attributes: u32,
        share_access: u32,
        create_disposition: u32,
        create_options: u32,
        ea_buffer: *const c_void,
        ea_length: u32,
    ) -> i32;
    fn NtSetInformationFile(
        handle: HANDLE,
        io_status: *mut IoStatusBlock,
        information: *const c_void,
        length: u32,
        information_class: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}

const FILE_OPEN: u32 = 1;
const FILE_CREATE: u32 = 2;
const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const OBJ_CASE_INSENSITIVE: u32 = 0x40;
const STATUS_OBJECT_NAME_NOT_FOUND: i32 = 0xc000_0034u32 as i32;
const STATUS_OBJECT_NAME_COLLISION: i32 = 0xc000_0035u32 as i32;
const FILE_RENAME_INFORMATION: u32 = 10;
const DIRECTORY_ACCESS: u32 = FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | FILE_TRAVERSE;
static NEXT_STAGE: AtomicU64 = AtomicU64::new(0);

fn creation_failure(status: i32, path: &Path) -> WriteFailure {
    // Namespace/permission refusals establish no mutation. I/O, disconnect and
    // other failures do not prove that an exclusive create left no entry.
    let refused = matches!(
        status as u32,
        0xc000_0022 | // STATUS_ACCESS_DENIED
        0xc000_0033 | // STATUS_OBJECT_NAME_INVALID
        0xc000_0035 | // STATUS_OBJECT_NAME_COLLISION
        0xc000_003a | // STATUS_OBJECT_PATH_NOT_FOUND
        0xc000_0043 | // STATUS_SHARING_VIOLATION
        0xc000_00a2 | // STATUS_MEDIA_WRITE_PROTECTED
        0xc000_00ba | // STATUS_FILE_IS_A_DIRECTORY
        0xc000_0103 // STATUS_NOT_A_DIRECTORY
    );
    WriteFailure {
        message: format!(
            "Exclusive Windows creation of {} failed{}: {}",
            path.display(),
            if refused {
                " without adopting an existing entry"
            } else {
                "; an entry may remain and must be inspected"
            },
            nt_error(status)
        ),
        uncertain: !refused,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    volume: u64,
    file: [u8; 16],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FileStamp {
    identity: Identity,
    size: u64,
    created: i64,
    written: i64,
    changed: i64,
    attributes: u32,
    links: u32,
}

#[derive(Debug)]
struct DirectoryNode {
    file: File,
    path: PathBuf,
    identity: Identity,
    parent: Option<Arc<DirectoryNode>>,
}

#[derive(Clone, Debug)]
pub(super) struct Directory {
    node: Arc<DirectoryNode>,
}

// One Windows component, never an NT/DOS path, ADS, or DOS device alias.
struct Name {
    units: [u16; 255],
    length: usize,
}

impl Name {
    fn new(name: &OsStr) -> Result<Self, String> {
        let mut result = Self {
            units: [0; 255],
            length: 0,
        };
        for unit in name.encode_wide() {
            if result.length == result.units.len()
                || unit < 32
                || matches!(unit, 34 | 42 | 47 | 58 | 60 | 62 | 63 | 92 | 124)
            {
                return Err("Unsafe Windows write-path component.".into());
            }
            result.units[result.length] = unit;
            result.length += 1;
        }
        let units = result.as_slice();
        if units.is_empty() || matches!(units.last(), Some(32 | 46)) {
            return Err("Empty, dot, or space-suffixed Windows write-path component.".into());
        }
        let stem = &units[..units
            .iter()
            .position(|unit| *unit == 46)
            .unwrap_or(units.len())];
        let equal_ascii = |value: &[u8]| {
            stem.len() == value.len()
                && stem
                    .iter()
                    .zip(value)
                    .all(|(a, b)| *a <= 127 && (*a as u8).eq_ignore_ascii_case(b))
        };
        let numbered_device = stem.len() == 4
            && (stem[..3]
                .iter()
                .zip(b"COM")
                .all(|(a, b)| *a <= 127 && (*a as u8).eq_ignore_ascii_case(b))
                || stem[..3]
                    .iter()
                    .zip(b"LPT")
                    .all(|(a, b)| *a <= 127 && (*a as u8).eq_ignore_ascii_case(b)))
            && matches!(stem[3], 49..=57 | 0xb9 | 0xb2 | 0xb3);
        if equal_ascii(b"CON")
            || equal_ascii(b"PRN")
            || equal_ascii(b"AUX")
            || equal_ascii(b"NUL")
            || equal_ascii(b"CONIN$")
            || equal_ascii(b"CONOUT$")
            || numbered_device
        {
            return Err("Windows device names are not writable project files.".into());
        }
        Ok(result)
    }

    fn as_slice(&self) -> &[u16] {
        &self.units[..self.length]
    }
}

fn nt_error(status: i32) -> String {
    // Conversion preserves the actual native failure (including sharing errors).
    io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32).to_string()
}

fn information(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION, String> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK
        || unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0
    {
        return Err(format!(
            "Cannot inspect disk-file handle: {}",
            io::Error::last_os_error()
        ));
    }
    if info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DEVICE) != 0 {
        return Err("Reparse points and device files are not writable project paths.".into());
    }
    Ok(info)
}

fn identity(file: &File) -> Result<Identity, String> {
    let mut info = FILE_ID_INFO::default();
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == 0
    {
        return Err(format!(
            "Cannot obtain native Windows file ID: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(Identity {
        volume: info.VolumeSerialNumber,
        file: info.FileId.Identifier,
    })
}

fn final_path(file: &File) -> Result<PathBuf, String> {
    let flags = FILE_NAME_NORMALIZED | VOLUME_NAME_DOS;
    let needed = unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), null_mut(), 0, flags) };
    if needed == 0 || needed > 32768 {
        return Err(format!(
            "Cannot resolve Windows handle path: {}",
            io::Error::last_os_error()
        ));
    }
    let mut buffer = vec![0; needed as usize];
    let length = unsafe {
        GetFinalPathNameByHandleW(file.as_raw_handle(), buffer.as_mut_ptr(), needed, flags)
    };
    if length == 0 || length >= needed {
        return Err("Windows handle path changed or could not be resolved.".into());
    }
    buffer.truncate(length as usize);
    Ok(PathBuf::from(OsString::from_wide(&buffer)))
}

fn root_handle(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .access_mode(DIRECTORY_ACCESS | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| format!("Cannot retain Windows volume-root handle: {error}"))
}

fn directory_name_policy(file: &File) -> NamePolicy {
    let mut info = FILE_CASE_SENSITIVE_INFO::default();
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileCaseSensitiveInfo,
            (&mut info as *mut FILE_CASE_SENSITIVE_INFO).cast(),
            size_of::<FILE_CASE_SENSITIVE_INFO>() as u32,
        )
    } == 0
    {
        // Older Windows and filesystems without this query are not assumed to
        // have NTFS's collation or directory-case-sensitivity behavior.
        return NamePolicy::Unknown;
    }
    if info.Flags & 1 != 0 {
        NamePolicy::AsciiSensitive
    } else {
        NamePolicy::AsciiInsensitive
    }
}

// Negative NTSTATUS is returned intact so absence is never inferred from a
// permission/sharing error, a missing ancestor, or an unsupported filesystem.
fn relative_open(
    parent: &File,
    name: &Name,
    access: u32,
    share: u32,
    disposition: u32,
    options: u32,
) -> Result<File, i32> {
    let mut text = UnicodeString {
        length: (name.length * 2) as u16,
        maximum_length: (name.length * 2) as u16,
        buffer: name.units.as_ptr().cast_mut(),
    };
    let mut attributes = ObjectAttributes {
        length: size_of::<ObjectAttributes>() as u32,
        root_directory: parent.as_raw_handle(),
        object_name: &mut text,
        attributes: if directory_name_policy(parent) == NamePolicy::AsciiSensitive {
            0
        } else {
            OBJ_CASE_INSENSITIVE
        },
        security_descriptor: null_mut(),
        security_quality_of_service: null_mut(),
    };
    let mut io_status = IoStatusBlock {
        status_or_pointer: 0,
        information: 0,
    };
    let mut handle = null_mut();
    // All pointers remain live during this synchronous call. A successful
    // handle transfers immediately to File, including every validation failure.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            access | SYNCHRONIZE,
            &mut attributes,
            &mut io_status,
            null(),
            FILE_ATTRIBUTE_NORMAL,
            share,
            disposition,
            options | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_REPARSE_POINT,
            null(),
            0,
        )
    };
    if status < 0 {
        return Err(status);
    }
    Ok(unsafe { File::from_raw_handle(handle) })
}

impl Directory {
    fn retain(
        file: File,
        path: PathBuf,
        parent: Option<Arc<DirectoryNode>>,
    ) -> Result<Self, String> {
        if information(&file)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err("A reviewed Windows directory is no longer a directory.".into());
        }
        if final_path(&file)? != path {
            return Err("Windows directory resolved to a different final path.".into());
        }
        let identity = identity(&file)?;
        Ok(Self {
            node: Arc::new(DirectoryNode {
                file,
                path,
                identity,
                parent,
            }),
        })
    }

    pub(super) fn open_root(root: &Path) -> Result<Self, String> {
        if !root.is_absolute() {
            return Err("Windows write root must be canonical and absolute.".into());
        }
        let mut components = root.components();
        match components.next() {
            Some(Component::Prefix(prefix))
                if matches!(
                    prefix.kind(),
                    Prefix::VerbatimDisk(_) | Prefix::VerbatimUNC(_, _)
                ) => {}
            _ => return Err("Windows write root must use a canonical disk or UNC path.".into()),
        }
        if components.next() != Some(Component::RootDir) {
            return Err("Windows write root has no volume root.".into());
        }
        let mut path = root
            .ancestors()
            .last()
            .ok_or("Missing Windows volume root.")?
            .to_path_buf();
        let mut directory = Self::retain(root_handle(&path)?, path.clone(), None)?;
        for component in components {
            let Component::Normal(name) = component else {
                return Err("Windows write root contains a non-normal component.".into());
            };
            let native = Name::new(name)?;
            let file = relative_open(
                &directory.node.file,
                &native,
                DIRECTORY_ACCESS,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                FILE_OPEN,
                FILE_DIRECTORY_FILE,
            )
            .map_err(nt_error)?;
            path.push(name);
            directory = Self::retain(file, path.clone(), Some(directory.node))?;
        }
        if directory.node.path != root {
            return Err("Windows write root is not canonical.".into());
        }
        Ok(directory)
    }

    pub(super) fn names(&self, limit: usize) -> Result<Vec<OsString>, String> {
        self.verify()?;
        // Native handle enumeration never re-traverses a substituted pathname.
        let mut buffer = [0u64; 8192];
        let mut restart = true;
        let mut names = Vec::new();
        loop {
            let class = if restart {
                FileFullDirectoryRestartInfo
            } else {
                FileFullDirectoryInfo
            };
            let ok = unsafe {
                GetFileInformationByHandleEx(
                    self.node.file.as_raw_handle(),
                    class,
                    buffer.as_mut_ptr().cast(),
                    std::mem::size_of_val(&buffer) as u32,
                )
            };
            if ok == 0 {
                if io::Error::last_os_error().raw_os_error() == Some(18) {
                    break;
                }
                return Err("Project directory enumeration failed".into());
            }
            restart = false;
            let mut offset = 0usize;
            loop {
                let base = buffer.as_ptr().cast::<u8>();
                let info = unsafe { &*base.add(offset).cast::<FILE_FULL_DIR_INFO>() };
                let start = offset + offset_of!(FILE_FULL_DIR_INFO, FileName);
                let bytes = info.FileNameLength as usize;
                if bytes % 2 != 0 || start + bytes > std::mem::size_of_val(&buffer) {
                    return Err("Invalid native project directory entry".into());
                }
                let wide =
                    unsafe { std::slice::from_raw_parts(base.add(start).cast::<u16>(), bytes / 2) };
                let name = OsString::from_wide(wide);
                if name != "." && name != ".." {
                    if names.len() == limit {
                        return Err("Project discovery exceeds 10000 entries; choose a narrower project root".into());
                    }
                    names.push(name);
                }
                if info.NextEntryOffset == 0 {
                    break;
                }
                let next = offset + info.NextEntryOffset as usize;
                if next <= offset
                    || next + size_of::<FILE_FULL_DIR_INFO>() > std::mem::size_of_val(&buffer)
                    || next % std::mem::align_of::<FILE_FULL_DIR_INFO>() != 0
                {
                    return Err("Invalid native project directory entry offset".into());
                }
                offset = next;
            }
        }
        self.verify()?;
        names.sort();
        Ok(names)
    }

    pub(super) fn same_object(&self, other: &Self) -> bool {
        self.node.identity == other.node.identity
    }

    pub(super) fn name_policy(&self) -> NamePolicy {
        directory_name_policy(&self.node.file)
    }

    pub(super) fn verify(&self) -> Result<(), String> {
        let mut current = Some(&self.node);
        while let Some(node) = current {
            if information(&node.file)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
                || identity(&node.file)? != node.identity
                || final_path(&node.file)? != node.path
            {
                return Err("Retained Windows directory identity or final path changed.".into());
            }
            let reopened = if let Some(parent) = &node.parent {
                let name = Name::new(node.path.file_name().ok_or("Missing directory name.")?)?;
                relative_open(
                    &parent.file,
                    &name,
                    DIRECTORY_ACCESS,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    FILE_OPEN,
                    FILE_DIRECTORY_FILE,
                )
                .map_err(nt_error)?
            } else {
                root_handle(&node.path)?
            };
            if information(&reopened)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
                || identity(&reopened)? != node.identity
                || final_path(&reopened)? != node.path
            {
                return Err(
                    "Windows directory pathname no longer names the retained object.".into(),
                );
            }
            current = node.parent.as_ref();
        }
        Ok(())
    }

    pub(super) fn child(&self, name: &OsStr) -> Result<Option<Self>, String> {
        let native = Name::new(name)?;
        self.verify()?;
        match relative_open(
            &self.node.file,
            &native,
            DIRECTORY_ACCESS,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_OPEN,
            FILE_DIRECTORY_FILE,
        ) {
            Ok(file) => {
                Self::retain(file, self.node.path.join(name), Some(self.node.clone())).map(Some)
            }
            Err(STATUS_OBJECT_NAME_NOT_FOUND) => Ok(None),
            Err(status) => Err(nt_error(status)),
        }
    }

    pub(super) fn absent(&self, name: &OsStr) -> Result<bool, String> {
        let native = Name::new(name)?;
        self.verify()?;
        match relative_open(
            &self.node.file,
            &native,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_OPEN,
            0,
        ) {
            Ok(_) => Ok(false), // Includes reparse entries, even dangling links.
            Err(STATUS_OBJECT_NAME_NOT_FOUND) => Ok(true),
            Err(status) => Err(nt_error(status)),
        }
    }

    pub(super) fn open_file(&self, name: &OsStr, write: bool) -> Result<File, String> {
        let native = Name::new(name)?;
        self.verify()?;
        let access = FILE_GENERIC_READ | if write { FILE_GENERIC_WRITE } else { 0 };
        // Validation must coexist with a retained publication handle carrying
        // WRITE and DELETE access. Sharing here grants no such access: that
        // retained handle still denies other writers and deleters.
        let share = FILE_SHARE_READ
            | if write {
                0
            } else {
                FILE_SHARE_WRITE | FILE_SHARE_DELETE
            };
        let file = relative_open(
            &self.node.file,
            &native,
            access,
            share,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE,
        )
        .map_err(nt_error)?;
        file_stamp(&file)?;
        if final_path(&file)? != self.node.path.join(name) {
            return Err("Windows file resolved to a different final path.".into());
        }
        self.verify()?;
        Ok(file)
    }

    pub(super) fn create_directory(&self, name: &OsStr) -> Result<Self, WriteFailure> {
        let before = |message| WriteFailure {
            message,
            uncertain: false,
        };
        let native = Name::new(name).map_err(before)?;
        self.verify().map_err(before)?;
        let path = self.node.path.join(name);
        // FILE_CREATE returns the newly created directory handle atomically;
        // an appeared directory is an error, never adopted as our own creation.
        let file = relative_open(
            &self.node.file,
            &native,
            DIRECTORY_ACCESS,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            FILE_CREATE,
            FILE_DIRECTORY_FILE,
        )
        .map_err(|status| creation_failure(status, &path))?;
        let result =
            Self::retain(file, path.clone(), Some(self.node.clone())).and_then(|directory| {
                self.verify()?;
                Ok(directory)
            });
        result.map_err(|message| WriteFailure {
            message: format!("Created directory {} may remain: {message}", path.display()),
            uncertain: true,
        })
    }

    pub(super) fn publish(&self, name: &OsStr, content: &[u8]) -> Result<File, WriteFailure> {
        let before = |message| WriteFailure {
            message,
            uncertain: false,
        };
        let native = Name::new(name).map_err(before)?;
        if content.len() > MAX_BYTES {
            return Err(before(
                "New Windows file exceeds the controlled-write byte limit.".into(),
            ));
        }
        self.verify().map_err(before)?;
        if !self.absent(name).map_err(before)? {
            return Err(before(
                "Reviewed new Windows file already exists; nothing was overwritten.".into(),
            ));
        }
        let (stage_name, mut stage) = self.stage()?;
        let stage_path = self.node.path.join(&stage_name);
        let target_path = self.node.path.join(name);
        let mut published = false;
        // File data is flushed before and after the namespace operation.
        // Windows exposes no portable parent-directory fsync guarantee.
        let result: Result<(), String> = (|| {
            file_stamp(&stage)?;
            if final_path(&stage)? != stage_path {
                return Err("Staging file resolved to a different final path.".into());
            }
            stage
                .write_all(content)
                .map_err(|error| format!("Cannot finish new-file staging: {error}"))?;
            stage
                .sync_all()
                .map_err(|error| format!("Cannot sync complete new-file staging: {error}"))?;
            file_stamp(&stage)?;
            self.verify()?;
            rename_no_replace(&stage, &self.node.file, &native)?;
            published = true;
            if final_path(&stage)? != target_path {
                return Err("Published file has an unexpected final path.".into());
            }
            file_stamp(&stage)?;
            stage
                .sync_all()
                .map_err(|error| format!("Published file sync failed: {error}"))?;
            self.verify()?;
            Ok(())
        })();
        match result {
            Ok(()) => Ok(stage),
            Err(message) if published => Err(WriteFailure {
                message: format!(
                    "New file {} was published; local effects may remain: {message}",
                    target_path.display()
                ),
                uncertain: true,
            }),
            Err(message) => {
                // Delete only the exact handle created by us, never a pathname
                // that another process might have substituted. Failure names
                // the residue instead of pretending cleanup/rollback succeeded.
                let cleanup = match final_path(&stage) {
                    Ok(path) if path == stage_path => dispose(&stage),
                    Ok(path) => {
                        return Err(WriteFailure {
                            message: format!(
                                "Publication failed but the owned file now resolves to {}; it was not deleted. Local effects are uncertain: {message}",
                                path.display()
                            ),
                            uncertain: true,
                        });
                    }
                    Err(error) => {
                        return Err(WriteFailure {
                            message: format!(
                                "Publication outcome and staging location {} could not be established ({error}); the owned handle was not deleted: {message}",
                                stage_path.display()
                            ),
                            uncertain: true,
                        });
                    }
                };
                drop(stage);
                let uncertain = cleanup.is_err();
                let message = match cleanup {
                    Ok(()) => {
                        format!("New file was not published; owned staging was removed: {message}")
                    }
                    Err(error) => format!(
                        "New file was not published; staging residue {} may remain ({error}): {message}",
                        stage_path.display()
                    ),
                };
                Err(WriteFailure { message, uncertain })
            }
        }
    }

    fn stage(&self) -> Result<(OsString, File), WriteFailure> {
        for _ in 0..128 {
            let name = OsString::from(format!(
                ".eevee-write-{}-{}.tmp",
                std::process::id(),
                NEXT_STAGE.fetch_add(1, Ordering::Relaxed),
            ));
            let native = Name::new(&name)?;
            match relative_open(
                &self.node.file,
                &native,
                FILE_GENERIC_READ | FILE_GENERIC_WRITE | DELETE,
                FILE_SHARE_READ,
                FILE_CREATE,
                FILE_NON_DIRECTORY_FILE,
            ) {
                Ok(file) => return Ok((name, file)),
                Err(STATUS_OBJECT_NAME_COLLISION) => continue,
                Err(status) => return Err(creation_failure(status, &self.node.path.join(&name))),
            }
        }
        Err(WriteFailure {
            message: "Cannot reserve an exclusive Windows staging name; target was not published."
                .into(),
            uncertain: false,
        })
    }
}

// FILE_RENAME_INFORMATION has the same layout as FILE_RENAME_INFO. Use the
// native class explicitly: RootDirectory anchors the destination, and the zero
// ReplaceIfExists/Flags field guarantees failure for *any* appeared entry.
fn rename_no_replace(file: &File, parent: &File, name: &Name) -> Result<(), String> {
    let bytes = (offset_of!(FILE_RENAME_INFO, FileName) + name.length * 2)
        .max(size_of::<FILE_RENAME_INFO>());
    let mut storage = [0usize; (size_of::<FILE_RENAME_INFO>() + 510).div_ceil(size_of::<usize>())];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).RootDirectory = parent.as_raw_handle();
        (*info).FileNameLength = (name.length * 2) as u32;
        std::ptr::copy_nonoverlapping(
            name.units.as_ptr(),
            (*info).FileName.as_mut_ptr(),
            name.length,
        );
    }
    let mut io_status = IoStatusBlock {
        status_or_pointer: 0,
        information: 0,
    };
    let status = unsafe {
        NtSetInformationFile(
            file.as_raw_handle(),
            &mut io_status,
            info.cast(),
            bytes as u32,
            FILE_RENAME_INFORMATION,
        )
    };
    if status < 0 {
        return Err(format!(
            "No-replace Windows publication refused: {}",
            nt_error(status)
        ));
    }
    Ok(())
}

fn dispose(file: &File) -> Result<(), String> {
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileDispositionInfo,
            (&info as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(())
}

pub(super) fn file_stamp(file: &File) -> Result<FileStamp, String> {
    let info = information(file)?;
    let size = (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow);
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 || info.nNumberOfLinks != 1 {
        return Err("Windows writes require a regular file with exactly one hard link.".into());
    }
    if size > MAX_BYTES as u64 {
        return Err("Windows file exceeds the controlled-write byte limit.".into());
    }
    let mut basic = FILE_BASIC_INFO::default();
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            size_of::<FILE_BASIC_INFO>() as u32,
        )
    } == 0
    {
        return Err(format!(
            "Cannot inspect Windows file change times: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(FileStamp {
        identity: identity(file)?,
        size,
        created: basic.CreationTime,
        written: basic.LastWriteTime,
        changed: basic.ChangeTime,
        attributes: basic.FileAttributes,
        links: info.nNumberOfLinks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Read, Seek, SeekFrom};
    use std::os::windows::fs::{symlink_dir, symlink_file};

    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "eevee-windows-writes-{}-{}",
                std::process::id(),
                NEXT_STAGE.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }

        fn directory(&self) -> Directory {
            Directory::open_root(&self.0).unwrap()
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn has_symlink_privilege(result: io::Result<()>) -> bool {
        match result {
            Ok(()) => true,
            Err(error) if error.raw_os_error() == Some(1314) => {
                eprintln!("Windows symlink privilege unavailable; reparse fixture skipped");
                false
            }
            Err(error) => panic!("cannot create Windows reparse fixture: {error}"),
        }
    }

    #[test]
    fn complete_empty_and_unicode_publication_leaves_no_staging() {
        let temp = Temp::new();
        let directory = temp.directory();
        directory.publish(OsStr::new("empty.txt"), b"").unwrap();
        directory
            .publish(OsStr::new("새 파일.txt"), "complete\r\n내용 🐈".as_bytes())
            .unwrap();
        directory
            .publish(OsStr::new("a"), b"short native rename buffer")
            .unwrap();
        assert_eq!(fs::read(temp.0.join("empty.txt")).unwrap(), b"");
        assert_eq!(
            fs::read_to_string(temp.0.join("새 파일.txt")).unwrap(),
            "complete\r\n내용 🐈"
        );
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 3);
        directory.verify().unwrap();
    }

    #[test]
    fn retained_publication_allows_named_validation_but_excludes_writers_and_deletion() {
        let temp = Temp::new();
        let directory = temp.directory();
        let name = OsStr::new("retained.txt");
        let mut retained = directory.publish(name, b"exact\r\nbytes").unwrap();
        let mut named = directory.open_file(name, false).unwrap();
        assert_eq!(file_stamp(&retained).unwrap(), file_stamp(&named).unwrap());
        let mut retained_bytes = Vec::new();
        retained.seek(SeekFrom::Start(0)).unwrap();
        retained.read_to_end(&mut retained_bytes).unwrap();
        let mut named_bytes = Vec::new();
        named.read_to_end(&mut named_bytes).unwrap();
        assert_eq!(retained_bytes, b"exact\r\nbytes");
        assert_eq!(retained_bytes, named_bytes);
        assert!(directory.open_file(name, true).is_err());
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .open(temp.0.join(name))
                .is_err()
        );
        assert!(fs::remove_file(temp.0.join(name)).is_err());
        assert!(fs::rename(temp.0.join(name), temp.0.join("moved.txt")).is_err());
        drop(named);
        assert!(fs::remove_file(temp.0.join(name)).is_err());
        drop(retained);
        fs::remove_file(temp.0.join(name)).unwrap();
    }

    #[test]
    fn native_no_replace_preserves_raced_file_and_directory() {
        let temp = Temp::new();
        let directory = temp.directory();
        for is_directory in [false, true] {
            let target = if is_directory {
                "directory"
            } else {
                "file.txt"
            };
            assert!(directory.absent(OsStr::new(target)).unwrap());
            let (stage_name, mut stage) = directory.stage().unwrap();
            stage
                .write_all(b"complete replacement must not win")
                .unwrap();
            stage.sync_all().unwrap();
            let path = temp.0.join(target);
            if is_directory {
                fs::create_dir(&path).unwrap();
            } else {
                fs::write(&path, b"raced original").unwrap();
            }
            assert!(
                rename_no_replace(
                    &stage,
                    &directory.node.file,
                    &Name::new(OsStr::new(target)).unwrap()
                )
                .is_err()
            );
            assert!(temp.0.join(&stage_name).exists());
            dispose(&stage).unwrap();
            drop(stage);
            assert!(!temp.0.join(stage_name).exists());
            if is_directory {
                assert!(path.is_dir());
            } else {
                assert_eq!(fs::read(path).unwrap(), b"raced original");
            }
            assert!(
                directory
                    .publish(OsStr::new(target), b"must not overwrite")
                    .is_err()
            );
        }
    }

    #[test]
    fn retained_chain_and_clones_block_ancestor_rename_until_last_drop() {
        let temp = Temp::new();
        fs::create_dir(temp.0.join("parent")).unwrap();
        fs::create_dir(temp.0.join("parent/child")).unwrap();
        let root = temp.directory();
        let parent = root.child(OsStr::new("parent")).unwrap().unwrap();
        let child = parent.child(OsStr::new("child")).unwrap().unwrap();
        let retained = child.clone();
        drop(child);
        drop(parent);
        drop(root);
        assert!(fs::rename(temp.0.join("parent"), temp.0.join("moved")).is_err());
        assert!(fs::remove_dir(temp.0.join("parent/child")).is_err());
        retained.verify().unwrap();
        retained
            .publish(OsStr::new("anchored.txt"), b"anchored")
            .unwrap();
        drop(retained);
        fs::rename(temp.0.join("parent"), temp.0.join("moved")).unwrap();
        assert_eq!(
            fs::read(temp.0.join("moved/child/anchored.txt")).unwrap(),
            b"anchored"
        );
    }

    #[test]
    fn exclusive_missing_directory_creation_keeps_native_created_handle() {
        let temp = Temp::new();
        let root = temp.directory();
        assert!(root.child(OsStr::new("nested")).unwrap().is_none());
        let nested = root.create_directory(OsStr::new("nested")).unwrap();
        assert!(!root.absent(OsStr::new("nested")).unwrap());
        let failure = root.create_directory(OsStr::new("nested")).unwrap_err();
        assert!(!failure.uncertain);
        assert_eq!(
            nested.node.identity,
            root.child(OsStr::new("nested"))
                .unwrap()
                .unwrap()
                .node
                .identity
        );
        let deeper = nested.create_directory(OsStr::new("deeper")).unwrap();
        deeper.publish(OsStr::new("file"), b"nested").unwrap();
        root.verify().unwrap(); // Child creation does not stale directory times.
        nested.verify().unwrap();
        assert_eq!(
            fs::read(temp.0.join("nested/deeper/file")).unwrap(),
            b"nested"
        );
        fs::write(temp.0.join("not-a-directory"), b"file").unwrap();
        assert!(root.child(OsStr::new("not-a-directory")).is_err());
        assert!(
            !root
                .create_directory(OsStr::new("not-a-directory"))
                .unwrap_err()
                .uncertain
        );
    }

    #[test]
    fn native_ids_distinguish_equal_content_and_replaced_entries() {
        let temp = Temp::new();
        fs::write(temp.0.join("first"), b"same").unwrap();
        fs::write(temp.0.join("second"), b"same").unwrap();
        let root = temp.directory();
        let first = root.open_file(OsStr::new("first"), false).unwrap();
        let old = file_stamp(&first).unwrap();
        assert_eq!(old, file_stamp(&first).unwrap());
        let second = root.open_file(OsStr::new("second"), false).unwrap();
        assert_ne!(old.identity, file_stamp(&second).unwrap().identity);
        drop(second);
        drop(first);
        fs::rename(temp.0.join("first"), temp.0.join("retired")).unwrap();
        fs::write(temp.0.join("first"), b"same").unwrap();
        assert_ne!(
            old.identity,
            file_stamp(&root.open_file(OsStr::new("first"), false).unwrap())
                .unwrap()
                .identity
        );
    }

    #[test]
    fn baseline_revalidation_can_read_while_exclusive_writer_is_retained() {
        let temp = Temp::new();
        fs::write(temp.0.join("selected"), b"reviewed dirty baseline").unwrap();
        let root = temp.directory();
        let writer = root.open_file(OsStr::new("selected"), true).unwrap();
        let baseline = file_stamp(&writer).unwrap();
        root.verify().unwrap();
        let mut reader = root.open_file(OsStr::new("selected"), false).unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"reviewed dirty baseline");
        assert_eq!(file_stamp(&reader).unwrap(), baseline);
        assert!(root.open_file(OsStr::new("selected"), true).is_err());
        assert!(
            OpenOptions::new()
                .write(true)
                .open(temp.0.join("selected"))
                .is_err()
        );
        assert!(fs::remove_file(temp.0.join("selected")).is_err());
        root.verify().unwrap();
        drop(reader);
        drop(writer);
    }

    #[test]
    fn opened_write_handle_denies_writers_deletion_and_truncates_only_explicitly() {
        let temp = Temp::new();
        fs::write(temp.0.join("dirty"), b"dirty working bytes").unwrap();
        let root = temp.directory();
        let mut file = root.open_file(OsStr::new("dirty"), true).unwrap();
        let original = file_stamp(&file).unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        assert_eq!(text, "dirty working bytes");
        assert!(
            OpenOptions::new()
                .write(true)
                .open(temp.0.join("dirty"))
                .is_err()
        );
        assert!(fs::remove_file(temp.0.join("dirty")).is_err());
        assert!(fs::rename(temp.0.join("dirty"), temp.0.join("moved")).is_err());
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(b"new").unwrap();
        file.set_len(3).unwrap();
        file.sync_all().unwrap();
        let updated = file_stamp(&file).unwrap();
        assert_eq!(original.identity, updated.identity);
        assert_ne!(original, updated);
        drop(file);
        assert_eq!(fs::read(temp.0.join("dirty")).unwrap(), b"new");
    }

    #[test]
    fn hardlinks_are_refused_by_open_and_retained_handle_stamp() {
        let temp = Temp::new();
        let root = temp.directory();
        fs::write(temp.0.join("original"), b"must stay").unwrap();
        fs::hard_link(temp.0.join("original"), temp.0.join("alias")).unwrap();
        assert!(root.open_file(OsStr::new("original"), false).is_err());
        assert!(root.open_file(OsStr::new("alias"), true).is_err());
        let raw = File::open(temp.0.join("original")).unwrap();
        assert!(file_stamp(&raw).is_err());
        drop(raw);
        assert!(
            !root
                .publish(OsStr::new("alias"), b"overwrite")
                .unwrap_err()
                .uncertain
        );
        assert_eq!(fs::read(temp.0.join("original")).unwrap(), b"must stay");
    }

    #[test]
    fn reparse_ancestors_leaves_and_dangling_names_are_never_adopted() {
        let temp = Temp::new();
        let outside = Temp::new();
        fs::write(outside.0.join("original"), b"outside").unwrap();
        if !has_symlink_privilege(symlink_dir(&outside.0, temp.0.join("redirect"))) {
            return;
        }
        let root = temp.directory();
        assert!(root.child(OsStr::new("redirect")).is_err());
        assert!(Directory::open_root(&temp.0.join("redirect")).is_err());
        assert!(!root.absent(OsStr::new("redirect")).unwrap());
        assert!(root.create_directory(OsStr::new("redirect")).is_err());
        assert!(has_symlink_privilege(symlink_file(
            outside.0.join("original"),
            temp.0.join("leaf")
        )));
        assert!(root.open_file(OsStr::new("leaf"), false).is_err());
        assert!(root.open_file(OsStr::new("leaf"), true).is_err());
        assert!(has_symlink_privilege(symlink_file(
            outside.0.join("missing"),
            temp.0.join("dangling")
        )));
        assert!(!root.absent(OsStr::new("dangling")).unwrap());
        assert!(
            root.publish(OsStr::new("dangling"), b"must not follow")
                .is_err()
        );
        let (_, stage) = root.stage().unwrap();
        assert!(
            rename_no_replace(
                &stage,
                &root.node.file,
                &Name::new(OsStr::new("dangling")).unwrap()
            )
            .is_err()
        );
        dispose(&stage).unwrap();
        drop(stage);
        assert!(!outside.0.join("missing").exists());
        assert_eq!(fs::read(outside.0.join("original")).unwrap(), b"outside");
    }

    #[test]
    fn device_ads_non_normal_and_oversized_inputs_never_mutate() {
        let temp = Temp::new();
        let root = temp.directory();
        for name in [
            "",
            ".",
            "..",
            "CON",
            "nul.txt",
            "COM1",
            "LPT³.txt",
            "CONOUT$",
            "file:stream",
            "sub/file",
            "sub\\file",
            "trailing.",
            "trailing ",
        ] {
            assert!(root.publish(OsStr::new(name), b"no").is_err(), "{name}");
            assert!(root.create_directory(OsStr::new(name)).is_err(), "{name}");
            assert!(root.open_file(OsStr::new(name), true).is_err(), "{name}");
        }
        assert!(
            !root
                .publish(OsStr::new("too-large"), &vec![b'x'; MAX_BYTES + 1])
                .unwrap_err()
                .uncertain
        );
        fs::write(temp.0.join("existing-large"), vec![b'x'; MAX_BYTES + 1]).unwrap();
        assert!(root.open_file(OsStr::new("existing-large"), false).is_err());
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
    }
}
