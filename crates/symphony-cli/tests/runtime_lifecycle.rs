//! Runtime lifecycle of the real `symphony-cli` binary: discovery publication and permissions,
//! authenticated API availability, single-scheduler coordination, crash recovery and log
//! hygiene. Runs on every CI platform, including the native Windows runner (where the discovery
//! file must carry the owner-only DACL).

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use symphony_platform::{is_process_alive, secure_fs, RuntimeDiscovery};

const BIN: &str = env!("CARGO_BIN_EXE_symphony-cli");

fn spawn_runtime(data_dir: &Path) -> Child {
    Command::new(BIN)
        .args(["--port", "0", "--data-dir"])
        .arg(data_dir)
        // Debug logging maximises the chance of catching a leaked token.
        .env("RUST_LOG", "debug")
        .env_remove("SYMPHONY_API_TOKEN")
        .current_dir(data_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn symphony-cli")
}

fn wait_for_discovery(data_dir: &Path, child: &mut Child) -> RuntimeDiscovery {
    let start = Instant::now();
    loop {
        if let Some(found) = RuntimeDiscovery::read_live(data_dir) {
            if found.pid == child.id() {
                return found;
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!("runtime exited early with {status}");
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "runtime.json was not published"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn kill_and_collect_output(mut child: Child) -> String {
    let _ = child.kill();
    let _ = child.wait();
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut out);
    }
    out
}

fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

#[test]
fn runtime_publishes_private_discovery_coordinates_one_scheduler_and_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // 1. Startup: discovery is published with owner-only permissions.
    let mut first = spawn_runtime(&data_dir);
    let record = wait_for_discovery(&data_dir, &mut first);
    assert_eq!(record.owner, "symphony-cli");
    assert_eq!(record.token.len(), 64);
    secure_fs::verify_private_file(&RuntimeDiscovery::file_path(&data_dir))
        .expect("runtime.json must be private to the current user");

    // 2. The authenticated local API answers with the published token only.
    let client = http();
    let state = format!("{}/api/v1/state", record.base_url);
    assert_eq!(
        client
            .get(&state)
            .bearer_auth(&record.token)
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(client.get(&state).send().unwrap().status(), 401);

    // 3. A second runtime for the same data directory refuses to start a scheduler.
    let second = Command::new(BIN)
        .args(["--port", "0", "--data-dir"])
        .arg(&data_dir)
        .current_dir(&data_dir)
        .output()
        .expect("run second symphony-cli");
    assert!(
        !second.status.success(),
        "a second scheduler must not start"
    );
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("already running"),
        "unexpected error: {stderr}"
    );
    // The first runtime is unaffected and still owns the discovery record.
    assert_eq!(
        RuntimeDiscovery::read_live(&data_dir).unwrap().pid,
        first.id()
    );

    // 4. Crash: the OS releases the lock; the stale record is ignored; no orphan remains.
    let first_pid = first.id();
    let output = kill_and_collect_output(first);
    assert!(!is_process_alive(first_pid), "runtime process must be gone");
    assert!(RuntimeDiscovery::read_live(&data_dir).is_none());
    assert!(
        !output.contains(&record.token),
        "the access token must never be logged"
    );

    // 5. Recovery: a restarted runtime takes over with a fresh token; the old one is dead.
    let mut restarted = spawn_runtime(&data_dir);
    let fresh = wait_for_discovery(&data_dir, &mut restarted);
    assert_ne!(fresh.token, record.token);
    let state = format!("{}/api/v1/state", fresh.base_url);
    assert_eq!(
        client
            .get(&state)
            .bearer_auth(&fresh.token)
            .send()
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .get(&state)
            .bearer_auth(&record.token)
            .send()
            .unwrap()
            .status(),
        401
    );
    let restarted_pid = restarted.id();
    let output = kill_and_collect_output(restarted);
    assert!(!is_process_alive(restarted_pid));
    assert!(!output.contains(&fresh.token));

    // 6. Persistent log files (the only record of a windowless desktop build): owner-only, written
    //    by both runs, and free of either token.
    let logs: Vec<_> = std::fs::read_dir(symphony_platform::logging::log_dir(&data_dir))
        .expect("log directory")
        .map(|e| e.unwrap().path())
        .collect();
    // One daily file, or two if the runs straddled local midnight.
    assert!((1..=2).contains(&logs.len()), "daily files: {logs:?}");
    let mut log = String::new();
    for file in &logs {
        secure_fs::verify_private_file(file).expect("the log file must be owner-only");
        log.push_str(&std::fs::read_to_string(file).unwrap());
    }
    assert!(
        log.contains("Symphony"),
        "the runtime logged to the file: {log}"
    );
    assert!(!log.contains(&record.token) && !log.contains(&fresh.token));
}

#[cfg(unix)]
#[test]
fn graceful_shutdown_removes_the_discovery_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_runtime(dir.path());
    wait_for_discovery(dir.path(), &mut child);
    // SAFETY: plain kill(2) on our own child.
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "no graceful exit"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!RuntimeDiscovery::file_path(dir.path()).exists());
}

#[test]
fn unreadable_vault_stops_the_runtime_until_explicitly_quarantined() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("credentials.vault"), b"corrupt").unwrap();
    let out = Command::new(BIN)
        .args(["--port", "0", "--data-dir"])
        .arg(dir.path())
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("credential_vault_error"));
    assert_eq!(
        std::fs::read(dir.path().join("credentials.vault")).unwrap(),
        b"corrupt",
        "the vault must not be replaced silently"
    );

    let mut child = Command::new(BIN)
        .args(["--port", "0", "--quarantine-credential-vault", "--data-dir"])
        .arg(dir.path())
        .current_dir(dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_discovery(dir.path(), &mut child);
    let _ = child.kill();
    let _ = child.wait();
    let moved = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("credentials.vault.corrupt-")
        });
    assert!(moved, "the corrupt vault is kept aside, not deleted");

    // With a readable vault the flag is a no-op: nothing is moved aside.
    symphony_platform::SecretStore::new(dir.path())
        .unwrap()
        .set_secret("GITHUB_TOKEN", "ghp_keep_me")
        .unwrap();
    let before = std::fs::read(dir.path().join("credentials.vault")).ok();
    assert!(before.is_some());
    let mut child = Command::new(BIN)
        .args(["--port", "0", "--quarantine-credential-vault", "--data-dir"])
        .arg(dir.path())
        .current_dir(dir.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for_discovery(dir.path(), &mut child);
    let _ = child.kill();
    let _ = child.wait();
    let quarantined = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("credentials.vault.corrupt-")
        })
        .count();
    assert_eq!(quarantined, 1, "a readable vault is never quarantined");
    assert_eq!(
        std::fs::read(dir.path().join("credentials.vault")).ok(),
        before
    );
}
