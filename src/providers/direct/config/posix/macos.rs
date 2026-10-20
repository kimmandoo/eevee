//! Darwin extended ACL checks are independent of Unix mode bits.
use std::ffi::c_void;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::ptr::null_mut;

const EXTENDED: i32 = 0x100;
const ACL_PROPERTY: i32 = 5;
const ALLOW: i32 = 1;
const DENY: i32 = 2;
const MAX_ENTRIES: i32 = 128;
const READ_ONLY: u64 = (1 << 1) | (1 << 3) | (1 << 7) | (1 << 9) | (1 << 11) | (1 << 20);
const ERROR: &str = "Cannot establish safe macOS ACL privacy. Existing HOME/provider/store ACLs are not modified; use session-only or repair permissions explicitly.";

type Acl = *mut c_void;
type Entry = *mut c_void;
unsafe extern "C" {
    fn filesec_init() -> *mut c_void;
    fn filesec_free(security: *mut c_void);
    fn filesec_query_property(security: *mut c_void, property: i32, present: *mut i32) -> i32;
    fn filesec_get_property(security: *mut c_void, property: i32, value: *mut c_void) -> i32;
    #[cfg_attr(target_arch = "x86_64", link_name = "fstatx_np$INODE64")]
    fn fstatx_np(fd: i32, metadata: *mut libc::stat, security: *mut c_void) -> i32;
    fn acl_set_fd_np(fd: i32, acl: Acl, kind: i32) -> i32;
    fn acl_get_entry(acl: Acl, index: i32, entry: *mut Entry) -> i32;
    fn acl_get_tag_type(entry: Entry, tag: *mut i32) -> i32;
    fn acl_get_permset_mask_np(entry: Entry, permissions: *mut u64) -> i32;
    fn acl_valid(acl: Acl) -> i32;
    fn acl_init(count: i32) -> Acl;
    fn acl_free(acl: Acl) -> i32;
}

struct FileSecurity(*mut c_void);
impl Drop for FileSecurity {
    fn drop(&mut self) {
        unsafe { filesec_free(self.0) };
    }
}

struct OwnedAcl(Acl);
impl Drop for OwnedAcl {
    fn drop(&mut self) {
        unsafe { acl_free(self.0) };
    }
}
impl OwnedAcl {
    fn read(file: &File) -> Result<(libc::stat, Option<Self>), String> {
        let security = unsafe { filesec_init() };
        if security.is_null() {
            return Err(ERROR.into());
        }
        let security = FileSecurity(security);
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        let mut present = 0;
        if unsafe { fstatx_np(file.as_raw_fd(), metadata.as_mut_ptr(), security.0) } != 0
            || unsafe { filesec_query_property(security.0, ACL_PROPERTY, &mut present) } != 0
        {
            return Err(ERROR.into());
        }
        // The mode/owner and ACL come from the same handle-bound snapshot.
        let metadata = unsafe { metadata.assume_init() };
        // Only a successful explicit absence query means there is no ACL.
        // Missing/unsupported metadata and all retrieval failures remain errors.
        if present == 0 {
            return Ok((metadata, None));
        }
        let mut acl: Acl = null_mut();
        if unsafe { filesec_get_property(security.0, ACL_PROPERTY, (&mut acl as *mut Acl).cast()) }
            != 0
            || acl.is_null()
        {
            return Err(ERROR.into());
        }
        let acl = Self(acl);
        if unsafe { acl_valid(acl.0) } != 0 {
            return Err(ERROR.into());
        }
        Ok((metadata, Some(acl)))
    }
    fn entry(&self, index: i32) -> Result<Option<Entry>, String> {
        let mut entry = null_mut();
        if unsafe { acl_get_entry(self.0, index, &mut entry) } == 0 {
            if entry.is_null() {
                return Err(ERROR.into());
            }
            return Ok(Some(entry));
        }
        // Darwin returns -1/EINVAL at the end, unlike the POSIX 1/0 convention.
        if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINVAL) {
            Ok(None)
        } else {
            Err(ERROR.into())
        }
    }
}

