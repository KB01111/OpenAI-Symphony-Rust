use chrono::Utc;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use symphony_core::{HardwareSnapshot, ResourcePolicy, SymphonyError, SymphonyResult};
use sysinfo::System;
use tracing::info;

pub mod logging;
pub mod secure_fs;
mod vault;

pub use vault::{SealedBlob, SecretStore, VaultProtector};

/// Windows 11 Power Status via `kernel32!GetSystemPowerStatus`.
#[derive(Debug, Clone, Copy)]
pub struct PowerStatus {
    pub on_battery: bool,
    pub battery_percent: Option<u8>,
    pub battery_saver_active: bool,
}

#[cfg(windows)]
mod win_power {
    use super::PowerStatus;

    #[repr(C)]
    struct SystemPowerStatus {
        ac_line_status: u8,
        battery_flag: u8,
        battery_life_percent: u8,
        system_status_flag: u8,
        battery_life_time: u32,
        battery_full_life_time: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemPowerStatus(lp_system_power_status: *mut SystemPowerStatus) -> i32;
    }

    pub fn query_power_status() -> PowerStatus {
        let mut status = SystemPowerStatus {
            ac_line_status: 1,
            battery_flag: 0,
            battery_life_percent: 255,
            system_status_flag: 0,
            battery_life_time: 0,
            battery_full_life_time: 0,
        };
        let ok = unsafe { GetSystemPowerStatus(&mut status as *mut SystemPowerStatus) };
        if ok == 0 {
            return PowerStatus {
                on_battery: false,
                battery_percent: None,
                battery_saver_active: false,
            };
        }
        let on_battery = status.ac_line_status == 0;
        let battery_percent = if status.battery_life_percent <= 100 {
            Some(status.battery_life_percent)
        } else {
            None
        };
        let battery_saver_active = status.system_status_flag == 1;
        PowerStatus {
            on_battery,
            battery_percent,
            battery_saver_active,
        }
    }
}

#[cfg(not(windows))]
mod win_power {
    use super::PowerStatus;
    pub fn query_power_status() -> PowerStatus {
        PowerStatus {
            on_battery: false,
            battery_percent: None,
            battery_saver_active: false,
        }
    }
}

/// Hardware-aware monitor optimized for Windows 11 laptops (e.g., ASUS Zephyrus G16 with
/// AMD Ryzen AI 9 HX 370, 32 GB RAM, hybrid Radeon 890M / RTX 4070 where dGPU may be disabled on battery).
/// Never probes discrete GPUs at startup so it will never wake a sleeping dGPU on battery.
#[derive(Clone)]
pub struct HardwareMonitor {
    sys: Arc<Mutex<System>>,
}

impl Default for HardwareMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl HardwareMonitor {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_all();
        sys.refresh_memory();
        Self {
            sys: Arc::new(Mutex::new(sys)),
        }
    }

    /// Capture lightweight CPU, RAM, and AC/battery telemetry and compute recommended concurrency.
    pub fn snapshot(&self, policy: &ResourcePolicy, configured_max: u32) -> HardwareSnapshot {
        let (cpu_brand, cpu_usage, total_mem_mb, used_mem_mb, avail_mem_mb) = {
            let mut guard = self.sys.lock().expect("sysinfo mutex");
            guard.refresh_cpu_usage();
            guard.refresh_memory();

            let brand = guard
                .cpus()
                .first()
                .map(|c| c.brand().trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "AMD Ryzen AI / x86_64 Processor".to_string());

            let usage = guard.global_cpu_usage();
            let total = guard.total_memory() / (1024 * 1024);
            let used = guard.used_memory() / (1024 * 1024);
            let avail = guard.available_memory() / (1024 * 1024);
            (brand, usage, total, used, avail)
        };

        let mem_pct = if total_mem_mb > 0 {
            (used_mem_mb as f32 / total_mem_mb as f32) * 100.0
        } else {
            0.0
        };

        let power = win_power::query_power_status();
        let thermal_throttling = cpu_usage >= policy.high_cpu_threshold_percent;

        let base_recommended = if power.on_battery {
            policy.max_concurrent_on_battery
        } else {
            policy.max_concurrent_on_ac
        };

        let mut effective = configured_max.min(base_recommended.max(1));
        let mut delay_reason = None;

        if mem_pct >= policy.high_memory_threshold_percent {
            effective = 0;
            delay_reason = Some(format!(
                "High memory pressure ({mem_pct:.1}% >= {:.1}%): delaying new agent dispatches",
                policy.high_memory_threshold_percent
            ));
        } else if policy.reduce_on_thermal_throttle && thermal_throttling {
            effective = (effective / 2).max(1);
            delay_reason = Some(format!(
                "High CPU load ({cpu_usage:.1}%): throttling concurrent agent dispatches to {effective}"
            ));
        }

        let power_mode = if power.on_battery {
            if power.battery_saver_active {
                "Battery Saver (Eco / iGPU Only)".to_string()
            } else {
                "Battery Power (1 Agent Limit)".to_string()
            }
        } else {
            "AC Power (Performance)".to_string()
        };

        HardwareSnapshot {
            cpu_brand,
            cpu_usage_percent: (cpu_usage * 10.0).round() / 10.0,
            total_memory_mb: total_mem_mb,
            used_memory_mb: used_mem_mb,
            available_memory_mb: avail_mem_mb,
            memory_usage_percent: (mem_pct * 10.0).round() / 10.0,
            on_battery: power.on_battery,
            battery_percent: power.battery_percent,
            power_mode,
            thermal_throttling,
            recommended_max_concurrency: base_recommended,
            effective_max_concurrency: effective,
            dispatch_delayed_reason: delay_reason,
            captured_at: Utc::now(),
        }
    }
}

