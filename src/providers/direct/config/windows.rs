//! Handle-bound checks for user-owned Windows provider configuration.
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::mem::size_of;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::ptr::{null_mut, read_unaligned};

use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HANDLE, LocalFree};
use windows_sys::Win32::Foundation::{GENERIC_EXECUTE, GENERIC_READ};
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
use windows_sys::Win32::Security::{
    ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetTokenInformation,
    INHERIT_ONLY_ACE, IsValidAcl, IsValidSid, IsWellKnownSid, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser as TOKEN_USER_CLASS,
    WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DEVICE, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_EXECUTE,
    FILE_GENERIC_READ, FILE_NAME_NORMALIZED, FILE_SHARE_READ, FILE_TYPE_DISK,
    GetFileInformationByHandle, GetFileType, GetFinalPathNameByHandleW, SYNCHRONIZE,
    VOLUME_NAME_DOS,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

// The ordinary allow/deny ACE tags from winnt.h. Other ACE layouts are deliberately
// unsupported: object-specific and conditional grants cannot be treated as simple SIDs.
const ALLOW_ACE: u8 = 0;
const DENY_ACE: u8 = 1;
const ERROR: &str = "Cannot read user provider file safely: require a regular, user-owned file with a trusted Windows ACL and unchanged resolved path.";

struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // GetSecurityInfo allocates the descriptor with LocalAlloc.
            unsafe { LocalFree(self.0) };
        }
    }
}

struct TokenUser {
    // Pointer-sized storage keeps TOKEN_USER and its embedded SID correctly aligned.
    storage: Vec<usize>,
}

impl TokenUser {
    fn current() -> Result<Self, String> {
        let mut raw = null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
            return Err(ERROR.into());
        }
        // Ownership transfers immediately, including all error paths below.
        let token = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut length = 0;
        unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TOKEN_USER_CLASS,
                null_mut(),
                0,
                &mut length,
            );
        }
        if length < size_of::<TOKEN_USER>() as u32 || length > 64 * 1024 {
            return Err(ERROR.into());
        }
        let mut user = Self {
            storage: vec![0; (length as usize).div_ceil(size_of::<usize>())],
        };
        let capacity = (user.storage.len() * size_of::<usize>()) as u32;
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TOKEN_USER_CLASS,
                user.storage.as_mut_ptr().cast(),
                capacity,
                &mut length,
            )
        } == 0
            || length < size_of::<TOKEN_USER>() as u32
            || length > capacity
        {
            return Err(ERROR.into());
        }
        if unsafe { IsValidSid(user.sid()) } == 0 {
            return Err(ERROR.into());
        }
        Ok(user)
    }

    fn sid(&self) -> PSID {
        // The successful TokenUser query owns both this structure and the pointed-to SID.
        unsafe { (*(self.storage.as_ptr().cast::<TOKEN_USER>())).User.Sid }
    }
}

fn final_path(handle: HANDLE) -> Result<PathBuf, String> {
    let flags = FILE_NAME_NORMALIZED | VOLUME_NAME_DOS;
    let needed = unsafe { GetFinalPathNameByHandleW(handle, null_mut(), 0, flags) };
    if needed == 0 || needed > 32768 {
        return Err(ERROR.into());
    }
    let mut buffer = vec![0; needed as usize];
    let length = unsafe {
        GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, flags)
    };
    if length == 0 || length >= buffer.len() as u32 {
        return Err(ERROR.into());
    }
    buffer.truncate(length as usize);
    Ok(PathBuf::from(OsString::from_wide(&buffer)))
}

fn trusted_acl(handle: HANDLE, credential: bool) -> Result<(), String> {
    let user = TokenUser::current()?;
    let mut owner = null_mut();
    let mut acl = null_mut();
    let mut descriptor = SecurityDescriptor(null_mut());
    let result = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor.0,
        )
    };
    if result != ERROR_SUCCESS
        || descriptor.0.is_null()
        || owner.is_null()
        || acl.is_null()
        || unsafe { IsValidSid(owner) } == 0
        || unsafe { EqualSid(owner, user.sid()) } == 0
        || unsafe { IsValidAcl(acl) } == 0
    {
        return Err(ERROR.into());
    }
    // A NULL DACL grants everyone access; an empty (non-NULL) ACL does not.
    check_acl(acl, user.sid(), credential)
}