/// Denies cannot broaden access. Allow entries on private objects fail closed,
/// including owner-only grants, rather than attempting directory-service/group
/// membership interpretation. Nonsecret objects permit read/search grants only.
pub(super) fn check(file: &File, private: bool) -> Result<(), String> {
    let (metadata, acl) = OwnedAcl::read(file)?;
    if metadata.st_uid != unsafe { libc::geteuid() }
        || metadata.st_mode & (if private { 0o077 } else { 0o022 }) != 0
    {
        return Err(ERROR.into());
    }
    let Some(acl) = acl else {
        return Ok(());
    };
    for index in 0..=MAX_ENTRIES {
        let Some(entry) = acl.entry(index)? else {
            return Ok(());
        };
        if index == MAX_ENTRIES {
            return Err(ERROR.into());
        }
        let mut tag = 0;
        if unsafe { acl_get_tag_type(entry, &mut tag) } != 0 {
            return Err(ERROR.into());
        }
        match tag {
            DENY => continue,
            ALLOW => {}
            _ => return Err(ERROR.into()),
        }
        let mut permissions = 0;
        if unsafe { acl_get_permset_mask_np(entry, &mut permissions) } != 0
            || permissions & !(if private { 0 } else { READ_ONLY }) != 0
        {
            return Err(ERROR.into());
        }
    }
    Err(ERROR.into())
}

/// Only the successful mkdirat/O_EXCL creation paths call this, before secrets
/// are written. Existing HOME, provider-owned files and existing store objects
/// never pass through this function.
pub(super) fn clear_new(file: &File) -> Result<(), String> {
    let (metadata, inherited) = OwnedAcl::read(file)?;
    let kind = metadata.st_mode & libc::S_IFMT;
    if metadata.st_uid != unsafe { libc::geteuid() }
        || metadata.st_mode & 0o077 != 0
        || !(kind == libc::S_IFDIR
            || kind == libc::S_IFREG && metadata.st_size == 0 && metadata.st_nlink == 1)
    {
        return Err(ERROR.into());
    }
    let Some(inherited) = inherited else {
        return Ok(());
    };
    if inherited.entry(0)?.is_none() {
        return check(file, true);
    }
    let empty = unsafe { acl_init(0) };
    if empty.is_null() {
        return Err(ERROR.into());
    }
    let empty = OwnedAcl(empty);
    if unsafe { acl_set_fd_np(file.as_raw_fd(), empty.0, EXTENDED) } != 0 {
        return Err(ERROR.into());
    }
    check(file, true)
}

#[cfg(test)]
pub(super) fn grant_for_test(file: &File, permissions: u64, inherit: bool) {
    unsafe extern "C" {
        fn acl_create_entry(acl: *mut Acl, entry: *mut Entry) -> i32;
        fn acl_set_tag_type(entry: Entry, tag: i32) -> i32;
        fn acl_set_qualifier(entry: Entry, qualifier: *const c_void) -> i32;
        fn acl_set_permset_mask_np(entry: Entry, permissions: u64) -> i32;
        fn acl_get_flagset_np(object: *mut c_void, flags: *mut *mut c_void) -> i32;
        fn acl_add_flag_np(flags: *mut c_void, flag: i32) -> i32;
        fn mbr_uid_to_uuid(uid: libc::uid_t, uuid: *mut u8) -> i32;
    }
    let foreign_uid = if unsafe { libc::geteuid() } == 65534 {
        65533
    } else {
        65534
    };
    let mut principal = [0u8; 16];
    assert_eq!(
        unsafe { mbr_uid_to_uuid(foreign_uid, principal.as_mut_ptr()) },
        0
    );
    let mut acl = OwnedAcl(unsafe { acl_init(1) });
    assert!(!acl.0.is_null());
    let mut entry = null_mut();
    assert_eq!(unsafe { acl_create_entry(&mut acl.0, &mut entry) }, 0);
    assert_eq!(unsafe { acl_set_tag_type(entry, ALLOW) }, 0);
    assert_eq!(
        unsafe { acl_set_qualifier(entry, principal.as_ptr().cast()) },
        0
    );
    assert_eq!(unsafe { acl_set_permset_mask_np(entry, permissions) }, 0);
    if inherit {
        let mut flags = null_mut();
        assert_eq!(unsafe { acl_get_flagset_np(entry, &mut flags) }, 0);
        assert_eq!(unsafe { acl_add_flag_np(flags, 1 << 5) }, 0); // file inheritance
        assert_eq!(unsafe { acl_add_flag_np(flags, 1 << 6) }, 0); // directory inheritance
    }
    assert_eq!(
        unsafe { acl_set_fd_np(file.as_raw_fd(), acl.0, EXTENDED) },
        0
    );
}
