//! Persistent, redacted diagnostic logs.
//!
//! The release desktop binary is a Windows GUI-subsystem process: it has no console, so logs
//! written only to stdout are lost. [`init_logging`] keeps the stdout output (CLI, development) and
//! adds daily log files in `<data dir>/logs`:
//!
//! - `<component>-YYYY-MM-DD.log`, created owner-only through [`crate::secure_fs`];
//! - every line passes through [`redact_secrets`] before it reaches the disk;
//! - only the newest [`LOG_RETENTION_FILES`] files of a component are kept;
//! - at most [`MAX_LOG_FILE_BYTES`] per file and day, so a runaway loop cannot fill the disk;
//! - panics are logged (message, location, backtrace) before the default hook runs.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use chrono::{Local, NaiveDate};
use symphony_core::redaction::redact_secrets;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::secure_fs;

/// Number of daily files kept per component.
pub const LOG_RETENTION_FILES: usize = 14;
/// Size limit of one daily file; later entries of that day are dropped.
pub const MAX_LOG_FILE_BYTES: u64 = 50 * 1024 * 1024;
/// Filter used when `RUST_LOG` is not set.
pub const DEFAULT_LOG_FILTER: &str = "info,symphony=debug";

/// Secrets known only at runtime (API tokens, vault values) that pattern-based redaction cannot
/// recognise. Every log line, on stdout and on disk, is redacted against them.
static RUNTIME_SECRETS: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Redact `secret` from every log line written from now on. Values shorter than 6 characters are
/// ignored (as by [`redact_secrets`]), so common words are never masked.
pub fn redact_from_logs(secret: &str) {
    let secret = secret.trim();
    if secret.len() < 6 {
        return;
    }
    if let Ok(mut known) = RUNTIME_SECRETS.write() {
        if !known.iter().any(|k| k == secret) {
            known.push(secret.to_string());
        }
    }
}

/// `line` with pattern-matched and registered secrets replaced by `[REDACTED]`.
fn redact_line(line: &[u8]) -> String {
    let text = String::from_utf8_lossy(line);
    match RUNTIME_SECRETS.read() {
        Ok(known) => redact_secrets(&text, &known),
        Err(_) => redact_secrets(&text, &[]),
    }
}

/// Directory holding the log files of a data directory.
pub fn log_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// Install the global subscriber: stdout plus, when `data_dir` is given, the daily files of
/// `component`. Also installs the panic hook. Returns the log directory, or the reason the files
/// could not be opened (logging then continues on stdout only, and the caller should report it).
pub fn init_logging(data_dir: Option<&Path>, component: &str) -> Result<Option<PathBuf>, String> {
    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| DEFAULT_LOG_FILTER.into());
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_writer(RedactedStdout)
        .with_filter(filter());

    let (file_layer, outcome) = match data_dir {
        None => (None, Ok(None)),
        Some(data_dir) => {
            let dir = log_dir(data_dir);
            match DailyLogFiles::open(&dir, component) {
                Ok(files) => (
                    Some(
                        tracing_subscriber::fmt::layer()
                            .with_ansi(false)
                            .with_writer(files)
                            .with_filter(filter()),
                    ),
                    Ok(Some(dir)),
                ),
                Err(err) => (
                    None,
                    Err(format!(
                        "Could not open the log directory {}: {err}",
                        dir.display()
                    )),
                ),
            }
        }
    };

    // `try_init`: a second initialisation (tests, embedding) keeps the first subscriber.
    let _ = tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .try_init();
    install_panic_hook();
    outcome
}

/// Log panics through `tracing` (so they reach the log files), then run the previous hook.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_string());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(target: "panic", %location, %backtrace, "panic: {payload}");
        previous(info);
    }));
}

/// Daily, owner-only, size-capped and redacted log files (see the module docs).
#[derive(Clone)]
pub struct DailyLogFiles {
    inner: Arc<Mutex<State>>,
}

struct State {
    dir: PathBuf,
    prefix: String,
    day: Option<NaiveDate>,
    file: Option<File>,
    written: u64,
}

impl DailyLogFiles {
    /// Create `dir` if needed, prune old files and open today's file.
    pub fn open(dir: &Path, prefix: &str) -> io::Result<Self> {
        secure_fs::ensure_dir(dir)?;
        let mut state = State {
            dir: dir.to_path_buf(),
            prefix: prefix.to_string(),
            day: None,
            file: None,
            written: 0,
        };
        state.roll_to(Local::now().date_naive())?;
        Ok(Self {
            inner: Arc::new(Mutex::new(state)),
        })
    }

    /// Path of the file for `day`.
    pub fn path_for(dir: &Path, prefix: &str, day: NaiveDate) -> PathBuf {
        dir.join(format!("{prefix}-{}.log", day.format("%Y-%m-%d")))
    }

    fn write_event(&self, buf: &[u8], today: NaiveDate) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        if state.day != Some(today) && state.roll_to(today).is_err() {
            // Keep the process running; stdout still gets the event.
            state.file = None;
        }
        let redacted = redact_line(buf);
        state.append(redacted.as_bytes());
    }
}

