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
    termination_sent: bool,
    #[cfg(test)]
    forced_verification_failures: usize,
    #[cfg(test)]
    forced_root_query_failures: usize,
}

pub(crate) enum OwnedProcessTreeSpawn {
    Ready(OwnedProcessTree),
    CleanupRequired {
        tree: OwnedProcessTree,
        error: io::Error,
    },
}

impl OwnedProcessTree {
    /// Spawns a process and retains any suspended child whose ownership setup needs cleanup.
    pub(crate) fn spawn(command: &mut Command) -> io::Result<OwnedProcessTreeSpawn> {
        command.kill_on_drop(true);
        let (child, owner, setup_error) = platform::spawn_owned(command)?;
        let tree = Self {
            child,
            owner,
            status: None,
            termination_sent: false,
            #[cfg(test)]
            forced_verification_failures: 0,
            #[cfg(test)]
            forced_root_query_failures: 0,
        };
        Ok(match setup_error {
            Some(error) => OwnedProcessTreeSpawn::CleanupRequired { tree, error },
            None => OwnedProcessTreeSpawn::Ready(tree),
        })
    }

    pub(crate) fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    pub(crate) fn try_wait_root(&mut self) -> io::Result<Option<ExitStatus>> {
        #[cfg(test)]
        if self.forced_root_query_failures > 0 {
            self.forced_root_query_failures -= 1;
            return Err(io::Error::other("injected provider-root query failure"));
        }
        if self.status.is_none() {
            self.status = self.child.try_wait()?;
        }
        Ok(self.status)
    }

    #[cfg(test)]
    pub(crate) fn force_verification_failures(&mut self, failures: usize) {
        self.forced_verification_failures = failures;
    }

    #[cfg(test)]
    pub(crate) fn force_root_query_failures(&mut self, failures: usize) {
        self.forced_root_query_failures = failures;
    }

    /// Terminates and verifies the complete owned scope. Repeated calls are harmless.
    pub(crate) async fn terminate_and_wait(&mut self) -> io::Result<ExitStatus> {
        if !self.termination_sent {
            // Give a provider that already emitted its terminal frame a short opportunity
            // to preserve its real exit status before escalating the whole scope.
            tokio::time::sleep(GRACE_PERIOD).await;
            platform::terminate(&self.owner, &mut self.child)?;
            // Never signal this numeric process-group identity again after waiting can
            // reap the root and permit its PID to be reused.
            self.termination_sent = true;
        }

        let deadline = Instant::now() + TERMINATION_TIMEOUT;
        if self.status.is_none() {
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
        }

        #[cfg(test)]
        if self.forced_verification_failures > 0 {
            self.forced_verification_failures -= 1;
            return Err(io::Error::other(
                "injected provider-scope verification failure",
            ));
        }

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
        self.status
            .ok_or_else(|| io::Error::other("owned process tree lost its root status"))
    }
}

