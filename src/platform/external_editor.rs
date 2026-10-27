//! Reviewed direct-argv editor round trip. No shell expansion or task dispatch.
use super::native_process::{INTERRUPTED, NativeChild, NativeSignals};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

pub const LIMIT: usize = 4096;
pub const DISCLOSURE: &str = "Only this Request is exported as plaintext, and it may contain secrets. The external editor is NOT sandboxed: it inherits your environment (which may contain credentials) and may read files, create backups or use the network. Removing Eevee's temporary file does not erase editor-owned copies. Eevee does not copy conversation history, credential stores or selected context into this file, and sends no provider request. The editor must save in place; replaced files, links and changed ownership are refused.";
pub const CONFIG_HELP: &str = "VISUAL takes precedence over EDITOR. Supported values: one executable without whitespace, or a JSON array of executable and literal arguments (for example [\"code\",\"--wait\"]). The exact temporary filename is appended. No shell, interpolation, tilde expansion or placeholders are applied.";

pub struct Plan {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub path: PathBuf,
    executable_identity: crate::workspace::runner::ExecutableIdentity,
}

fn configuration(value: &str) -> Result<Vec<String>, String> {
    let args: Vec<String> = if value.trim_start().starts_with('[') {
        serde_json::from_str(value)
            .map_err(|_| "Editor configuration must be a JSON argv array".to_owned())?
    } else if !value.is_empty() && !value.chars().any(char::is_whitespace) {
        vec![value.to_owned()]
    } else {
        return Err(CONFIG_HELP.into());
    };
    if args.is_empty()
        || args.len() > 32
        || args[0].is_empty()
        || args.iter().map(String::len).sum::<usize>() > 8192
        || args.iter().any(|arg| arg.chars().any(char::is_control))
    {
        return Err(
            "Editor argv must contain 1–32 control-free arguments, at most 8192 UTF-8 bytes".into(),
        );
    }
    Ok(args)
}

impl Plan {
    pub fn prepare() -> Result<Self, String> {
        let value = env::var("VISUAL")
            .ok()
            .filter(|value| !value.is_empty())
            .or_else(|| env::var("EDITOR").ok().filter(|value| !value.is_empty()))
            .ok_or_else(|| format!("Set VISUAL or EDITOR. {CONFIG_HELP}"))?;
        Self::from_configuration(&value)
    }

    fn from_configuration(value: &str) -> Result<Self, String> {
        let mut arguments = configuration(value)?;
        let executable = crate::workspace::runner::resolve_executable(
            &arguments.remove(0),
            &env::current_dir().map_err(|error| error.to_string())?,
        )?;
        let executable_identity = crate::workspace::runner::executable_identity(&executable)
            .map_err(|error| error.to_string())?;
        let mut random = [0_u8; 16];
        getrandom::getrandom(&mut random).map_err(|error| error.to_string())?;
        let token = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let parent = fs::canonicalize(env::temp_dir()).map_err(|error| error.to_string())?;
        let path = parent
            .join(format!("ev-request-{token}"))
            .join("request.txt");
        Ok(Self {
            executable,
            arguments,
            path,
            executable_identity,
        })
    }

    pub fn review(&self) -> String {
        format!(
            "Executable: {:?}\nSHA-256: {}\nLiteral argv: {:?}\nFinal argument: {:?}\n\n{DISCLOSURE}\n\n{CONFIG_HELP}",
            self.executable, self.executable_identity.sha256, self.arguments, self.path
        )
    }

    pub fn run(&self, original: &str, cwd: &Path) -> Result<String, String> {
        validate(original)?;
        let executable_name = self
            .executable
            .to_str()
            .ok_or("Editor executable path is not Unicode")?;
        if crate::workspace::runner::resolve_executable(executable_name, cwd)? != self.executable
            || crate::workspace::runner::executable_identity(&self.executable)
                .map_err(|error| error.to_string())?
                != self.executable_identity
        {
            return Err("Editor executable changed since review; original Request kept".into());
        }
        let mut temporary = Temporary::create(&self.path)?;
        let result = (|| {
            temporary
                .file
                .write_all(original.as_bytes())
                .map_err(|error| error.to_string())?;
            temporary
                .file
                .sync_all()
                .map_err(|error| error.to_string())?;
            temporary.directory_unchanged()?;
            let _signals = NativeSignals::new().map_err(|error| error.to_string())?;
            let mut command = Command::new(&self.executable);
            command
                .args(&self.arguments)
                .arg(&self.path)
                .current_dir(cwd);
            let mut child = NativeChild::spawn(&mut command)
                .map_err(|error| format!("Cannot launch editor: {error}"))?;
            #[cfg(unix)]
            {
                use std::io::IsTerminal;
                if std::io::stdin().is_terminal() {
                    child.claim_terminal().map_err(|error| error.to_string())?;
                }
            }
            #[cfg(windows)]
            child.claim_terminal().map_err(|error| error.to_string())?;
            let status = loop {
                if INTERRUPTED.load(std::sync::atomic::Ordering::Acquire) {
                    return Err("Editor interrupted; original Request kept".into());
                }
                match child.try_wait().map_err(|error| error.to_string())? {
                    Some(status) => break status,
                    None => std::thread::sleep(std::time::Duration::from_millis(25)),
                }
            };
            // Return foreground ownership before temporary cleanup or fullscreen reentry.
            drop(child);
            if !status.success() {
                return Err(format!("Editor exited {status}; original Request kept"));
            }
            temporary.read()
        })();
        match (result, temporary.cleanup()) {
            (Ok(text), Ok(())) => Ok(text),
            (Err(error), Ok(())) => Err(error),
            (result, Err(cleanup)) => Err(format!(
                "{} Temporary cleanup FAILED: {cleanup}. Original Request kept; inspect {:?} manually. Editor-owned copies are not erased.",
                result.err().unwrap_or_else(|| "Editor returned".into()),
                self.path
            )),
        }
    }
}