fn check_acl(acl: *const ACL, owner: PSID, credential: bool) -> Result<(), String> {
    // Only called with a valid ACL owned by a live security descriptor (or test buffer).
    let header = unsafe { read_unaligned(acl) };
    let base = acl as usize;
    let end = base.checked_add(header.AclSize as usize).ok_or(ERROR)?;
    for index in 0..u32::from(header.AceCount) {
        let mut raw = null_mut();
        if unsafe { GetAce(acl, index, &mut raw) } == 0 || raw.is_null() {
            return Err(ERROR.into());
        }
        let start = raw as usize;
        if start < base + size_of::<ACL>()
            || start
                .checked_add(size_of::<ACE_HEADER>())
                .is_none_or(|n| n > end)
        {
            return Err(ERROR.into());
        }
        let ace = unsafe { read_unaligned(raw.cast::<ACE_HEADER>()) };
        let length = usize::from(ace.AceSize);
        if length < size_of::<ACE_HEADER>() || start.checked_add(length).is_none_or(|n| n > end) {
            return Err(ERROR.into());
        }
        match ace.AceType {
            DENY_ACE => continue,
            ALLOW_ACE => {}
            _ => return Err(ERROR.into()),
        }
        if u32::from(ace.AceFlags) & INHERIT_ONLY_ACE != 0 {
            continue;
        }
        // ACCESS_ALLOWED_ACE is a header, access mask, then a variable-length SID.
        if length < 16 {
            return Err(ERROR.into());
        }
        let bytes = raw.cast::<u8>();
        let mask = unsafe { read_unaligned(bytes.add(4).cast::<u32>()) };
        let sid = unsafe { bytes.add(8).cast() };
        let sid_length = 8 + 4 * usize::from(unsafe { *bytes.add(9) });
        if sid_length > length - 8 || unsafe { IsValidSid(sid) } == 0 {
            return Err(ERROR.into());
        }
        if unsafe {
            EqualSid(sid, owner) != 0
                || IsWellKnownSid(sid, WinLocalSystemSid) != 0
                || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0
        } {
            continue;
        }
        // Unknown access bits fail closed as well. Synchronize alone cannot expose data.
        let allowed = if credential {
            SYNCHRONIZE
        } else {
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE | GENERIC_READ | GENERIC_EXECUTE
        };
        if mask & !allowed != 0 {
            return Err(ERROR.into());
        }
    }
    Ok(())
}

