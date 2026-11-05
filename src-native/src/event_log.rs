//! Optional, bounded operational JSON Lines log. No packet or media-content capture.
#![cfg_attr(not(windows), allow(dead_code))]
use serde_json::{json, Value};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, OnceLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: u64 = 5 * 1024 * 1024;
static LOG: OnceLock<Logger> = OnceLock::new();
pub struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        shutdown();
    }
}
struct Logger {
    path: PathBuf,
    enabled: AtomicBool,
    dropped: AtomicU64,
    failed: AtomicBool,
    sender: mpsc::SyncSender<Command>,
}
enum Command {
    Entry(Value),
    Stop(mpsc::SyncSender<()>),
    #[cfg(target_os = "android")]
    Flush(mpsc::SyncSender<bool>),
}

pub fn initialize(path: PathBuf, enable: bool) -> anyhow::Result<()> {
    if enable {
        OpenOptions::new().create(true).append(true).open(&path)?;
    }
    let (sender, receiver) = mpsc::sync_channel(128);
    let destination = path.clone();
    std::thread::Builder::new()
        .name("client-event-log".into())
        .spawn(move || {
            let mut output: Option<File> = None;
            while let Ok(command) = receiver.recv() {
                match command {
                    Command::Entry(entry) => {
                        if write_entry(&destination, &mut output, &entry).is_err() {
                            output = None;
                            if let Some(log) = LOG.get() {
                                log.failed.store(true, Ordering::Relaxed);
                            }
                        }
                    }
                    #[cfg(target_os = "android")]
                    Command::Flush(ack) => {
                        let ok = output.as_mut().is_none_or(|file| file.flush().is_ok());
                        let _ = ack.send(ok);
                    }
                    Command::Stop(ack) => {
                        if let Some(file) = &mut output {
                            let _ = file.flush();
                        }
                        let _ = ack.send(());
                        break;
                    }
                }
            }
        })?;
    let _ = LOG.set(Logger {
        path,
        enabled: AtomicBool::new(enable),
        dropped: AtomicU64::new(0),
        failed: AtomicBool::new(false),
        sender,
    });
    record(
        "INFO",
        "application_started",
        json!({"version":env!("CARGO_PKG_VERSION")}),
    );
    Ok(())
}

pub fn configure(enable: bool) -> anyhow::Result<()> {
    if let Some(log) = LOG.get() {
        if enable {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log.path)?;
            log.enabled.store(true, Ordering::Release);
            log.failed.store(false, Ordering::Relaxed);
            record("INFO", "logging_enabled", json!({}));
        } else {
            record("INFO", "logging_disabled", json!({}));
            log.enabled.store(false, Ordering::Release);
        }
    }
    Ok(())
}

pub fn enabled() -> bool {
    LOG.get().is_some_and(|l| l.enabled.load(Ordering::Acquire))
}

/// Callers supply only explicitly selected metadata, never raw errors/URLs/payloads.
pub fn record(level: &'static str, event: &'static str, details: Value) {
    let Some(log) = LOG.get().filter(|l| l.enabled.load(Ordering::Acquire)) else {
        return;
    };
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let entry = json!({"timestamp":timestamp(millis),"level":level,"event":event,
        "details":details,"dropped_log_events":log.dropped.load(Ordering::Relaxed),
        "previous_write_failed":log.failed.load(Ordering::Relaxed)});
    if log.sender.try_send(Command::Entry(entry)).is_err() {
        log.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

/// Export barrier outside media/UI threads. Keeps the process-wide logger alive.
#[cfg(target_os = "android")]
pub fn flush() -> anyhow::Result<()> {
    let Some(log) = LOG.get() else {
        return Ok(());
    };
    let (tx, rx) = mpsc::sync_channel(1);
    let until = std::time::Instant::now() + Duration::from_secs(2);
    let mut command = Command::Flush(tx);
    loop {
        match log.sender.try_send(command) {
            Ok(()) => break,
            Err(mpsc::TrySendError::Full(c)) if std::time::Instant::now() < until => {
                command = c;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => anyhow::bail!("Diagnostic logger unavailable"),
        }
    }
    anyhow::ensure!(
        rx.recv_timeout(until.saturating_duration_since(std::time::Instant::now()))
            .unwrap_or(false),
        "Diagnostic flush failed"
    );
    Ok(())
}

pub fn shutdown() {
    let Some(log) = LOG.get() else {
        return;
    };
    record("INFO", "application_stopped", json!({}));
    let (tx, rx) = mpsc::sync_channel(1);
    // Shutdown is outside the media path. Never wait indefinitely for disk I/O.
    let until = std::time::Instant::now() + Duration::from_secs(2);
    let mut command = Command::Stop(tx);
    loop {
        match log.sender.try_send(command) {
            Ok(()) => {
                let _ = rx.recv_timeout(until.saturating_duration_since(std::time::Instant::now()));
                break;
            }
            Err(mpsc::TrySendError::Full(c)) if std::time::Instant::now() < until => {
                command = c;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => break,
        }
    }
}

fn write_entry(
    path: &std::path::Path,
    output: &mut Option<File>,
    entry: &Value,
) -> std::io::Result<()> {
    let mut bytes = serde_json::to_vec(entry)?;
    bytes.push(b'\n');
    if output.is_none() {
        *output = Some(OpenOptions::new().create(true).append(true).open(path)?);
    }
    if output.as_ref().unwrap().metadata()?.len() + bytes.len() as u64 > MAX_BYTES {
        *output = None;
        let previous = path.with_extension("previous.jsonl");
        match std::fs::remove_file(&previous) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
        std::fs::rename(path, previous)?;
        *output = Some(OpenOptions::new().create_new(true).write(true).open(path)?);
    }
    let file = output.as_mut().unwrap();
    file.write_all(&bytes)?;
    file.flush()
}

// Gregorian civil date from Unix days; log timestamps are RFC 3339 in UTC.
fn timestamp(ms: u64) -> String {
    let seconds = ms / 1000;
    let days = (seconds / 86400) as i64 + 719468;
    let era = days / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
        ms % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamps_and_rotation_preserve_complete_json_records() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(timestamp(1709164800123), "2024-02-29T00:00:00.123Z");
        let dir = std::env::temp_dir().join(format!(
            "evoparseq-log-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("client-events.jsonl");
        let mut output = None;
        write_entry(&path, &mut output, &json!({"event":"connection_failed"})).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&std::fs::read_to_string(&path).unwrap()).unwrap()
                ["event"],
            "connection_failed"
        );
        // Windows append-only handles cannot truncate or extend a file.
        drop(output.take());
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_BYTES)
            .unwrap();
        write_entry(&path, &mut output, &json!({"event":"decoder_error"})).unwrap();
        drop(output);
        assert!(path.with_extension("previous.jsonl").exists());
        assert_eq!(
            serde_json::from_str::<Value>(&std::fs::read_to_string(&path).unwrap()).unwrap()
                ["event"],
            "decoder_error"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
