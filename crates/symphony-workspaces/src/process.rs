//! Lifecycle helpers for background subprocesses (agents, git, validation commands).
//!
//! Two production problems are handled here:
//!
//! - **Console windows.** The release desktop binary uses the Windows GUI subsystem, so every
//!   console program it starts (`git`, `cmd.exe`, `powershell.exe`, agent CLIs) would open a visible
//!   console window. [`hide_console_window`] sets `CREATE_NO_WINDOW`.
//! - **Process trees.** Agents are often started through a wrapper (`cmd.exe /C`, `bash -lc`, npm
//!   shims) and spawn their own tools. Killing only the direct child leaves the rest running and
//!   still editing the workspace. [`ProcessTree`] owns the whole tree: a Job Object with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` on Windows (the tree also dies when the host crashes,
//!   because the kernel closes the job handle), a dedicated process group on Unix.

use tokio::process::{Child, Command};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Never show a console window for this command (Windows); no-op elsewhere.
pub fn hide_console_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Prepare `cmd` so that [`ProcessTree::attach`] can later terminate everything it starts: no
/// console window, and on Unix its own process group (so a signal reaches the whole tree and the
/// terminal's Ctrl+C is handled by the host's graceful shutdown instead). The child is also
/// killed when its handle is dropped.
pub fn prepare_tree_command(cmd: &mut Command) -> &mut Command {
    hide_console_window(cmd);
    #[cfg(unix)]
    cmd.process_group(0);
    cmd.kill_on_drop(true)
}

/// Owns every process started by one child (see the module docs). Dropping it terminates the
/// tree, so a cancelled or timed-out agent turn cannot leave processes behind.
pub struct ProcessTree {
    #[cfg(windows)]
    job: Option<JobHandle>,
    #[cfg(unix)]
    pgid: Option<i32>,
}

impl ProcessTree {
    /// Take ownership of `child`'s process tree. `child` must come from a command prepared with
    /// [`prepare_tree_command`]. Never fails: when the platform refuses (e.g. a job assignment is
    /// denied), the tree degrades to `kill_on_drop` of the direct child and a warning is logged.
    pub fn attach(child: &Child) -> Self {
        #[cfg(windows)]
        {
            let job = child.raw_handle().and_then(|handle| {
                JobHandle::kill_on_close_for(handle)
                    .map_err(|err| {
                        tracing::warn!(
                            error = %err,
                            "Could not place the subprocess in a job object; only the direct child will be terminated"
                        );
                    })
                    .ok()
            });
            Self { job }
        }
        #[cfg(unix)]
        {
            // `process_group(0)` makes the child the leader of a group whose id is its pid.
            let pgid = child.id().and_then(|pid| i32::try_from(pid).ok());
            Self { pgid }
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = child;
            Self {}
        }
    }

    /// Terminate every process in the tree now. Idempotent.
    ///
    /// On Unix this must run while the group leader has not been reaped yet (i.e. before
    /// `Child::wait`/`Child::kill` return), otherwise the id could in theory belong to a new group;
    /// call [`ProcessTree::release`] once the child has been waited for.
    pub fn kill(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            job.terminate();
        }
        #[cfg(unix)]
        if let Some(pgid) = self.pgid.take() {
            // SAFETY: plain syscall; a group that no longer exists yields ESRCH, which is ignored.
            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
        }
    }

    /// Stop tracking the tree after its leader exited normally and was waited for. On Windows the
    /// job is still closed, which ends processes the agent left running in the background.
    pub fn release(&mut self) {
        #[cfg(windows)]
        self.kill();
        #[cfg(unix)]
        {
            self.pgid = None;
        }
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Spawn `cmd` (stdout/stderr should be piped) as a process tree and collect its output. Returns
/// `None` when `limit` elapses first; the whole tree is then killed before the child is reaped, so
/// a timed-out hook or validation command cannot leave e.g. `node` running behind `npm test`.
pub async fn output_with_timeout(
    cmd: &mut Command,
    limit: std::time::Duration,
) -> Option<std::io::Result<std::process::Output>> {
    prepare_tree_command(cmd);
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => return Some(Err(err)),
    };
    let mut tree = ProcessTree::attach(&child);
    // Pinned outside `select!` so the child is still owned (unreaped) when the timer wins.
    let output = child.wait_with_output();
    tokio::pin!(output);
    tokio::select! {
        res = &mut output => {
            tree.release();
            Some(res)
        }
        _ = tokio::time::sleep(limit) => {
            tree.kill();
            None
        }
    }
}