/// `expected_canonical` must be the canonical path already checked against the project
/// root by the caller. Compare against the opened handle, not another path lookup: a
/// parent junction changed between canonicalization and open must not bypass that check.
pub(super) fn open_user_file(
    path: &Path,
    expected_canonical: &Path,
    credential: bool,
) -> Result<File, String> {
    if !path.is_absolute() || !expected_canonical.is_absolute() {
        return Err(ERROR.into());
    }
    let file = OpenOptions::new()
        .access_mode(FILE_GENERIC_READ)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| ERROR.to_string())?;
    let handle = file.as_raw_handle();
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileType(handle) } != FILE_TYPE_DISK
        || unsafe { GetFileInformationByHandle(handle, &mut info) } == 0
        || info.dwFileAttributes
            & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_DEVICE)
            != 0
        || final_path(handle)? != expected_canonical
    {
        return Err(ERROR.into());
    }
    trusted_acl(handle, credential)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::mem::size_of_val;
    use std::os::windows::fs::{symlink_dir, symlink_file};
    use std::sync::atomic::{AtomicU64, Ordering};
    use windows_sys::Win32::Security::Authorization::SetSecurityInfo;
    use windows_sys::Win32::Security::{
        ACL_REVISION, AddAccessAllowedAceEx, CreateWellKnownSid, InitializeAcl,
        PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_MAX_SID_SIZE, WinWorldSid,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ALL_ACCESS, FILE_GENERIC_WRITE, WRITE_DAC, WRITE_OWNER,
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Temp(PathBuf);

    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "eevee-windows-config-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path.canonicalize().unwrap())
        }

        fn file(&self, name: &str, world_rights: Option<u32>) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, "configuration").unwrap();
            let file = OpenOptions::new()
                .access_mode(WRITE_DAC | WRITE_OWNER)
                .open(&path)
                .unwrap();
            let user = TokenUser::current().unwrap();
            let mut world = [0u32; (SECURITY_MAX_SID_SIZE as usize).div_ceil(4)];
            let mut world_length = size_of_val(&world) as u32;
            assert_ne!(
                unsafe {
                    CreateWellKnownSid(
                        WinWorldSid,
                        null_mut(),
                        world.as_mut_ptr().cast(),
                        &mut world_length,
                    )
                },
                0
            );
            let mut storage = [0u32; 256];
            let acl = storage.as_mut_ptr().cast::<ACL>();
            assert_ne!(
                unsafe { InitializeAcl(acl, size_of_val(&storage) as u32, ACL_REVISION) },
                0
            );
            assert_ne!(
                unsafe { AddAccessAllowedAceEx(acl, ACL_REVISION, 0, FILE_ALL_ACCESS, user.sid()) },
                0
            );
            if let Some(rights) = world_rights {
                assert_ne!(
                    unsafe {
                        AddAccessAllowedAceEx(
                            acl,
                            ACL_REVISION,
                            0,
                            rights,
                            world.as_mut_ptr().cast(),
                        )
                    },
                    0
                );
            }
            assert_eq!(
                unsafe {
                    SetSecurityInfo(
                        file.as_raw_handle(),
                        SE_FILE_OBJECT,
                        OWNER_SECURITY_INFORMATION
                            | DACL_SECURITY_INFORMATION
                            | PROTECTED_DACL_SECURITY_INFORMATION,
                        user.sid(),
                        null_mut(),
                        acl,
                        null_mut(),
                    )
                },
                ERROR_SUCCESS
            );
            path
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn private_and_world_writable_configuration() {
        let temp = Temp::new();
        let private = temp.file("private.toml", None);
        assert!(open_user_file(&private, &private.canonicalize().unwrap(), false).is_ok());
        assert!(open_user_file(&private, &private.canonicalize().unwrap(), true).is_ok());
        let public = temp.file("public.toml", Some(FILE_GENERIC_READ));
        assert!(open_user_file(&public, &public.canonicalize().unwrap(), false).is_ok());
        assert!(open_user_file(&public, &public.canonicalize().unwrap(), true).is_err());
        let writable = temp.file("writable.toml", Some(FILE_GENERIC_WRITE));
        assert!(open_user_file(&writable, &writable.canonicalize().unwrap(), false).is_err());
    }

    #[test]
    fn leaf_reparse_and_resolved_identity() {
        let temp = Temp::new();
        let original = temp.file("original.toml", None);
        let alternate = temp.file("alternate.toml", None);
        let original_canonical = original.canonicalize().unwrap();
        assert!(open_user_file(&alternate, &original_canonical, false).is_err());
        assert!(open_user_file(&temp.0, &temp.0, false).is_err());
        assert!(open_user_file(Path::new("relative.toml"), &original_canonical, false).is_err());
        let link = temp.0.join("leaf.toml");
        match symlink_file(&original, &link) {
            Ok(()) => {
                assert!(open_user_file(&link, &original_canonical, false).is_err());
                fs::remove_file(link).unwrap();
            }
            Err(error) if error.raw_os_error() == Some(1314) => {
                eprintln!("Windows symlink privilege unavailable; leaf-reparse assertion skipped");
            }
            Err(error) => panic!("cannot create reparse fixture: {error}"),
        }
        // A returned handle excludes writers and deletion for the duration of the read.
        let opened = open_user_file(&original, &original_canonical, false).unwrap();
        assert!(OpenOptions::new().write(true).open(&original).is_err());
        assert!(fs::remove_file(&original).is_err());
        drop(opened);
    }

    #[test]
    fn null_dacl_and_unknown_grants_fail_closed() {
        let temp = Temp::new();
        let path = temp.file("null-dacl.toml", None);
        let file = OpenOptions::new()
            .access_mode(WRITE_DAC)
            .open(&path)
            .unwrap();
        assert_eq!(
            unsafe {
                SetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            },
            ERROR_SUCCESS
        );
        drop(file);
        assert!(open_user_file(&path, &path.canonicalize().unwrap(), false).is_err());

        let user = TokenUser::current().unwrap();
        let mut storage = [0u32; 256];
        let acl = storage.as_mut_ptr().cast::<ACL>();
        assert_ne!(
            unsafe { InitializeAcl(acl, size_of_val(&storage) as u32, ACL_REVISION) },
            0
        );
        assert_ne!(
            unsafe { AddAccessAllowedAceEx(acl, ACL_REVISION, 0, FILE_ALL_ACCESS, user.sid()) },
            0
        );
        assert!(check_acl(acl, user.sid(), true).is_ok());
        let mut ace = null_mut();
        assert_ne!(unsafe { GetAce(acl, 0, &mut ace) }, 0);
        // ACCESS_ALLOWED_CALLBACK_ACE is deliberately not interpreted as an ordinary grant.
        unsafe { (*ace.cast::<ACE_HEADER>()).AceType = 9 };
        assert!(check_acl(acl, user.sid(), false).is_err());
    }

    #[test]
    fn parent_reparse_redirect_cannot_change_checked_target() {
        let safe = Temp::new();
        let project = Temp::new();
        let links = Temp::new();
        let original = safe.file("provider.toml", None);
        project.file("provider.toml", None);
        let link = links.0.join("parent");
        match symlink_dir(&safe.0, &link) {
            Ok(()) => {}
            Err(error) if error.raw_os_error() == Some(1314) => {
                eprintln!(
                    "Windows symlink privilege unavailable; parent-reparse assertion skipped"
                );
                return;
            }
            Err(error) => panic!("cannot create parent-reparse fixture: {error}"),
        }
        let requested = link.join("provider.toml");
        let expected = requested.canonicalize().unwrap();
        assert_eq!(expected, original.canonicalize().unwrap());
        assert!(open_user_file(&requested, &expected, false).is_ok());
        fs::remove_dir(&link).unwrap();
        symlink_dir(&project.0, &link).unwrap();
        assert!(open_user_file(&requested, &expected, false).is_err());
        fs::remove_dir(&link).unwrap();
    }
}
