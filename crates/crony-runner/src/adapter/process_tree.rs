use std::{
    io,
    process::ExitStatus,
    time::{Duration, Instant},
};

use tokio::process::{Child, Command};

const GRACE_PERIOD: Duration = Duration::from_millis(350);
const TERMINATION_TIMEOUT: Duration = Duration::from_secs(5);
const VERIFY_INTERVAL: Duration = Duration::from_millis(20);
const VERIFY_PASSES: usize = 3;

pub(crate) struct OwnedProcessTree {
    child: Child,
    owner: platform::Owner,
    status: Option<ExitStatus>,
    terminated: bool,
}

impl OwnedProcessTree {
    /// Spawns a process only after arranging an inheritable OS ownership boundary.
    pub(crate) fn spawn(command: &mut Command) -> io::Result<Self> {
        command.kill_on_drop(true);
        let (child, owner) = platform::spawn_owned(command)?;
        Ok(Self {
            child,
            owner,
            status: None,
            terminated: false,
        })
    }

    pub(crate) fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    /// Terminates and verifies the complete owned scope. Repeated calls are harmless.
    pub(crate) async fn terminate_and_wait(&mut self) -> io::Result<ExitStatus> {
        if self.terminated {
            platform::verify_empty(&self.owner, VERIFY_PASSES, VERIFY_INTERVAL).await?;
            return self
                .status
                .ok_or_else(|| io::Error::other("owned process tree lost its root status"));
        }

        // Give a provider that already emitted its terminal frame a short opportunity
        // to preserve its real exit status before escalating the whole scope.
        tokio::time::sleep(GRACE_PERIOD).await;
        platform::terminate(&self.owner)?;

        let deadline = Instant::now() + TERMINATION_TIMEOUT;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let status = tokio::time::timeout(remaining, self.child.wait())
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "provider root did not exit before the process-tree deadline",
                )
            })??;
        self.status = Some(status);

        let remaining = deadline.saturating_duration_since(Instant::now());
        tokio::time::timeout(
            remaining,
            platform::verify_empty(&self.owner, VERIFY_PASSES, VERIFY_INTERVAL),
        )
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "provider descendants survived the process-tree deadline",
            )
        })??;
        self.terminated = true;
        Ok(status)
    }
}

impl Drop for OwnedProcessTree {
    fn drop(&mut self) {
        if !self.terminated {
            // Drop cannot truthfully wait from arbitrary async-runtime contexts. The
            // OS-owned scope is synchronously terminated here, and no success is emitted.
            platform::terminate_on_drop(&self.owner);
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::{io, os::unix::process::CommandExt, time::Duration};

    use tokio::process::{Child, Command};

    pub(super) struct Owner {
        pgid: libc::pid_t,
    }

    pub(super) fn spawn_owned(command: &mut Command) -> io::Result<(Child, Owner)> {
        // SAFETY: setsid is async-signal-safe and touches no memory shared with the
        // parent. It executes in the child after fork and before provider code.
        unsafe {
            command.as_std_mut().pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let child = command.spawn()?;
        let pgid = child
            .id()
            .and_then(|pid| libc::pid_t::try_from(pid).ok())
            .ok_or_else(|| io::Error::other("spawned provider has no valid process-group ID"))?;
        Ok((child, Owner { pgid }))
    }

    pub(super) fn terminate(owner: &Owner) -> io::Result<()> {
        signal_group(owner.pgid, libc::SIGTERM)?;
        std::thread::sleep(super::GRACE_PERIOD);
        signal_group(owner.pgid, libc::SIGKILL)
    }

    pub(super) fn terminate_on_drop(owner: &Owner) {
        let _ = signal_group(owner.pgid, libc::SIGKILL);
    }

    pub(super) async fn verify_empty(
        owner: &Owner,
        passes: usize,
        interval: Duration,
    ) -> io::Result<()> {
        for _ in 0..passes {
            // Signal zero observes the scoped process group without affecting it. No
            // destructive signal is sent after the root has been reaped, so PID/PGID
            // reuse can only fail verification, never target an unrelated process.
            let result = unsafe { libc::kill(-owner.pgid, 0) };
            if result == 0 {
                return Err(io::Error::other(
                    "provider process group still contains descendants",
                ));
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
            tokio::time::sleep(interval).await;
        }
        Ok(())
    }

    fn signal_group(pgid: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
        let result = unsafe { libc::kill(-pgid, signal) };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::{
        io,
        mem::{size_of, zeroed},
        os::windows::process::CommandExt,
        time::Duration,
    };

    use tokio::process::{Child, Command};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First,
                Thread32Next,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JobObjectBasicAccountingInformation, JobObjectExtendedLimitInformation,
                QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
            },
            Threading::{CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
        },
    };

    pub(super) struct Owner {
        job: isize,
    }

    impl Owner {
        fn handle(&self) -> HANDLE {
            self.job as HANDLE
        }
    }

    impl Drop for Owner {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle());
            }
        }
    }

