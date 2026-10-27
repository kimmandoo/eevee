//! Sole-main-loop ownership for a native child while terminal modes are suspended.
use std::io;
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
pub(crate) struct NativeSignals {
    interrupt: libc::sigaction,
    background: libc::sigaction,
}
#[cfg(unix)]
extern "C" fn record_interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::Release);
}
#[cfg(unix)]
impl NativeSignals {
    pub(crate) fn new() -> io::Result<Self> {
        INTERRUPTED.store(false, Ordering::Release);
        // SAFETY: signal handler uses only a lock-free atomic; restore on Drop.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = record_interrupt as *const () as usize;
            libc::sigemptyset(&mut action.sa_mask);
            let mut interrupt = std::mem::zeroed();
            let mut background = std::mem::zeroed();
            if libc::sigaction(libc::SIGINT, &action, &mut interrupt) != 0 {
                return Err(io::Error::last_os_error());
            }
            action.sa_sigaction = libc::SIG_IGN;
            if libc::sigaction(libc::SIGTTOU, &action, &mut background) != 0 {
                libc::sigaction(libc::SIGINT, &interrupt, std::ptr::null_mut());
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                interrupt,
                background,
            })
        }
    }
}
#[cfg(unix)]
impl Drop for NativeSignals {
    fn drop(&mut self) {
        unsafe {
            libc::sigaction(libc::SIGINT, &self.interrupt, std::ptr::null_mut());
            libc::sigaction(libc::SIGTTOU, &self.background, std::ptr::null_mut());
        }
    }
}
#[cfg(windows)]
pub(crate) struct NativeSignals;
#[cfg(windows)]
unsafe extern "system" fn console_interrupt(event: u32) -> i32 {
    if event == 0 || event == 1 {
        INTERRUPTED.store(true, Ordering::Release);
        1
    } else {
        0
    }
}
#[cfg(windows)]
impl NativeSignals {
    pub(crate) fn new() -> io::Result<Self> {
        INTERRUPTED.store(false, Ordering::Release);
        if unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(console_interrupt), 1)
        } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self)
        }
    }
}
#[cfg(windows)]
impl Drop for NativeSignals {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(console_interrupt), 0);
        }
    }
}

pub(crate) struct NativeChild {
    child: std::process::Child,
    #[cfg(unix)]
    foreground: libc::pid_t,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}
impl NativeChild {
    pub(crate) fn spawn(command: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
        }
        Ok(Self {
            child: command.spawn()?,
            #[cfg(unix)]
            foreground: unsafe { libc::tcgetpgrp(libc::STDIN_FILENO) },
            #[cfg(windows)]
            job: std::ptr::null_mut(),
        })
    }

    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub(crate) fn claim_terminal(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            if self.foreground < 0
                || unsafe { libc::tcsetpgrp(libc::STDIN_FILENO, self.child.id() as libc::pid_t) }
                    != 0
            {
                return Err(io::Error::last_os_error());
            }
            // A child that read before foreground handoff may have received SIGTTIN.
            unsafe {
                libc::kill(-(self.child.id() as libc::pid_t), libc::SIGCONT);
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
            use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
            use windows_sys::Win32::System::JobObjects::*;
            use windows_sys::Win32::System::Threading::{
                OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
            };
            self.job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if self.job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if unsafe {
                SetInformationJobObject(
                    self.job,
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )
            } == 0
                || unsafe { AssignProcessToJobObject(self.job, self.child.as_raw_handle()) } == 0
            {
                return Err(io::Error::last_os_error());
            }
            // std::process owns the process handle but not its primary-thread
            // handle. Enumerate the still-suspended new process's sole thread;
            // no child instruction can run before Job admission above.
            let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            let mut found = unsafe { Thread32First(snapshot, &mut entry) } != 0;
            let mut thread = std::ptr::null_mut();
            while found {
                if entry.th32OwnerProcessID == self.child.id() {
                    thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                    break;
                }
                entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
                found = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
            }
            unsafe {
                CloseHandle(snapshot);
            }
            if thread.is_null() {
                return Err(io::Error::other(
                    "Suspended native child thread unavailable",
                ));
            }
            let previous = unsafe { ResumeThread(thread) };
            unsafe {
                CloseHandle(thread);
            }
            if previous != 1 {
                return Err(io::Error::other("Native child thread admission failed"));
            }
            Ok(())
        }
    }
}
impl Drop for NativeChild {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL);
            if self.foreground >= 0 {
                libc::tcsetpgrp(libc::STDIN_FILENO, self.foreground);
            }
        }
        #[cfg(windows)]
        if !self.job.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.job);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