impl Drop for OwnedProcessTree {
    fn drop(&mut self) {
        if !self.termination_sent {
            // Drop cannot truthfully wait from arbitrary async-runtime contexts. The
            // OS-owned scope is synchronously terminated here, and no success is emitted.
            platform::terminate_on_drop(&self.owner, &mut self.child);
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::{io, time::Duration};

    use tokio::process::{Child, Command};

    pub(super) struct Owner;

    pub(super) fn spawn_owned(
        _command: &mut Command,
    ) -> io::Result<(Child, Owner, Option<io::Error>)> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "external CLI providers are disabled on Unix: setsid/setpgid descendants can escape process-group ownership",
        ))
    }

    pub(super) fn terminate(_owner: &Owner, _child: &mut Child) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Unix external-provider ownership is unavailable",
        ))
    }

    pub(super) fn terminate_on_drop(_owner: &Owner, _child: &mut Child) {}

    pub(super) async fn verify_empty(
        _owner: &Owner,
        _passes: usize,
        _interval: Duration,
    ) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Unix external-provider ownership is unavailable",
        ))
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
        root_assigned: bool,
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

    pub(super) fn spawn_owned(
        command: &mut Command,
    ) -> io::Result<(Child, Owner, Option<io::Error>)> {
        let mut owner = create_kill_on_close_job()?;
        command.as_std_mut().creation_flags(CREATE_SUSPENDED);
        let child = command.spawn()?;
        let Some(process) = child.raw_handle().map(|handle| handle as HANDLE) else {
            return Ok((
                child,
                owner,
                Some(io::Error::other("suspended provider has no process handle")),
            ));
        };

        if unsafe { AssignProcessToJobObject(owner.handle(), process) } == 0 {
            let error = io::Error::last_os_error();
            return Ok((
                child,
                owner,
                Some(io::Error::new(
                    error.kind(),
                    format!("assign suspended provider to Job Object: {error}"),
                )),
            ));
        }
        owner.root_assigned = true;
        if let Err(error) = resume_primary_thread(child.id()) {
            return Ok((child, owner, Some(error)));
        }
        Ok((child, owner, None))
    }

    pub(super) fn terminate(owner: &Owner, child: &mut Child) -> io::Result<()> {
        if !owner.root_assigned {
            return match child.start_kill() {
                Ok(()) => Ok(()),
                Err(error) => match child.try_wait()? {
                    Some(_) => Ok(()),
                    None => Err(error),
                },
            };
        }
        if unsafe { TerminateJobObject(owner.handle(), 1) } == 0 {
            let error = io::Error::last_os_error();
            if active_processes(owner)? != 0 {
                return Err(error);
            }
        }
        Ok(())
    }

    pub(super) fn terminate_on_drop(owner: &Owner, child: &mut Child) {
        if owner.root_assigned {
            unsafe {
                TerminateJobObject(owner.handle(), 1);
            }
        } else {
            let _ = child.start_kill();
        }
    }

    pub(super) async fn verify_empty(
        owner: &Owner,
        passes: usize,
        interval: Duration,
    ) -> io::Result<()> {
        if !owner.root_assigned {
            return Ok(());
        }
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
        let owner = Owner {
            job: job as isize,
            root_assigned: false,
        };
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
}

