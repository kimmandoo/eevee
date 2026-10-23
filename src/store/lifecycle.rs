//! Read-only admission for a retained executable with the identical format identity.
use super::*;

const SNAPSHOT_FILE: &str = "ev-run-lifecycle-snapshot.output";

fn copy_snapshot(source: &Path, directory: &SpoolDirectory) -> Result<[u8; 32], String> {
    let destination = directory.create(SNAPSHOT_FILE)?;
    recovery_digest_into(source, Some(destination))
}

struct Scratch(PathBuf);
impl Scratch {
    fn create() -> Result<Self, String> {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).map_err(|error| error.to_string())?;
        let path = fs::canonicalize(std::env::temp_dir())
            .map_err(|error| error.to_string())?
            .join(format!(
                ".ev-lifecycle-{:032x}",
                u128::from_le_bytes(random)
            ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path).map_err(|error| error.to_string())?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Inspect every schema and strict serialized row with this build's real reader.
/// Never open the original with SQLite: even a read-only WAL connection can
/// create sidecars. Inspect a bounded private byte-copy outside the data folder.
pub fn inspect_lifecycle_data(data_dir: &Path) -> Result<(), String> {
    let path = data_dir.join(DATABASE);
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Cannot inspect lifecycle data: {error}")),
        Ok(_) => {}
    }
    // This app uses rollback journals. Do not guess at a multi-file WAL snapshot
    // or recover an interrupted writer as part of downgrade discovery.
    let inspect_sidecars = || -> Result<(), String> {
        for suffix in ["-journal", "-wal", "-shm"] {
            let sidecar = data_dir.join(format!("{DATABASE}{suffix}"));
            if regular_target(&sidecar, true)?
                && (suffix != "-journal"
                    || read_regular(&sidecar)?
                        .metadata()
                        .map_err(|error| error.to_string())?
                        .len()
                        != 0)
            {
                return Err("Rollback compatibility requires a settled rollback-journal store; WAL/SHM or an outstanding journal needs ordinary storage recovery first".into());
            }
        }
        Ok(())
    };
    inspect_sidecars()?;
    let temporary = fs::canonicalize(std::env::temp_dir()).map_err(|error| error.to_string())?;
    let data_root = fs::canonicalize(data_dir).map_err(|error| error.to_string())?;
    if temporary.starts_with(&data_root) {
        return Err(
            "Lifecycle inspection requires a temporary directory outside protected task data"
                .into(),
        );
    }
    let scratch = Scratch::create()?;
    // Reuse the anchored spool primitive: files are mode 0600 on Unix and
    // protected owner-rights-only DACL on Windows, regardless of TEMP's ACL.
    let directory = SpoolDirectory::open(&scratch.0)?;
    let digest = copy_snapshot(&path, &directory)?;
    let copy = scratch.0.join(SNAPSHOT_FILE);
    {
        let mut connection = open_readonly(&copy)?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        inspect(&transaction)?;
    }
    inspect_sidecars()?;
    if digest != recovery_digest(&path, None)? {
        return Err(
            "Data changed during rollback compatibility inspection; prepare a new review".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let directory = Scratch::create().unwrap();
            let path = directory.0.clone();
            std::mem::forget(directory);
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn actual_strict_preference_incompatibility_refuses_without_touching_source_bytes() {
        let fixture = Fixture::new();
        let path = fixture.0.join(DATABASE);
        let mut connection = Connection::open(&path).unwrap();
        initialize(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO provider_preferences(id,record) VALUES(1,?1)",
                [r#"{"provider":"Codex","selections":{},"text_density":"future_density"}"#],
            )
            .unwrap();
        drop(connection);
        let before = fs::read(&path).unwrap();
        let names_before: Vec<_> = fs::read_dir(&fixture.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(inspect_lifecycle_data(&fixture.0).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let names_after: Vec<_> = fs::read_dir(&fixture.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names_before, names_after);
    }

    #[test]
    fn compatible_current_strict_data_is_inspected_without_creating_source_sidecars() {
        let fixture = Fixture::new();
        let path = fixture.0.join(DATABASE);
        let mut connection = Connection::open(&path).unwrap();
        initialize(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO provider_preferences(id,record) VALUES(1,?1)",
                [r#"{"provider":"Codex","selections":{},"text_density":"comfortable"}"#],
            )
            .unwrap();
        drop(connection);
        let before = fs::read(&path).unwrap();
        inspect_lifecycle_data(&fixture.0).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
    }
}

#[cfg(test)]
mod snapshot_privacy_tests {
    use super::*;

    #[test]
    fn compatibility_snapshot_copies_exact_bytes_with_private_file_permissions() {
        let scratch = Scratch::create().unwrap();
        let source = scratch.0.join(DATABASE);
        let bytes = b"private lifecycle compatibility source";
        fs::write(&source, bytes).unwrap();
        let directory = SpoolDirectory::open(&scratch.0).unwrap();
        assert_eq!(
            copy_snapshot(&source, &directory).unwrap(),
            recovery_digest(&source, None).unwrap()
        );
        let copy = scratch.0.join(SNAPSHOT_FILE);
        assert_eq!(fs::read(&copy).unwrap(), bytes);
        assert_eq!(fs::read(&source).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&copy).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        #[cfg(windows)]
        assert_owner_only_dacl(&read_regular(&copy).unwrap());
    }

    #[cfg(windows)]
    fn assert_owner_only_dacl(file: &File) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
        use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
        use windows_sys::Win32::Security::{
            ACE_HEADER, DACL_SECURITY_INFORMATION, GetAce, GetSecurityDescriptorControl,
            INHERITED_ACE, IsValidAcl, SE_DACL_PROTECTED,
        };
        struct Descriptor(*mut std::ffi::c_void);
        impl Drop for Descriptor {
            fn drop(&mut self) {
                // SAFETY: GetSecurityInfo allocated this descriptor with LocalAlloc.
                unsafe {
                    LocalFree(self.0);
                }
            }
        }
        let mut acl = std::ptr::null_mut();
        let mut raw = std::ptr::null_mut();
        // SAFETY: file owns a live handle and all output pointers are valid.
        assert_eq!(
            unsafe {
                GetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut acl,
                    std::ptr::null_mut(),
                    &mut raw,
                )
            },
            ERROR_SUCCESS
        );
        let descriptor = Descriptor(raw);
        assert!(!acl.is_null());
        // SAFETY: the descriptor and ACL returned above remain owned and live.
        assert_ne!(unsafe { IsValidAcl(acl) }, 0);
        let mut control = 0u16;
        let mut revision = 0u32;
        assert_ne!(
            unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) },
            0
        );
        assert_ne!(control & SE_DACL_PROTECTED, 0);
        assert_eq!(unsafe { (*acl).AceCount }, 1);
        let mut ace = std::ptr::null_mut();
        assert_ne!(unsafe { GetAce(acl, 0, &mut ace) }, 0);
        assert!(!ace.is_null());
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };
        assert_eq!(header.AceType, 0); // Ordinary ACCESS_ALLOWED_ACE only.
        assert_eq!(u32::from(header.AceFlags) & INHERITED_ACE, 0);
        assert!(header.AceSize >= 20);
        // ACCESS_ALLOWED_ACE has an eight-byte header/mask prefix. The sole
        // variable SID is S-1-3-4 (OWNER RIGHTS), never Everyone or inherited.
        assert_eq!(
            unsafe { std::slice::from_raw_parts(ace.cast::<u8>().add(8), 12) },
            &[1, 1, 0, 0, 0, 0, 0, 3, 4, 0, 0, 0]
        );
    }
}
