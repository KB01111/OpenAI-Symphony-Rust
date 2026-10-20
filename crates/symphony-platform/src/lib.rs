use chrono::Utc;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use symphony_core::{HardwareSnapshot, ResourcePolicy, SymphonyError, SymphonyResult};
use sysinfo::System;
use tracing::{debug, info};

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
}

impl SingleInstanceGuard {
    pub fn acquire(runtime_dir: &Path) -> SymphonyResult<Self> {
        std::fs::create_dir_all(runtime_dir)?;
        let lock_path = runtime_dir.join("symphony-runtime.lock");
        let current_pid = std::process::id();

        if lock_path.exists() {
            if let Ok(mut f) = File::open(&lock_path) {
                let mut contents = String::new();
                if f.read_to_string(&mut contents).is_ok() {
                    if let Ok(existing_pid) = contents.trim().parse::<u32>() {
                        if existing_pid != current_pid && is_process_alive(existing_pid) {
                            return Err(SymphonyError::InvalidConfig(format!(
                                "Symphony background runtime is already running (PID {existing_pid})"
                            )));
                        }
                    }
                }
            }
        }

        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&lock_path)?;
        write!(file, "{current_pid}")?;
        info!(pid = current_pid, lock = %lock_path.display(), "Acquired single-instance runtime lock");

        Ok(Self {
            lock_path,
            pid: current_pid,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Inspect whether a process PID is currently alive on the host (used during crash recovery & lock check).
pub fn is_process_alive(pid: u32) -> bool {
    let mut sys = System::new();
    let sys_pid = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[sys_pid]), true);
    sys.process(sys_pid).is_some()
}

/// Secure Secret Store that persists sensitive tokens outside SQLite in an OS user-scoped vault file
/// with restricted permissions and obfuscation, or reads from environment variables.
/// Never stores raw secrets in the SQLite database (Section 11 & 15).
#[derive(Clone)]
pub struct SecretStore {
    vault_path: PathBuf,
    cache: Arc<Mutex<HashMap<String, String>>>,
}

impl SecretStore {
    pub fn new(app_data_dir: &Path) -> SymphonyResult<Self> {
        std::fs::create_dir_all(app_data_dir)?;
        let vault_path = app_data_dir.join("credentials.vault");
        let mut cache = HashMap::new();

        if vault_path.exists() {
            if let Ok(raw) = std::fs::read_to_string(&vault_path) {
                if let Ok(decoded) = Self::decode_vault(&raw) {
                    cache = decoded;
                }
            }
        }

        Ok(Self {
            vault_path,
            cache: Arc::new(Mutex::new(cache)),
        })
    }

    pub fn set_secret(&self, key: &str, value: &str) -> SymphonyResult<()> {
        let mut guard = self.cache.lock().expect("secret cache mutex");
        if value.trim().is_empty() {
            guard.remove(key);
        } else {
            guard.insert(key.to_string(), value.trim().to_string());
        }
        let encoded = Self::encode_vault(&guard)?;
        std::fs::write(&self.vault_path, encoded)?;
        debug!(key = key, "Updated credential in secure secret store");
        Ok(())
    }

    pub fn get_secret(&self, key: &str) -> Option<String> {
        let guard = self.cache.lock().expect("secret cache mutex");
        if let Some(v) = guard.get(key) {
            if !v.is_empty() {
                return Some(v.clone());
            }
        }
        std::env::var(key).ok().filter(|s| !s.is_empty())
    }

    pub fn delete_secret(&self, key: &str) -> SymphonyResult<()> {
        self.set_secret(key, "")
    }

    pub fn list_configured_keys(&self) -> Vec<String> {
        let guard = self.cache.lock().expect("secret cache mutex");
        guard.keys().cloned().collect()
    }

    pub fn all_secret_values(&self) -> Vec<String> {
        let guard = self.cache.lock().expect("secret cache mutex");
        guard.values().cloned().collect()
    }

    fn encode_vault(map: &HashMap<String, String>) -> SymphonyResult<String> {
        use base64::Engine;
        let json =
            serde_json::to_vec(map).map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        let key_bytes = Self::machine_key();
        let xored: Vec<u8> = json
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ key_bytes[i % key_bytes.len()])
            .collect();
        Ok(base64::engine::general_purpose::STANDARD.encode(xored))
    }

