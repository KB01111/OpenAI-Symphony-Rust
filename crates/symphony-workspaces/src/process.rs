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
    /// On Unix this must run while the group leader has not been reaped yet, otherwise the group
    /// id could in theory have been reused; [`ProcessTree::wait`] takes care of that.
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

    /// Wait for `child` (the tree's leader) to exit, end whatever it left running in its tree
    /// (background tools must not keep editing a workspace after the turn or hook is over), then
    /// reap it. Cancel-safe: dropping the future leaves the leader unreaped, so [`kill`] (or
    /// `Drop`) still targets the right processes.
    ///
    /// [`kill`]: ProcessTree::kill
    pub async fn wait(&mut self, child: &mut Child) -> std::io::Result<std::process::ExitStatus> {
        #[cfg(unix)]
        if let Some(pid) = self.pgid {
            // Observe the exit without reaping: while the leader is a zombie its id cannot be
            // reused, so signalling the group afterwards cannot hit an unrelated process.
            let _ = tokio::task::spawn_blocking(move || wait_for_exit_without_reaping(pid)).await;
            self.kill();
        }
        let status = child.wait().await;
        #[cfg(windows)]
        self.kill();
        status
    }
}

/// Block until `pid` (our child) has exited, leaving it unreaped (`WNOWAIT`).
#[cfg(unix)]
fn wait_for_exit_without_reaping(pid: i32) {
    loop {
        // SAFETY: zero-initialised siginfo_t is valid output storage for waitid.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: plain syscall on our own child.
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if rc == 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return;
        }
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Spawn `cmd` (stdout/stderr should be piped) as a process tree and collect its output. Returns
/// `None` when `limit` elapses first. Either way the whole tree ends with the command: on a
/// timeout it is killed before the leader is reaped, and processes the command left running in
/// the background are ended when it exits, so a hook or validation cannot leave e.g. `node`
/// running behind `npm test`.
pub async fn output_with_timeout(
    cmd: &mut Command,
    limit: std::time::Duration,
) -> Option<std::io::Result<std::process::Output>> {
    let deadline = tokio::time::Instant::now() + limit;
    prepare_tree_command(cmd);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => return Some(Err(err)),
    };
    let mut tree = ProcessTree::attach(&child);
    let stdout = tokio::spawn(read_to_end(child.stdout.take()));
    let stderr = tokio::spawn(read_to_end(child.stderr.take()));

    let status = match tokio::time::timeout_at(deadline, tree.wait(&mut child)).await {
        Ok(Ok(status)) => status,
        Ok(Err(err)) => return Some(Err(err)),
        Err(_) => {
            tree.kill();
            stdout.abort();
            stderr.abort();
            return None;
        }
    };
    // The tree is gone, so the pipes reach EOF unless a process escaped it (e.g. `setsid`).
    let collected = tokio::time::timeout_at(deadline, async {
        (
            stdout.await.unwrap_or_default(),
            stderr.await.unwrap_or_default(),
        )
    })
    .await;
    let (stdout, stderr) = match collected {
        Ok(output) => output,
        Err(_) => return None,
    };
    Some(Ok(std::process::Output {
        status,
        stdout,
        stderr,
    }))
}

async fn read_to_end<R: tokio::io::AsyncRead + Unpin>(pipe: Option<R>) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut buf).await;
    }
    buf
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
    async fn a_finished_leader_takes_its_background_processes_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("background.pid");
        // The leader exits successfully but leaves a detached background "tool" behind.
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(format!(
            "sleep 300 >/dev/null 2>&1 & echo $! > '{}'; exit 0",
            pid_file.display()
        ));
        prepare_tree_command(&mut cmd);
        let mut child = cmd.spawn().unwrap();
        let mut tree = ProcessTree::attach(&child);
        assert!(tree.wait(&mut child).await.unwrap().success());
        assert!(
            tree.pgid.is_none(),
            "a reaped group id must never be signalled later"
        );

        let background: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let stat =
                std::fs::read_to_string(format!("/proc/{background}/stat")).unwrap_or_default();
            if !pid_alive(background) || stat.is_empty() || stat.contains(") Z ") {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the background process outlived its finished leader"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::path::Path;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };

    fn running(pid: u32) -> bool {
        // SAFETY: plain handle queries; the handle is closed before returning.
        unsafe {
            let handle = OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            );
            if handle.is_null() {
                return false;
            }
            let exited = WaitForSingleObject(handle, 0) == WAIT_OBJECT_0;
            CloseHandle(handle);
            !exited
        }
    }

    /// `powershell.exe` that starts a long-running grandchild (like an agent starting a tool),
    /// records its pid and keeps running itself.
    fn wrapper_with_grandchild(pid_file: &Path) -> Command {
        let mut cmd = Command::new("powershell.exe");
        cmd.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"]).arg(format!(
            "$p = Start-Process -FilePath ping.exe -ArgumentList '-n','300','127.0.0.1' -WindowStyle Hidden -PassThru; \
             Set-Content -Path '{}' -Value $p.Id; Start-Sleep -Seconds 300",
            pid_file.display()
        ));
        cmd
    }

    async fn read_pid(pid_file: &Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Ok(raw) = std::fs::read_to_string(pid_file) {
                if let Ok(pid) = raw.trim().parse() {
                    return pid;
                }
            }
            assert!(
                Instant::now() < deadline,
                "the wrapper never started its grandchild"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn assert_terminated(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while running(pid) {
            assert!(
                Instant::now() < deadline,
                "process {pid} survived its job object"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn dropping_the_tree_kills_grandchildren_through_the_job_object() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("grandchild.pid");
        let mut cmd = wrapper_with_grandchild(&pid_file);
        prepare_tree_command(&mut cmd);
        let child = cmd.spawn().unwrap();
        let tree = ProcessTree::attach(&child);
        assert!(
            tree.job.is_some(),
            "the child must be assigned to a job object"
        );

        let grandchild = read_pid(&pid_file).await;
        assert!(running(grandchild));
        let wrapper = child.id().unwrap();

        drop(tree);
        drop(child);
        assert_terminated(grandchild).await;
        assert_terminated(wrapper).await;
    }

    #[tokio::test]
    async fn a_timed_out_command_takes_its_grandchildren_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("grandchild.pid");
        let mut cmd = wrapper_with_grandchild(&pid_file);
        cmd.stdout(std::process::Stdio::piped());
        assert!(output_with_timeout(&mut cmd, Duration::from_secs(15))
            .await
            .is_none());
        assert_terminated(read_pid(&pid_file).await).await;
    }
}
