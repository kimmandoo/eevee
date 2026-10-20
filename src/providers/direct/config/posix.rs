//! Handle-anchored access to the exact canonical target that passed the project guard.
use std::ffi::CString;
use std::fs::{self, File, Metadata, OpenOptions};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path};

#[cfg(target_os = "macos")]
mod macos;

const ERROR: &str = "User provider storage identity, permissions or ACL changed; refusing access.";

fn same_object(expected: &Metadata, opened: &Metadata) -> bool {
    expected.dev() == opened.dev()
        && expected.ino() == opened.ino()
        && expected.uid() == opened.uid()
        && expected.mode() == opened.mode()
}

/// Walk an already-canonical absolute directory path, never following a component.
/// Directory handles anchor later operations even if a path ancestor is renamed.
pub(super) fn open_directory(checked: &Path) -> Result<File, String> {
    if !checked.is_absolute() {
        return Err(ERROR.into());
    }
    let expected = fs::symlink_metadata(checked).map_err(|_| ERROR.to_string())?;
    if !expected.is_dir() {
        return Err(ERROR.into());
    }
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")
        .map_err(|_| ERROR.to_string())?;
    for component in checked.components() {
        let name = match component {
            Component::RootDir => continue,
            Component::Normal(name) => {
                CString::new(name.as_bytes()).map_err(|_| ERROR.to_string())?
            }
            _ => return Err(ERROR.into()),
        };
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return Err(ERROR.into());
        }
        directory = unsafe { File::from_raw_fd(descriptor) };
    }
    let opened = directory.metadata().map_err(|_| ERROR.to_string())?;
    if !same_object(&expected, &opened) {
        return Err(ERROR.into());
    }
    Ok(directory)
}

/// `checked` is the canonical non-project path; `expected` describes the original
/// regular leaf. A swapped original parent can never redirect this open into a project.
pub(super) fn open_user_file(
    checked: &Path,
    expected: &Metadata,
    credential: bool,
) -> Result<File, String> {
    if !checked.is_absolute() {
        return Err(ERROR.into());
    }
    let target = fs::symlink_metadata(checked).map_err(|_| ERROR.to_string())?;
    if !target.is_file() || !same_object(expected, &target) {
        return Err(ERROR.into());
    }
    let directory = open_directory(checked.parent().ok_or(ERROR)?)?;
    let name = CString::new(checked.file_name().ok_or(ERROR)?.as_bytes())
        .map_err(|_| ERROR.to_string())?;
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
        )
    };
    if descriptor < 0 {
        return Err(ERROR.into());
    }
    let file = unsafe { File::from_raw_fd(descriptor) };
    let opened = file.metadata().map_err(|_| ERROR.to_string())?;
    if !opened.is_file()
        || opened.len() > super::FILE_LIMIT
        || !same_object(expected, &opened)
        || opened.uid() != unsafe { libc::geteuid() }
        || opened.mode() & (if credential { 0o077 } else { 0o022 }) != 0
    {
        return Err(ERROR.into());
    }
    check_acl(&file, credential)?;
    Ok(file)
}

/// Private objects cannot have ACL allow grants; nonsecret config/HOME may have
/// read/search grants, but no write, deletion, security or ownership grants.
pub(super) fn check_acl(file: &File, private: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::check(file, private)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (file, private);
        Ok(())
    }
}

/// Called only on newly-created app-owned objects, before any credential bytes.
pub(super) fn clear_new_acl(file: &File) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::clear_new(file)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = file;
        Ok(())
    }
}

#[cfg(all(test, target_os = "macos"))]
pub(super) fn grant_acl_for_test(file: &File, permissions: u64, inherit: bool) {
    macos::grant_for_test(file, permissions, inherit);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::fs::{PermissionsExt, symlink};
    struct Temporary(std::path::PathBuf);
    impl Temporary {
        fn new() -> Self {
            let mut random = [0; 16];
            getrandom::getrandom(&mut random).unwrap();
            let path = std::env::temp_dir().join(format!(
                "eevee-checked-config-{:x}",
                u128::from_le_bytes(random)
            ));
            fs::create_dir(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            Self(path.canonicalize().unwrap())
        }
        fn file(&self, directory: &str, text: &str) -> std::path::PathBuf {
            let parent = self.0.join(directory);
            fs::create_dir(&parent).unwrap();
            let path = parent.join("providers.toml");
            fs::write(&path, text).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            path
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn swapped_original_parent_cannot_redirect_the_checked_target() {
        let temp = Temporary::new();
        let safe = temp.file("outside", "safe-user-config");
        let project = temp.file("project", "project-must-never-be-read");
        let alias = temp.0.join("alias");
        symlink(safe.parent().unwrap(), &alias).unwrap();
        let requested = alias.join("providers.toml");
        let checked = requested.canonicalize().unwrap();
        let expected = fs::symlink_metadata(&requested).unwrap();
        fs::remove_file(&alias).unwrap();
        symlink(project.parent().unwrap(), &alias).unwrap();
        let mut file = open_user_file(&checked, &expected, false).unwrap();
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        assert_eq!(text, "safe-user-config");
        let changed_leaf = fs::symlink_metadata(&requested).unwrap();
        assert!(open_user_file(&checked, &changed_leaf, false).is_err());
    }

    #[test]
    fn swapped_checked_parent_symlink_and_leaf_replacement_are_refused() {
        let temp = Temporary::new();
        let safe = temp.file("outside", "safe");
        let project = temp.file("project", "project");
        let expected = fs::symlink_metadata(&safe).unwrap();
        fs::rename(safe.parent().unwrap(), temp.0.join("previous")).unwrap();
        symlink(project.parent().unwrap(), temp.0.join("outside")).unwrap();
        assert!(open_user_file(&safe, &expected, false).is_err());
        assert!(open_directory(&temp.0.join("outside")).is_err());
        fs::remove_file(temp.0.join("outside")).unwrap();
        fs::rename(temp.0.join("previous"), temp.0.join("outside")).unwrap();
        fs::remove_file(&safe).unwrap();
        fs::write(&safe, "replacement").unwrap();
        assert!(open_user_file(&safe, &expected, false).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_provider_acl_grants_cannot_hide_behind_private_mode_bits() {
        let temp = Temporary::new();
        let path = temp.file("provider", "isolated-fake-credential");
        let file = File::open(&path).unwrap();
        grant_acl_for_test(&file, 1 << 1, false);
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert!(super::super::read_user_file(&path, true, None).is_err());
        assert!(check_acl(&file, true).is_err()); // provider ACL was not rewritten
        assert!(super::super::read_user_file(&path, false, None).is_ok());
        grant_acl_for_test(&file, 1 << 2, false);
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert!(super::super::read_user_file(&path, false, None).is_err());
        assert!(check_acl(&file, false).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_new_empty_owned_file_can_strip_inherited_acl_before_secret_write() {
        let temp = Temporary::new();
        let parent = open_directory(&temp.0).unwrap();
        grant_acl_for_test(&parent, (1 << 1) | (1 << 3), true);
        let path = temp.0.join("new-owned-empty");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        assert!(check_acl(&file, true).is_err());
        clear_new_acl(&file).unwrap();
        assert!(check_acl(&file, true).is_ok());
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert!(check_acl(&parent, true).is_err()); // the user-owned parent is untouched
    }
}