pub fn validate(text: &str) -> Result<(), String> {
    if text.len() > LIMIT {
        return Err("Editor output exceeds the 4096-byte Request limit".into());
    }
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' && chars.peek() == Some(&'\n') {
            continue;
        }
        if ch.is_control() && !matches!(ch, '\n' | '\t')
            || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            return Err("Editor output contains unsafe controls (only LF, paired CRLF and tab are accepted)".into());
        }
    }
    Ok(())
}

struct Temporary {
    path: PathBuf,
    directory: File,
    directory_identity: String,
    file: File,
    file_identity: String,
}

fn open_nofollow(path: &Path, directory: bool, create: bool) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true).write(create).create_new(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(
            libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK
                | if directory { libc::O_DIRECTORY } else { 0 },
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .share_mode(3)
            .custom_flags(0x0020_0000 | if directory { 0x0200_0000 } else { 0 });
    }
    let file = options.open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err("Temporary path is not a regular owned file/directory".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Temporary reparse point refused".into());
        }
    }
    Ok(file)
}

fn identity(file: &File, private: bool) -> Result<String, String> {
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if private
            && (metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
                || metadata.is_file() && metadata.nlink() != 1)
        {
            return Err("Temporary ownership, permissions or link count changed".into());
        }
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: owned live handle and writable ABI-correct information.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if private && metadata.is_file() && info.nNumberOfLinks != 1 {
            return Err("Temporary hard link refused".into());
        }
        Ok(format!(
            "{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
}

fn private_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .map_err(|error| error.to_string())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::{
                Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SECURITY_ATTRIBUTES,
            },
            Storage::FileSystem::CreateDirectoryW,
        };
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
            return Err(std::io::Error::last_os_error().to_string());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        // SAFETY: descriptor and path live throughout the call.
        let result = unsafe { CreateDirectoryW(path.as_ptr(), &attributes) };
        let error = std::io::Error::last_os_error();
        // SAFETY: conversion allocated with LocalAlloc.
        unsafe {
            LocalFree(descriptor);
        }
        if result == 0 {
            Err(error.to_string())
        } else {
            Ok(())
        }
    }
}

fn open_member(directory: &File, path: &Path, create: bool) -> Result<File, String> {
    #[cfg(unix)]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        let flags = if create {
            libc::O_RDWR | libc::O_CREAT | libc::O_EXCL
        } else {
            libc::O_RDONLY
        };
        // SAFETY: directory is live, the fixed single component has no NUL, and
        // the new descriptor is adopted exactly once.
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c"request.txt".as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // SAFETY: successful openat returned a new owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        if !file
            .metadata()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("Temporary output is not a regular file".into());
        }
        let _ = path;
        Ok(file)
    }
    #[cfg(windows)]
    {
        let _ = directory;
        open_nofollow(path, false, create)
    }
}

impl Temporary {
    fn create(path: &Path) -> Result<Self, String> {
        let parent = path.parent().ok_or("Missing temporary directory")?;
        private_directory(parent)?;
        let result = (|| {
            let directory = open_nofollow(parent, true, false)?;
            let directory_identity = identity(&directory, true)?;
            let file = open_member(&directory, path, true)?;
            let file_identity = identity(&file, true)?;
            Ok(Self {
                path: path.to_path_buf(),
                directory,
                directory_identity,
                file,
                file_identity,
            })
        })();
        if result.is_err()
            && let Err(error) = fs::remove_dir(parent)
        {
            return Err(format!(
                "Temporary setup failed; cleanup FAILED at {parent:?}: {error}"
            ));
        }
        result
    }