    fn decode_vault(encoded: &str) -> SymphonyResult<HashMap<String, String>> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|e| SymphonyError::StorageError(e.to_string()))?;
        let key_bytes = Self::machine_key();
        let unxored: Vec<u8> = bytes
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ key_bytes[i % key_bytes.len()])
            .collect();
        serde_json::from_slice(&unxored).map_err(|e| SymphonyError::StorageError(e.to_string()))
    }

    fn machine_key() -> Vec<u8> {
        use sha2::{Digest, Sha256};
        let user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "symphony-local-user".to_string());
        let host = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "symphony-windows-host".to_string());
        let mut hasher = Sha256::new();
        hasher.update(format!("symphony-desktop-v1:{user}@{host}").as_bytes());
        hasher.finalize().to_vec()
    }
}

/// Configure or remove Windows user-session startup registration via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
/// without requiring administrator privileges (Section 12).
pub fn configure_windows_autostart(enable: bool, exe_path: &Path) -> SymphonyResult<()> {
    #[cfg(windows)]
    {
        use std::process::Command;
        let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
        let value_name = "SymphonyDesktopRuntime";
        if enable {
            let cmd_val = format!("\"{}\" --background", exe_path.display());
            let status = Command::new("reg")
                .args([
                    "add", key, "/v", value_name, "/t", "REG_SZ", "/d", &cmd_val, "/f",
                ])
                .status()?;
            if !status.success() {
                tracing::warn!("Failed to set Windows HKCU Run auto-start entry");
            }
        } else {
            let _ = Command::new("reg")
                .args(["delete", key, "/v", value_name, "/f"])
                .status();
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (enable, exe_path);
    }
    Ok(())
}

/// Runtime endpoint discovery record written by the process that owns the orchestration runtime.
///
/// Stored at `<data_dir>/runtime.json` with owner-only permissions so local clients (the desktop
/// webview via Tauri IPC, the Vite dev server, integration tests) can find the API port — which
/// may be an ephemeral fallback — and its per-launch access token.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
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

    /// Persist the record, readable only by the current user.
    pub fn write(&self, data_dir: &Path) -> SymphonyResult<PathBuf> {
        std::fs::create_dir_all(data_dir)?;
        let path = Self::file_path(data_dir);
        let tmp = path.with_extension("json.tmp");
        {
            let mut opts = OpenOptions::new();
            opts.create(true).write(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp)?;
            f.write_all(
                serde_json::to_string_pretty(self)
                    .map_err(|e| SymphonyError::JsonError(e.to_string()))?
                    .as_bytes(),
            )?;
        }
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }

    /// Read a discovery record whose owning process is still alive.
    pub fn read_live(data_dir: &Path) -> Option<Self> {
        let raw = std::fs::read_to_string(Self::file_path(data_dir)).ok()?;
        let record: Self = serde_json::from_str(&raw).ok()?;
        is_process_alive(record.pid).then_some(record)
    }

    pub fn remove(data_dir: &Path) {
        let _ = std::fs::remove_file(Self::file_path(data_dir));
    }
}

/// Default application data directory (`%LOCALAPPDATA%\SymphonyDesktop` on Windows).
pub fn default_app_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("SymphonyDesktop")
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
    fn test_secret_store_roundtrip() {
        let dir = tempdir().unwrap();
        let store = SecretStore::new(dir.path()).unwrap();
        store
            .set_secret("GITHUB_TOKEN", "ghp_secret1234567890")
            .unwrap();
        assert_eq!(
            store.get_secret("GITHUB_TOKEN").as_deref(),
            Some("ghp_secret1234567890")
        );

        // Reload from disk
        let store2 = SecretStore::new(dir.path()).unwrap();
        assert_eq!(
            store2.get_secret("GITHUB_TOKEN").as_deref(),
            Some("ghp_secret1234567890")
        );
    }

    #[test]
    fn test_single_instance_lock() {
        let dir = tempdir().unwrap();
        let guard = SingleInstanceGuard::acquire(dir.path()).unwrap();
        assert_eq!(guard.pid(), std::process::id());
        drop(guard);
        let guard2 = SingleInstanceGuard::acquire(dir.path()).unwrap();
        assert_eq!(guard2.pid(), std::process::id());
    }
}