    pub(super) fn spawn_owned(command: &mut Command) -> io::Result<(Child, Owner)> {
        let owner = create_kill_on_close_job()?;
        command.as_std_mut().creation_flags(CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("suspended provider has no process handle"))?
            as HANDLE;

        if unsafe { AssignProcessToJobObject(owner.handle(), process) } == 0 {
            let error = io::Error::last_os_error();
            fail_suspended_spawn(&mut child, &owner);
            return Err(io::Error::new(
                error.kind(),
                format!("assign suspended provider to Job Object: {error}"),
            ));
        }
        if let Err(error) = resume_primary_thread(child.id()) {
            fail_suspended_spawn(&mut child, &owner);
            return Err(error);
        }
        Ok((child, owner))
    }

    pub(super) fn terminate(owner: &Owner) -> io::Result<()> {
        if unsafe { TerminateJobObject(owner.handle(), 1) } == 0 {
            let error = io::Error::last_os_error();
            if active_processes(owner)? != 0 {
                return Err(error);
            }
        }
        Ok(())
    }

    pub(super) fn terminate_on_drop(owner: &Owner) {
        unsafe {
            TerminateJobObject(owner.handle(), 1);
        }
    }

    pub(super) async fn verify_empty(
        owner: &Owner,
        passes: usize,
        interval: Duration,
    ) -> io::Result<()> {
        for _ in 0..passes {
            if active_processes(owner)? != 0 {
                return Err(io::Error::other(
                    "provider Job Object still contains active processes",
                ));
            }
            tokio::time::sleep(interval).await;
        }
        Ok(())
    }

    fn create_kill_on_close_job() -> io::Result<Owner> {
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let owner = Owner { job: job as isize };
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let result = unsafe {
            SetInformationJobObject(
                owner.handle(),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(owner)
    }

    fn resume_primary_thread(pid: Option<u32>) -> io::Result<()> {
        let pid = pid.ok_or_else(|| io::Error::other("suspended provider has no process ID"))?;
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let result = (|| {
            let mut entry: THREADENTRY32 = unsafe { zeroed() };
            entry.dwSize = size_of::<THREADENTRY32>() as u32;
            let mut found = unsafe { Thread32First(snapshot, &mut entry) } != 0;
            while found {
                if entry.th32OwnerProcessID == pid {
                    let thread =
                        unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                    if thread.is_null() {
                        return Err(io::Error::last_os_error());
                    }
                    let resumed = unsafe { ResumeThread(thread) };
                    unsafe {
                        CloseHandle(thread);
                    }
                    if resumed == u32::MAX {
                        return Err(io::Error::last_os_error());
                    }
                    return Ok(());
                }
                found = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
            }
            Err(io::Error::other(
                "suspended provider primary thread was not found",
            ))
        })();
        unsafe {
            CloseHandle(snapshot);
        }
        result
    }

    fn active_processes(owner: &Owner) -> io::Result<u32> {
        let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { zeroed() };
        let result = unsafe {
            QueryInformationJobObject(
                owner.handle(),
                JobObjectBasicAccountingInformation,
                &mut accounting as *mut _ as *mut _,
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            )
        };
        if result == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(accounting.ActiveProcesses)
        }
    }

    fn fail_suspended_spawn(child: &mut Child, owner: &Owner) {
        unsafe {
            TerminateJobObject(owner.handle(), 1);
        }
        let _ = child.start_kill();
    }
}

#[cfg(test)]
pub(crate) fn process_is_alive(pid: u32) -> bool {
    platform_test::process_is_alive(pid)
}

#[cfg(all(test, unix))]
mod platform_test {
    pub(crate) fn process_is_alive(pid: u32) -> bool {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        let result = unsafe { libc::kill(pid, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(all(test, windows))]
mod platform_test {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };

    pub(crate) fn process_is_alive(pid: u32) -> bool {
        let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        if process.is_null() {
            return false;
        }
        let alive = unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT;
        unsafe {
            CloseHandle(process);
        }
        alive
    }
}