/// Single-instance runtime lock to prevent spawning duplicate background orchestrator instances (Section 12).
pub struct SingleInstanceGuard {
    lock_path: PathBuf,
    pid: u32,
    /// Holds the OS-level exclusive lock for as long as the guard lives; the OS releases it when
    /// the process exits, so a crashed owner never leaves a stale lock behind.
    _file: File,
}

impl SingleInstanceGuard {
    /// Become the single runtime owner of `runtime_dir`. Acquisition is atomic across processes
    /// (exclusive advisory file lock), so a CLI and a desktop host starting concurrently cannot
    /// both own the same data directory.
    ///
    /// A lock whose recorded owner has already exited is waited for (up to [`STALE_LOCK_GRACE`])
    /// instead of being reported as a running runtime: Windows releases the locks of a terminated
    /// process asynchronously, so a restart right after a crash can still find the lock held.
    ///
    /// Fails with [`SymphonyError::InvalidConfig`] when another runtime holds the lock; any other
    /// error is an I/O failure.
    pub fn acquire(runtime_dir: &Path) -> SymphonyResult<Self> {
        Self::acquire_within(runtime_dir, STALE_LOCK_GRACE)
    }

    /// [`acquire`](Self::acquire) without waiting for a lock left by an exited owner.
    pub fn try_acquire(runtime_dir: &Path) -> SymphonyResult<Self> {
        Self::acquire_within(runtime_dir, Duration::ZERO)
    }

    fn acquire_within(runtime_dir: &Path, grace: Duration) -> SymphonyResult<Self> {
        std::fs::create_dir_all(runtime_dir)?;
        let lock_path = runtime_dir.join("symphony-runtime.lock");
        let current_pid = std::process::id();

        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        let deadline = Instant::now() + grace;
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => {
                    let owner = lock_owner_pid(&lock_path, runtime_dir);
                    // An unknown owner may be a runtime that has not published its PID yet: treat
                    // it as running, like a live one.
                    let owner_exited = owner.is_some_and(|pid| !is_process_alive(pid));
                    if owner_exited && Instant::now() < deadline {
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    return Err(SymphonyError::InvalidConfig(match owner {
                        Some(pid) => {
                            format!("Symphony background runtime is already running (PID {pid})")
                        }
                        None => "Symphony background runtime is already running".to_string(),
                    }));
                }
                Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
            }
        }
        file.set_len(0)?;
        write!(file, "{current_pid}")?;
        file.flush()?;
        // A discovery record left behind now belongs to a dead owner. Until this runtime publishes
        // its own, a contender would read that PID as the lock owner's and wait for its release.
        RuntimeDiscovery::remove(runtime_dir);
        info!(pid = current_pid, lock = %lock_path.display(), "Acquired single-instance runtime lock");

