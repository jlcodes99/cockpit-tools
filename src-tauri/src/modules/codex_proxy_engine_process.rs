//! Keep each Windows Mihomo process tied to its owning tunnel and host.
use std::io;
use tokio::process::{Child, Command};

#[cfg(not(windows))]
pub(super) struct ProcessLifetime;

#[cfg(windows)]
pub(super) use windows_job::ProcessLifetime;

/// The caller must send the `-f -` configuration only after this returns. Stable
/// Rust/Tokio cannot set PROC_THREAD_ATTRIBUTE_JOB_LIST at process creation; an
/// engine still awaiting stdin cannot open its configured listeners in that gap.
/// Use a new private working directory as well: empty stdin must not fall back
/// to a config.yaml inherited from the host's directory. Closing the host also
/// closes stdin if it exits before job assignment.
pub(super) fn spawn(command: &mut Command) -> io::Result<(Child, ProcessLifetime)> {
    #[cfg(not(windows))]
    {
        command.spawn().map(|child| (child, ProcessLifetime))
    }
    #[cfg(windows)]
    {
        let lifetime = ProcessLifetime::new()?;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let mut child = command.spawn()?;
        if let Err(error) = lifetime.attach(&child) {
            // A short-lived version probe may exit before assignment. It cannot
            // leave a running process behind, and its output remains readable.
            if !matches!(child.try_wait(), Ok(Some(_))) {
                // Fail closed: never return a live engine without its job.
                let _ = child.start_kill();
                return Err(error);
            }
        }
        Ok((child, lifetime))
    }
}

#[cfg(windows)]
mod windows_job {
    use std::{
        io,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    use tokio::process::Child;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::HANDLE,
            System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
        },
    };

    // An unnamed job belongs only to this tunnel. OwnedHandle is Send + Sync,
    // and closes on drop; process termination closes it even without Rust Drop.
    pub(in super::super) struct ProcessLifetime(OwnedHandle);

    impl ProcessLifetime {
        pub(super) fn new() -> io::Result<Self> {
            // No SECURITY_ATTRIBUTES means the job handle is not inheritable.
            let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }?;
            // SAFETY: CreateJobObjectW returned a new, uniquely owned handle.
            let job = unsafe { OwnedHandle::from_raw_handle(job.0) };
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: limits has the matching layout and stays valid for the call.
            unsafe {
                SetInformationJobObject(
                    HANDLE(job.as_raw_handle()),
                    JobObjectExtendedLimitInformation,
                    (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )?;
            }
            Ok(Self(job))
        }

        pub(super) fn attach(&self, child: &Child) -> io::Result<()> {
            let handle = child.raw_handle().ok_or_else(|| {
                io::Error::new(io::ErrorKind::BrokenPipe, "Mihomo process already exited")
            })?;
            // SAFETY: both handles are valid for this call. Only this child is
            // assigned; the host and independently launched clients are not.
            unsafe { AssignProcessToJobObject(HANDLE(self.0.as_raw_handle()), HANDLE(handle))? };
            Ok(())
        }
    }
}

#[cfg(all(test, windows))]
#[path = "codex_proxy_engine_process_tests.rs"]
mod tests;