impl State {
    fn roll_to(&mut self, day: NaiveDate) -> io::Result<()> {
        self.day = Some(day);
        self.file = None;
        self.written = 0;
        let path = DailyLogFiles::path_for(&self.dir, &self.prefix, day);
        let file = if path.exists() {
            // Never append to a file someone else could read (or swapped in).
            secure_fs::verify_private_file(&path)?;
            OpenOptions::new().append(true).open(&path)?
        } else {
            // Created owner-only, then reopened for appending like an existing file: another
            // process (e.g. a second desktop instance before it exits) may append concurrently.
            drop(secure_fs::create_private_file(&path)?);
            OpenOptions::new().append(true).open(&path)?
        };
        self.written = file.metadata().map(|m| m.len()).unwrap_or(0);
        self.file = Some(file);
        prune(&self.dir, &self.prefix, LOG_RETENTION_FILES);
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if self.written >= MAX_LOG_FILE_BYTES {
            return;
        }
        let remaining = MAX_LOG_FILE_BYTES - self.written;
        let result = if (bytes.len() as u64) < remaining {
            file.write_all(bytes).map(|_| bytes.len() as u64)
        } else {
            // The notice is cut to the remaining budget, so the file never exceeds the cap; the
            // file is then full and nothing else is written today.
            let notice = b"[log size limit reached; further entries for today are dropped]\n";
            let fitting = &notice[..notice.len().min(remaining as usize)];
            file.write_all(fitting).map(|_| remaining)
        };
        match result {
            Ok(n) => self.written += n,
            // A failing disk must not take the runtime down; stop writing to this file.
            Err(_) => self.file = None,
        }
    }
}

/// Delete all but the newest `keep` files of `prefix` (names sort chronologically).
fn prune(dir: &Path, prefix: &str, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| is_daily_log_name(n, prefix))
        })
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(keep);
    for old in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(old);
    }
}

fn is_daily_log_name(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('-'))
        .and_then(|rest| rest.strip_suffix(".log"))
        .is_some_and(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok())
}

/// Stdout with the same redaction as the files (service managers often persist stdout).
struct RedactedStdout;

impl Write for RedactedStdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::stdout().write_all(redact_line(buf).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }
}

impl<'a> MakeWriter<'a> for RedactedStdout {
    type Writer = RedactedStdout;

    fn make_writer(&'a self) -> Self::Writer {
        RedactedStdout
    }
}

/// Writer handed to `tracing_subscriber` for one event.
pub struct DailyLogWriter<'a> {
    files: &'a DailyLogFiles,
}

impl Write for DailyLogWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // The fmt layer writes each formatted event with a single `write_all`, so redaction sees
        // whole lines.
        self.files.write_event(buf, Local::now().date_naive());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for DailyLogFiles {
    type Writer = DailyLogWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        DailyLogWriter { files: self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn lines_are_redacted_and_the_file_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        let files = DailyLogFiles::open(&logs, "desktop").unwrap();
        let token = format!("ghp_{}", "a".repeat(36));
        files.write_event(
            format!("calling GitHub with {token}\n").as_bytes(),
            Local::now().date_naive(),
        );
        let path = DailyLogFiles::path_for(&logs, "desktop", Local::now().date_naive());
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("calling GitHub with [REDACTED]"),
            "{written}"
        );
        assert!(!written.contains(&token));
        secure_fs::verify_private_file(&path).unwrap();
    }

    #[test]
    fn registered_runtime_secrets_are_redacted() {
        let dir = tempfile::tempdir().unwrap();
        let files = DailyLogFiles::open(dir.path(), "cli").unwrap();
        // A 64-hex API token matches no pattern; only registration can catch it.
        let token = "4f1c".repeat(16);
        redact_from_logs(&token);
        redact_from_logs("abc"); // too short to register
        let today = Local::now().date_naive();
        files.write_event(format!("token={token} abc\n").as_bytes(), today);
        let written =
            std::fs::read_to_string(DailyLogFiles::path_for(dir.path(), "cli", today)).unwrap();
        assert_eq!(written, "token=[REDACTED] abc\n");
    }

    #[test]
    fn a_new_day_rolls_over_and_old_files_are_pruned() {
        let dir = tempfile::tempdir().unwrap();
        for d in 1..=20 {
            let path = DailyLogFiles::path_for(dir.path(), "cli", day(&format!("2026-01-{d:02}")));
            std::fs::write(path, "old\n").unwrap();
        }
        std::fs::write(dir.path().join("cli-notes.txt"), "kept").unwrap();
        std::fs::write(dir.path().join("desktop-2026-01-01.log"), "kept").unwrap();

        let files = DailyLogFiles::open(dir.path(), "cli").unwrap();
        files.write_event(b"next day\n", day("2099-01-01"));

        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.starts_with("cli-2"))
            .collect();
        names.sort();
        assert_eq!(names.len(), LOG_RETENTION_FILES);
        assert_eq!(names.last().unwrap(), "cli-2099-01-01.log");
        assert!(!names.contains(&"cli-2026-01-01.log".to_string()));
        // Other components and unrelated files are never touched.
        assert!(dir.path().join("cli-notes.txt").exists());
        assert!(dir.path().join("desktop-2026-01-01.log").exists());
    }

    #[test]
    fn a_file_stops_growing_at_the_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let files = DailyLogFiles::open(dir.path(), "cli").unwrap();
        let today = Local::now().date_naive();
        files.inner.lock().unwrap().written = MAX_LOG_FILE_BYTES - 10;
        files.write_event(b"this line does not fit\n", today);
        files.write_event(b"neither does this one\n", today);
        let content =
            std::fs::read_to_string(DailyLogFiles::path_for(dir.path(), "cli", today)).unwrap();
        // Only the 10 bytes left in the budget are used, even by the notice.
        assert_eq!(content, "[log size ");
        assert_eq!(files.inner.lock().unwrap().written, MAX_LOG_FILE_BYTES);
    }

    #[cfg(unix)]
    #[test]
    fn a_log_file_readable_by_others_is_not_appended_to() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = DailyLogFiles::path_for(dir.path(), "cli", Local::now().date_naive());
        std::fs::write(&path, "planted\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(DailyLogFiles::open(dir.path(), "cli").is_err());
    }
}