        Ok(Self {
            lock_path,
            pid: current_pid,
            _file: file,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }
}

/// PID of the runtime holding the lock: from the lock file, or — where the lock also blocks
/// reading it (Windows) — from the discovery record the owner published.
fn lock_owner_pid(lock_path: &Path, runtime_dir: &Path) -> Option<u32> {
    let mut contents = String::new();
    File::open(lock_path)
        .and_then(|mut f| f.read_to_string(&mut contents))
        .ok()
        .and_then(|_| contents.trim().parse::<u32>().ok())
        .or_else(|| {
            let raw =
                secure_fs::read_private_file(&RuntimeDiscovery::file_path(runtime_dir)).ok()?;
            let raw = zeroize::Zeroizing::new(raw);
            serde_json::from_slice::<RuntimeDiscovery>(&raw)
                .ok()
                .map(|r| r.pid)
        })
}

/// How long [`SingleInstanceGuard::acquire`] waits for the OS to release a lock whose owner has
/// already exited.
pub const STALE_LOCK_GRACE: Duration = Duration::from_secs(10);

/// Inspect whether a process PID is currently running on the host (used during crash recovery & lock check).
pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        win_process::is_running(pid)
    }
    #[cfg(not(windows))]
    {
        let mut sys = System::new();
        let sys_pid = sysinfo::Pid::from_u32(pid);
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[sys_pid]), true);
        sys.process(sys_pid).is_some()
    }
}

/// A Windows process that has exited stays in the system process list while anything still holds
/// a handle to it (its parent, an antivirus scan), so a process snapshot reports it as running;
/// the process object's own signalled state is authoritative.
#[cfg(windows)]
mod win_process {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_ACCESS_DENIED, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    pub fn is_running(pid: u32) -> bool {
        // SAFETY: plain handle APIs; the handle is closed before returning.
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                // A process we may not open (another account's, or protected) still exists; an
                // unused PID fails with ERROR_INVALID_PARAMETER.
                return GetLastError() == ERROR_ACCESS_DENIED;
            }
            // Only a signalled process has exited; a failed wait is no evidence of that.
            let running = WaitForSingleObject(handle, 0) != WAIT_OBJECT_0;
            CloseHandle(handle);
            running
        }
    }
}

/// Name of the per-user autostart value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
pub const AUTOSTART_VALUE_NAME: &str = "SymphonyDesktopRuntime";

/// Command line registered for autostart: the quoted executable plus `--background`.
pub fn autostart_command(exe_path: &Path) -> String {
    format!("\"{}\" --background", exe_path.display())
}