#[cfg(windows)]
struct JobHandle(windows_sys::Win32::Foundation::HANDLE);

// SAFETY: a job object handle is a kernel handle that may be used and closed from any thread.
#[cfg(windows)]
unsafe impl Send for JobHandle {}
#[cfg(windows)]
unsafe impl Sync for JobHandle {}

#[cfg(windows)]
impl JobHandle {
    fn kill_on_close_for(process: std::os::windows::io::RawHandle) -> std::io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        // SAFETY: anonymous job object with default security; the handle is owned by `JobHandle`.
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let job = Self(raw);

        // SAFETY: zero is a valid bit pattern for this plain-data struct.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: `limits` is a valid JOBOBJECT_EXTENDED_LIMIT_INFORMATION of the size passed.
        let ok = unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: `process` is the live process handle owned by the tokio `Child`.
        if unsafe { AssignProcessToJobObject(job.0, process as _) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(job)
    }

    fn terminate(self) {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        // SAFETY: valid job handle; closing it afterwards (Drop) also kills because of
        // KILL_ON_JOB_CLOSE, terminating explicitly just makes it immediate.
        unsafe {
            TerminateJobObject(self.0, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for JobHandle {
    fn drop(&mut self) {
        // SAFETY: the handle was created by CreateJobObjectW and is closed exactly once.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Duration;

    fn pid_alive(pid: i32) -> bool {
        // SAFETY: signal 0 only checks for existence.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[tokio::test]
    async fn dropping_the_tree_kills_grandchildren() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("grandchild.pid");
        let mut cmd = Command::new("sh");
        // The wrapper starts a long-running grandchild in the background, records its pid and
        // then waits itself, like `cmd.exe /C` or an npm shim would.
        cmd.arg("-c").arg(format!(
            "sleep 300 & echo $! > '{}'; wait",
            pid_file.display()
        ));
        prepare_tree_command(&mut cmd);
        let child = cmd.spawn().unwrap();
        let tree = ProcessTree::attach(&child);

        let grandchild = loop {
            if let Ok(raw) = std::fs::read_to_string(&pid_file) {
                if let Ok(pid) = raw.trim().parse::<i32>() {
                    break pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(pid_alive(grandchild));

        // Same drop order as the agent sessions: the tree first, while the leader is unreaped.
        drop(tree);
        drop(child);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while pid_alive(grandchild) {
            // A killed grandchild is a zombie until its (also killed) parent is reaped by init.
            let stat =
                std::fs::read_to_string(format!("/proc/{grandchild}/stat")).unwrap_or_default();
            if stat.is_empty() || stat.contains(") Z ") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "grandchild {grandchild} survived the tree"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn output_with_timeout_collects_output_and_kills_on_timeout() {
        let mut ok = Command::new("sh");
        ok.arg("-c")
            .arg("echo hello")
            .stdout(std::process::Stdio::piped());
        let out = output_with_timeout(&mut ok, Duration::from_secs(10))
            .await
            .expect("finished in time")
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");

        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("grandchild.pid");
        let mut slow = Command::new("sh");
        slow.arg("-c")
            .arg(format!(
                "sleep 300 & echo $! > '{}'; wait",
                pid_file.display()
            ))
            .stdout(std::process::Stdio::piped());
        let started = std::time::Instant::now();
        assert!(output_with_timeout(&mut slow, Duration::from_millis(500))
            .await
            .is_none());
        assert!(started.elapsed() < Duration::from_secs(10));
        let grandchild: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let stat =
                std::fs::read_to_string(format!("/proc/{grandchild}/stat")).unwrap_or_default();
            if !pid_alive(grandchild) || stat.is_empty() || stat.contains(") Z ") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "grandchild survived the timeout"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[tokio::test]
    async fn released_tree_leaves_finished_processes_alone() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg("exit 0");
        prepare_tree_command(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let mut tree = ProcessTree::attach(&child);
        assert!(child.wait().await.unwrap().success());
        tree.release();
        assert!(
            tree.pgid.is_none(),
            "a reaped group id must not be signalled later"
        );
    }
}