#[cfg(all(test, windows))]
pub(crate) fn process_is_alive(pid: u32) -> bool {
    platform_test::process_is_alive(pid)
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

#[cfg(all(test, windows))]
mod tests {
    use std::{
        path::{Path, PathBuf},
        process::Stdio,
        time::Duration,
    };

    use serde::Deserialize;
    use tokio::process::{Child, Command};

    use super::{OwnedProcessTree, OwnedProcessTreeSpawn, process_is_alive};

    #[derive(Debug, Deserialize)]
    struct FixturePids {
        parent: u32,
        grandchild: u32,
    }

    fn fixture_script() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fake-external-agent.mjs")
    }

    fn fixture_command(pid_file: &Path, flags: &[&str]) -> Command {
        let mut command = Command::new("node");
        command
            .arg(fixture_script())
            .arg("--fixture-tree-parent")
            .arg(pid_file)
            .args(flags)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn spawn_ready(command: &mut Command) -> OwnedProcessTree {
        match OwnedProcessTree::spawn(command).expect("spawn owned fixture") {
            OwnedProcessTreeSpawn::Ready(tree) => tree,
            OwnedProcessTreeSpawn::CleanupRequired { error, .. } => {
                panic!("fixture ownership setup failed: {error}")
            }
        }
    }

    async fn wait_for_fixture_pids(pid_file: &Path) -> FixturePids {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(bytes) = tokio::fs::read(pid_file).await
                    && let Ok(pids) = serde_json::from_slice(&bytes)
                {
                    return pids;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fixture PID file timeout")
    }

    async fn wait_until_dead(pid: u32) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while process_is_alive(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fixture process remained alive");
    }

    async fn spawn_unrelated() -> Child {
        let mut command = Command::new("node");
        command
            .arg(fixture_script())
            .arg("--fixture-tree-grandchild")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command.spawn().expect("spawn unrelated fixture")
    }

    async fn stop_unrelated(mut child: Child) {
        child.start_kill().expect("kill unrelated fixture");
        child.wait().await.expect("wait for unrelated fixture");
    }

    #[tokio::test]
    async fn stubborn_tree_is_removed_idempotently_without_killing_unrelated_process() {
        let test_dir = std::env::temp_dir()
            .join("crony-process-tree-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&test_dir).expect("create fixture directory");
        let pid_file = test_dir.join("tree-pids.json");
        let mut tree = spawn_ready(&mut fixture_command(&pid_file, &["--stubborn"]));
        let pids = wait_for_fixture_pids(&pid_file).await;
        let unrelated = spawn_unrelated().await;
        let unrelated_pid = unrelated.id().expect("unrelated fixture PID");

        assert!(process_is_alive(pids.parent));
        assert!(process_is_alive(pids.grandchild));
        assert!(process_is_alive(unrelated_pid));

        let first_status = tree
            .terminate_and_wait()
            .await
            .expect("terminate stubborn process tree");
        let repeated_status = tree
            .terminate_and_wait()
            .await
            .expect("repeat process-tree termination");

        assert_eq!(first_status, repeated_status);
        wait_until_dead(pids.parent).await;
        wait_until_dead(pids.grandchild).await;
        assert!(
            process_is_alive(unrelated_pid),
            "termination escaped the owned process tree"
        );

        stop_unrelated(unrelated).await;
        let _ = std::fs::remove_dir_all(test_dir);
    }

    #[tokio::test]
    async fn termination_handles_a_descendant_that_already_exited() {
        let test_dir = std::env::temp_dir()
            .join("crony-process-tree-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&test_dir).expect("create fixture directory");
        let pid_file = test_dir.join("tree-pids.json");
        let mut tree = spawn_ready(&mut fixture_command(&pid_file, &["--exit-soon"]));
        let pids = wait_for_fixture_pids(&pid_file).await;

        wait_until_dead(pids.grandchild).await;
        assert!(process_is_alive(pids.parent));
        tree.terminate_and_wait()
            .await
            .expect("terminate tree after descendant exit");
        tree.terminate_and_wait()
            .await
            .expect("repeat termination after descendant exit");

        wait_until_dead(pids.parent).await;
        // The earlier wait already proves this exact child exited. Do not reopen the
        // numeric PID after unrelated concurrent tests may have caused Windows to
        // reuse it for a different process.
        let _ = std::fs::remove_dir_all(test_dir);
    }

    #[tokio::test]
    async fn verification_failure_keeps_the_owned_scope_for_retry() {
        let test_dir = std::env::temp_dir()
            .join("crony-process-tree-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&test_dir).expect("create fixture directory");
        let pid_file = test_dir.join("tree-pids.json");
        let mut tree = spawn_ready(&mut fixture_command(&pid_file, &[]));
        let pids = wait_for_fixture_pids(&pid_file).await;
        tree.force_verification_failures(1);

        let error = tree
            .terminate_and_wait()
            .await
            .expect_err("injected verification must fail closed");
        assert!(error.to_string().contains("injected"));
        tree.terminate_and_wait()
            .await
            .expect("retry verifies the same owned scope");

        wait_until_dead(pids.parent).await;
        wait_until_dead(pids.grandchild).await;
        let _ = std::fs::remove_dir_all(test_dir);
    }

    #[tokio::test]
    async fn root_query_failure_keeps_the_owned_scope_for_verified_cleanup() {
        let test_dir = std::env::temp_dir()
            .join("crony-process-tree-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&test_dir).expect("create fixture directory");
        let pid_file = test_dir.join("tree-pids.json");
        let mut tree = spawn_ready(&mut fixture_command(&pid_file, &["--stubborn"]));
        let pids = wait_for_fixture_pids(&pid_file).await;
        tree.force_root_query_failures(1);

        let error = tree
            .try_wait_root()
            .expect_err("injected root query must fail closed");
        assert!(error.to_string().contains("injected"));
        tree.terminate_and_wait()
            .await
            .expect("query failure must retain the same owned scope");

        wait_until_dead(pids.parent).await;
        wait_until_dead(pids.grandchild).await;
        let _ = std::fs::remove_dir_all(test_dir);
    }
}
