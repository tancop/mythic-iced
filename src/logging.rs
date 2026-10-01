//! Logging to stderr plus a rotating log file.
//!
//! Files live in `CACHE_DIR/mythic/logs` as `mythic-<date>-<time>.log`.
//! Only the newest [`MAX_LOG_FILES`] files matching that scheme are kept;
//! anything else (e.g. a renamed file) is left alone.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const MAX_LOG_FILES: usize = 10;
const LOG_PREFIX: &str = "mythic-";
const LOG_SUFFIX: &str = ".log";

pub fn logs_dir() -> PathBuf {
    dirs::cache_dir().unwrap().join("mythic").join("logs")
}

/// `mythic-YYYY-MM-DD-HHMMSS.log`; no colons so it is safe on Windows.
fn log_filename(now: chrono::DateTime<chrono::Local>) -> String {
    format!("{LOG_PREFIX}{}.log", now.format("%Y-%m-%d-%H%M%S"))
}

/// Parse our own naming scheme back to a sortable timestamp. Returns `None`
/// for anything else, which is therefore never rotated out.
fn parse_log_filename(name: &str) -> Option<chrono::NaiveDateTime> {
    let stem = name.strip_prefix(LOG_PREFIX)?.strip_suffix(LOG_SUFFIX)?;
    chrono::NaiveDateTime::parse_from_str(stem, "%Y-%m-%d-%H%M%S").ok()
}

/// Delete the oldest scheme-matching files until fewer than
/// [`MAX_LOG_FILES`] remain (room is made for the new file next).
fn rotate_logs_in(dir: &Path) -> io::Result<()> {
    let mut ours: Vec<(chrono::NaiveDateTime, PathBuf)> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let name = entry.file_name();
            let stamp = parse_log_filename(name.to_str()?)?;
            Some((stamp, entry.path()))
        })
        .collect();
    ours.sort();

    while ours.len() >= MAX_LOG_FILES {
        let (_, oldest) = ours.remove(0);
        if let Err(e) = fs::remove_file(&oldest) {
            eprintln!("warning: failed to rotate out old log file {oldest:?}: {e}");
            break;
        }
    }
    Ok(())
}

fn create_log_file(dir: &Path) -> io::Result<File> {
    fs::create_dir_all(dir)?;
    if let Err(e) = rotate_logs_in(dir) {
        eprintln!("warning: log rotation failed: {e}");
    }
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(log_filename(chrono::Local::now())))
}

/// Duplicates every record to stderr and the log file.
struct Tee {
    file: File,
    stderr: io::Stderr,
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write_all(buf)?;
        self.stderr.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.stderr.flush()
    }
}

/// Initialize logging. Honors `RUST_LOG` when set; otherwise the
/// `MYTHIC_DEBUG` env var selects the equivalent of
/// `RUST_LOG="warn,mythic=debug"`.
pub fn init() {
    let mut builder = env_logger::Builder::from_default_env();
    if std::env::var_os("RUST_LOG").is_none() && std::env::var_os("MYTHIC_DEBUG").is_some() {
        builder.parse_filters("warn,mythic=debug");
    }

    match create_log_file(&logs_dir()) {
        Ok(file) => {
            builder.target(env_logger::Target::Pipe(Box::new(Tee {
                file,
                stderr: io::stderr(),
            })));
        }
        Err(e) => {
            eprintln!("warning: file logging disabled: {e}");
        }
    }

    let _ = builder.try_init();
    log::info!("logging to {}", logs_dir().display());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(year: u32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> String {
        format!("mythic-{year:04}-{month:02}-{day:02}-{hour:02}{min:02}{sec:02}.log")
    }

    #[test]
    fn filename_round_trips_through_matcher() {
        let name = log_filename(
            chrono::Local::now()
                .date_naive()
                .and_hms_opt(1, 2, 3)
                .unwrap()
                .and_local_timezone(chrono::Local)
                .unwrap(),
        );
        assert!(parse_log_filename(&name).is_some());
        assert_eq!(
            parse_log_filename(&stamp(2026, 10, 1, 14, 30, 22)).map(|dt| dt.to_string()),
            Some("2026-10-01 14:30:22".to_string())
        );
    }

    #[test]
    fn foreign_files_never_match() {
        assert_eq!(parse_log_filename("mythic-backup.log"), None);
        assert_eq!(parse_log_filename("mythic-2026-10-01-143022.txt"), None);
        assert_eq!(parse_log_filename("other.log"), None);
        assert_eq!(parse_log_filename("mythic-.log"), None);
        assert_eq!(parse_log_filename("mythic-2026-13-99-999999.log"), None);
    }

    #[test]
    fn rotation_makes_room_and_preserves_foreign_files() {
        let dir = std::env::temp_dir().join(format!("mythic-log-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // 12 matching files, one per minute.
        for minute in 0..12 {
            fs::write(dir.join(stamp(2026, 10, 1, 14, minute, 0)), "x").unwrap();
        }
        // Must be preserved: unparsable-but-similar name and unrelated file.
        fs::write(dir.join("mythic-keep-me.log"), "x").unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();

        rotate_logs_in(&dir).unwrap();

        let mut remaining: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        remaining.sort();

        // The three oldest scheme-matching files are gone, leaving room
        // for the new log file; the rest stay.
        assert!(!remaining.contains(&stamp(2026, 10, 1, 14, 0, 0)));
        assert!(!remaining.contains(&stamp(2026, 10, 1, 14, 1, 0)));
        assert!(!remaining.contains(&stamp(2026, 10, 1, 14, 2, 0)));
        assert!(remaining.contains(&stamp(2026, 10, 1, 14, 3, 0)));
        assert!(remaining.contains(&stamp(2026, 10, 1, 14, 11, 0)));
        assert!(remaining.contains(&"mythic-keep-me.log".to_string()));
        assert!(remaining.contains(&"notes.txt".to_string()));
        assert_eq!(
            remaining
                .iter()
                .filter(|name| parse_log_filename(name).is_some())
                .count(),
            9
        );

        fs::remove_dir_all(&dir).unwrap();
    }
}