    fn directory_unchanged(&self) -> Result<(), String> {
        let directory = open_nofollow(
            self.path.parent().ok_or("Missing temporary parent")?,
            true,
            false,
        )?;
        if identity(&directory, true)? != self.directory_identity
            || identity(&self.directory, true)? != self.directory_identity
        {
            return Err("Temporary directory identity changed".into());
        }
        Ok(())
    }

    fn read(&self) -> Result<String, String> {
        self.directory_unchanged()?;
        let mut file = open_member(&self.directory, &self.path, false)?;
        if identity(&file, true)? != self.file_identity
            || identity(&self.file, true)? != self.file_identity
        {
            return Err("Editor replaced the temporary file; original Request kept".into());
        }
        let before = file.metadata().map_err(|error| error.to_string())?;
        let mut bytes = Vec::with_capacity(LIMIT + 1);
        (&mut file)
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        let after = file.metadata().map_err(|error| error.to_string())?;
        self.directory_unchanged()?;
        if before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
            || identity(&file, true)? != self.file_identity
            || identity(&open_member(&self.directory, &self.path, false)?, true)?
                != self.file_identity
        {
            return Err("Editor output changed during import".into());
        }
        let text = String::from_utf8(bytes).map_err(|_| "Editor output is not UTF-8".to_owned())?;
        validate(&text)?;
        Ok(text)
    }

    fn cleanup(self) -> Result<(), String> {
        self.directory_unchanged()?;
        // Remove only this entry, never follow an editor-created link or recurse into backups.
        #[cfg(unix)]
        let removed = {
            use std::os::fd::AsRawFd;
            // SAFETY: anchored live directory and fixed single filename.
            if unsafe { libc::unlinkat(self.directory.as_raw_fd(), c"request.txt".as_ptr(), 0) }
                == 0
            {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        };
        #[cfg(windows)]
        let removed = fs::remove_file(&self.path);
        match removed {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        let parent = self
            .path
            .parent()
            .ok_or("Missing temporary parent")?
            .to_path_buf();
        drop(self);
        fs::remove_dir(parent).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_is_literal_not_shell() {
        assert_eq!(
            configuration(r#"["editor","$HOME","; touch /tmp/no"]"#).unwrap(),
            ["editor", "$HOME", "; touch /tmp/no"]
        );
        assert!(configuration("editor --wait").is_err());
        assert!(configuration("[]").is_err());
    }
    #[test]
    fn exact_text_validation() {
        assert!(validate("한글😀\r\nlast\n\n").is_ok());
        for text in ["bad\r", "bad\0", "\u{1b}[31m", "\u{202e}fake"] {
            assert!(validate(text).is_err());
        }
        assert!(validate(&"a".repeat(LIMIT + 1)).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn disposable_editors_preserve_or_refuse_exact_bytes_and_clean() {
        let text = "한글😀\r\nlast\n\n";
        let plan = Plan::from_configuration(
            r#"["/bin/sh","-c","printf '한글😀\\r\\nlast\\n\\n' > \"$1\"","editor"]"#,
        )
        .unwrap();
        assert_eq!(plan.run("original", Path::new("/tmp")).unwrap(), text);
        assert!(!plan.path.parent().unwrap().exists());
        for script in [
            "exit 7",
            "printf '\\377' > \"$1\"",
            "rm \"$1\"; ln -s /etc/passwd \"$1\"",
            "dd if=/dev/zero bs=4097 count=1 > \"$1\" 2>/dev/null",
            "mv \"$1\" \"$1.old\"; printf changed > \"$1\"",
            "ln \"$1\" \"$1.link\"",
        ] {
            let value = serde_json::to_string(&["/bin/sh", "-c", script, "editor"]).unwrap();
            let plan = Plan::from_configuration(&value).unwrap();
            assert!(plan.run("original", Path::new("/tmp")).is_err(), "{script}");
            // Unsafe/editor-owned copies are not silently recursively erased.
            if plan.path.parent().unwrap().exists() {
                fs::remove_dir_all(plan.path.parent().unwrap()).unwrap();
            }
        }
    }
    #[cfg(unix)]
    #[test]
    fn in_place_executable_change_after_review_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let reservation = Plan::from_configuration("/bin/sh").unwrap();
        let root = reservation.path.parent().unwrap();
        fs::create_dir(root).unwrap();
        let executable = root.join("editor");
        fs::write(&executable, b"#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let plan = Plan::from_configuration(executable.to_str().unwrap()).unwrap();
        fs::write(&executable, b"#!/bin/sh\nexit 7\n").unwrap();
        assert!(
            plan.run("original", Path::new("/tmp"))
                .unwrap_err()
                .contains("changed since review")
        );
        assert!(!plan.path.parent().unwrap().exists());
        fs::remove_dir_all(root).unwrap();
    }
}