/// Configure or remove Windows user-session startup registration via
/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` without administrator privileges.
///
/// Uses the registry API directly and reads the value back, so a rejected write (group policy,
/// registry virtualization, access denied) is returned as an error instead of being reported as
/// success. On other platforms this is a no-op.
pub fn configure_windows_autostart(enable: bool, exe_path: &Path) -> SymphonyResult<()> {
    #[cfg(windows)]
    {
        win_autostart::configure(AUTOSTART_VALUE_NAME, enable, exe_path)
    }
    #[cfg(not(windows))]
    {
        let _ = (enable, exe_path);
        Ok(())
    }
}

/// The currently registered autostart command, if any (always `None` off Windows).
pub fn windows_autostart_command() -> SymphonyResult<Option<String>> {
    #[cfg(windows)]
    {
        win_autostart::read(AUTOSTART_VALUE_NAME)
    }
    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

#[cfg(windows)]
#[doc(hidden)]
pub mod win_autostart {
    //! Registry access for autostart; `value_name` is a parameter so tests use their own value.
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::null_mut;
    use symphony_core::{SymphonyError, SymphonyResult};
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
        RRF_RT_REG_SZ,
    };

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    fn registry_error(action: &str, code: u32) -> SymphonyError {
        SymphonyError::IoError(format!(
            "Windows rejected {action} of the autostart entry: {}",
            std::io::Error::from_raw_os_error(code as i32)
        ))
    }

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the key was opened by RegCreateKeyExW.
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn open_run_key(access: u32) -> SymphonyResult<Key> {
        let mut key: HKEY = null_mut();
        // SAFETY: valid NUL-terminated subkey name and out pointer.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(RUN_KEY).as_ptr(),
                0,
                null_mut(),
                REG_OPTION_NON_VOLATILE,
                access,
                null_mut(),
                &mut key,
                null_mut(),
            )
        };
        if status != 0 {
            return Err(registry_error("opening the Run key for", status));
        }
        Ok(Key(key))
    }

    pub fn read(value_name: &str) -> SymphonyResult<Option<String>> {
        let key = open_run_key(KEY_QUERY_VALUE)?;
        let name = wide(value_name);
        let mut buf = vec![0u16; 1024];
        loop {
            let mut size = (buf.len() * 2) as u32;
            // SAFETY: `buf` has `size` writable bytes.
            let status = unsafe {
                RegGetValueW(
                    key.0,
                    null_mut(),
                    name.as_ptr(),
                    RRF_RT_REG_SZ,
                    null_mut(),
                    buf.as_mut_ptr() as *mut _,
                    &mut size,
                )
            };
            match status {
                0 => {
                    let len = (size as usize / 2).saturating_sub(1);
                    return Ok(Some(String::from_utf16_lossy(&buf[..len])));
                }
                ERROR_FILE_NOT_FOUND => return Ok(None),
                ERROR_MORE_DATA => buf.resize(size as usize / 2 + 1, 0),
                code => return Err(registry_error("reading", code)),
            }
        }
    }

    pub fn configure(value_name: &str, enable: bool, exe_path: &Path) -> SymphonyResult<()> {
        let key = open_run_key(KEY_SET_VALUE | KEY_QUERY_VALUE)?;
        let name = wide(value_name);
        if enable {
            let command = super::autostart_command(exe_path);
            let data = wide(&command);
            // SAFETY: `data` is a NUL-terminated UTF-16 string of the given byte length.
            let status = unsafe {
                RegSetValueExW(
                    key.0,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    data.as_ptr() as *const u8,
                    (data.len() * 2) as u32,
                )
            };
            if status != 0 {
                return Err(registry_error("writing", status));
            }
            if read(value_name)?.as_deref() != Some(command.as_str()) {
                return Err(SymphonyError::IoError(
                    "The autostart entry did not persist (registry write was not applied)".into(),
                ));
            }
        } else {
            // SAFETY: valid key and NUL-terminated value name.
            let status = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
            if status != 0 && status != ERROR_FILE_NOT_FOUND {
                return Err(registry_error("removing", status));
            }
            if read(value_name)?.is_some() {
                return Err(SymphonyError::IoError(
                    "The autostart entry could not be removed".into(),
                ));
            }
        }
        Ok(())
    }
}

/// Runtime endpoint discovery record written by the process that owns the orchestration runtime.
///
/// Stored at `<data_dir>/runtime.json` with owner-only permissions so local clients (the desktop
/// webview via Tauri IPC, the Vite dev server, integration tests) can find the API port — which
/// may be an ephemeral fallback — and its per-launch access token.
#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct RuntimeDiscovery {
    pub base_url: String,
    pub token: String,
    pub pid: u32,
    pub api_version: u32,
    pub owner: String,
}

impl RuntimeDiscovery {
    pub fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join("runtime.json")
    }

    /// Atomically publish the record, readable only by the current user.
    ///
    /// The temp file is created with owner-only permissions (Unix `0600`; on Windows a protected
    /// DACL for the current user and `SYSTEM`, applied at creation so inherited directory ACLs
    /// never apply), verified, and only then renamed to `runtime.json`. If the permissions cannot
    /// be enforced (e.g. a FAT volume or a foreign-owned directory) nothing is published and an
    /// error is returned — there is no insecure fallback.
    pub fn write(&self, data_dir: &Path) -> SymphonyResult<PathBuf> {
        secure_fs::ensure_dir(data_dir)?;
        let path = Self::file_path(data_dir);
        let json = zeroize::Zeroizing::new(
            serde_json::to_vec_pretty(self).map_err(|e| SymphonyError::JsonError(e.to_string()))?,
        );
        secure_fs::write_private_atomic(&path, &json).map_err(|e| {
            SymphonyError::IoError(format!(
                "Could not publish runtime discovery file {}: {e}",
                path.display()
            ))
        })?;
        Ok(path)
    }

    /// Read a discovery record whose owning process is still alive.
    ///
    /// A record that is not private to the current user is ignored: it may have been planted by
    /// another account to redirect clients (and their requests) to a server it controls.
    pub fn read_live(data_dir: &Path) -> Option<Self> {
        let path = Self::file_path(data_dir);
        // Checked and read through one handle, so the file cannot be swapped in between.
        let raw = match secure_fs::read_private_file(&path) {
            Ok(raw) => zeroize::Zeroizing::new(raw),
            Err(err) => {
                if err.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(error = %err, "Ignoring runtime discovery file with unsafe permissions");
                }
                return None;
            }
        };
        let record: Self = serde_json::from_slice(&raw).ok()?;
        is_process_alive(record.pid).then_some(record)
    }

    pub fn remove(data_dir: &Path) {
        let _ = std::fs::remove_file(Self::file_path(data_dir));
    }
}

impl std::fmt::Debug for RuntimeDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeDiscovery")
            .field("base_url", &self.base_url)
            .field("token", &"[redacted]")
            .field("pid", &self.pid)
            .field("api_version", &self.api_version)
            .field("owner", &self.owner)
            .finish()
    }
}

/// Default application data directory shared by the desktop host and `symphony-cli`
/// (`%LOCALAPPDATA%\symphony-desktop` on Windows).
pub fn default_app_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("symphony-desktop")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_hardware_snapshot_and_resource_policy() {
        let monitor = HardwareMonitor::new();
        let policy = ResourcePolicy::default();
        let snap = monitor.snapshot(&policy, 4);
        assert!(snap.total_memory_mb > 0);
        assert!(snap.recommended_max_concurrency >= 1);
    }

    #[test]
    fn test_single_instance_lock() {
        let dir = tempdir().unwrap();
        let guard = SingleInstanceGuard::acquire(dir.path()).unwrap();
        assert_eq!(guard.pid(), std::process::id());
        // A second owner is rejected while the first guard holds the lock.
        assert!(SingleInstanceGuard::acquire(dir.path()).is_err());
        drop(guard);
        let guard2 = SingleInstanceGuard::acquire(dir.path()).unwrap();
        assert_eq!(guard2.pid(), std::process::id());
    }

    /// PID of a process that has exited; the returned child still holds its process handle.
    fn exited_child() -> (std::process::Child, u32) {
        #[cfg(windows)]
        let mut child = std::process::Command::new("cmd")
            .args(["/C", "exit"])
            .spawn()
            .unwrap();
        #[cfg(not(windows))]
        let mut child = std::process::Command::new("true").spawn().unwrap();
        child.wait().unwrap();
        let pid = child.id();
        (child, pid)
    }

    /// Hold the lock the way a runtime does, after recording `owner` as its PID.
    fn hold_lock(dir: &Path, owner: u32) -> File {
        RuntimeDiscovery {
            base_url: "http://127.0.0.1:1".into(),
            token: "t".repeat(64),
            pid: owner,
            api_version: 1,
            owner: "symphony-cli".into(),
        }
        .write(dir)
        .unwrap();
        let lock_path = dir.join("symphony-runtime.lock");
        std::fs::write(&lock_path, owner.to_string()).unwrap();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        file.lock().unwrap();
        file
    }

    #[test]
    fn an_exited_process_is_not_alive_while_a_handle_to_it_is_open() {
        assert!(is_process_alive(std::process::id()));
        let (_child, pid) = exited_child();
        assert!(!is_process_alive(pid));
    }

    #[test]
    fn a_live_lock_owner_is_refused_without_waiting() {
        let dir = tempdir().unwrap();
        let _held = hold_lock(dir.path(), std::process::id());
        let start = Instant::now();
        let Err(err) = SingleInstanceGuard::acquire_within(dir.path(), Duration::from_secs(30))
        else {
            panic!("a second owner must be refused");
        };
        assert!(err.to_string().contains("already running"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_lock_left_by_an_exited_owner_is_taken_over_once_the_os_releases_it() {
        let dir = tempdir().unwrap();
        let (_child, dead) = exited_child();
        let held = hold_lock(dir.path(), dead);
        // The OS releases a terminated process's lock with a delay.
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            drop(held);
        });
        let guard =
            SingleInstanceGuard::acquire_within(dir.path(), Duration::from_secs(30)).unwrap();
        assert_eq!(guard.pid(), std::process::id());
        release.join().unwrap();
    }

    #[test]
    fn acquiring_the_lock_discards_a_dead_owners_discovery_record() {
        let dir = tempdir().unwrap();
        let (_child, dead) = exited_child();
        drop(hold_lock(dir.path(), dead));
        assert!(RuntimeDiscovery::file_path(dir.path()).exists());
        let _guard = SingleInstanceGuard::try_acquire(dir.path()).unwrap();
        assert!(!RuntimeDiscovery::file_path(dir.path()).exists());
    }

    #[test]
    fn the_wait_for_a_stale_lock_is_bounded() {
        let dir = tempdir().unwrap();
        let (_child, dead) = exited_child();
        let _held = hold_lock(dir.path(), dead);
        let start = Instant::now();
        assert!(
            SingleInstanceGuard::acquire_within(dir.path(), Duration::from_millis(300)).is_err()
        );
        assert!(start.elapsed() >= Duration::from_millis(300));
    }
}
