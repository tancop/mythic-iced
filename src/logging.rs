//! Logging to stderr plus a rotating log file.
//!
//! Files live in `CACHE_DIR/mythic/logs` as `mythic-<date>-<time>.log`.
//! Only the newest [`MAX_LOG_FILES`] files matching that scheme are kept;
//! anything else (e.g. a renamed file) is left alone. On startup the
//! newest file is additionally dropped when it has zero length, so
//! sessions that logged nothing can't crowd out the real logs.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_LOG_FILES: usize = 10;
const LOG_PREFIX: &str = "mythic-";
const LOG_SUFFIX: &str = ".log";

/// Settings-page toggle. When on, logging behaves like the old
/// `MYTHIC_DEBUG` env var (`RUST_LOG="warn,mythic=debug"`); when off,
/// like a bare `RUST_LOG="error"`. Only meaningful when `RUST_LOG` is
/// unset — an explicit `RUST_LOG` always rules.
static DEBUG_LOGGING: AtomicBool = AtomicBool::new(false);

/// Whether the settings-page debug-logging checkbox is currently on.
pub fn debug_logging_enabled() -> bool {
    DEBUG_LOGGING.load(Ordering::Relaxed)
}

pub fn set_debug_logging(enabled: bool) {
    DEBUG_LOGGING.store(enabled, Ordering::Relaxed);
}

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

/// A session that logged nothing (normal mode without errors) still
/// leaves a zero-byte log file behind. Drop the newest one before
/// rotating so empty files can't eat rotation slots and end up crowding
/// out the real logs. Silent on anything but a failed remove.
fn drop_empty_previous_log(dir: &Path) {
    let newest = fs::read_dir(dir).ok().map(|entries| {
        entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| {
                let name = entry.file_name();
                let stamp = parse_log_filename(name.to_str()?)?;
                Some((stamp, entry.path()))
            })
            .max()
    });
    let Some((_, path)) = newest.unwrap_or(None) else {
        return;
    };
    if fs::metadata(&path).is_ok_and(|meta| meta.len() == 0)
        && let Err(e) = fs::remove_file(&path)
    {
        eprintln!("warning: failed to remove empty log file {path:?}: {e}");
    }
}

fn open_log_file(dir: &Path) -> io::Result<std::fs::File> {
    fs::create_dir_all(dir)?;
    drop_empty_previous_log(dir);
    if let Err(e) = rotate_logs_in(dir) {
        eprintln!("warning: log rotation failed: {e}");
    }
    fern::log_file(dir.join(log_filename(chrono::Local::now())))
}

fn parse_level(s: &str) -> Option<log::LevelFilter> {
    log::LevelFilter::from_str(s).ok()
}

/// Initialize logging. Honors `RUST_LOG` when set; otherwise the
/// `MYTHIC_DEBUG` env var (or the settings-page checkbox at runtime)
/// toggles between the equivalent of `RUST_LOG="warn,mythic=debug"`
/// (debug mode) and `"error"` (normal mode).
pub fn init() {
    let rust_log = std::env::var_os("RUST_LOG").map(|v| v.to_string_lossy().into_owned());

    // `RUST_LOG` overrides `MYTHIC_DEBUG`, as before — so the checkbox
    // only starts checked when debug mode is actually active, i.e.
    // `MYTHIC_DEBUG` without `RUST_LOG`.
    DEBUG_LOGGING.store(
        rust_log.is_none() && std::env::var_os("MYTHIC_DEBUG").is_some(),
        Ordering::Relaxed,
    );

    let filters = {
        match &rust_log {
            // An explicit `RUST_LOG` rules alone, and the checkbox has
            // no effect.
            Some(rust_log) => rust_log.clone(),
            // Union of the normal (`error`) and debug (`warn,mythic=debug`)
            // levels; the fern filter below denies records back down to
            // normal-mode verbosity while the checkbox is off. fern ANDs
            // filters with the level config, so a filter can only make
            // logging *less* verbose — hence building levels at the
            // permissive union and gating through the filter.
            None => "warn,mythic=debug".to_owned(),
        }
    };

    let mut dispatch = fern::Dispatch::new().format(|out, message, record| {
        out.finish(format_args!(
            "[{} {:<5} {}] {}",
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z"),
            record.level(),
            record.target(),
            message
        ))
    });

    let mut has_default_level = false;
    for directive in filters.split(',').map(str::trim).filter(|d| !d.is_empty()) {
        let (target, level) = directive
            .split_once('=')
            .map(|(target, level)| (Some(target.trim()), level.trim()))
            .unwrap_or((None, directive));
        let Some(level) = parse_level(level) else {
            eprintln!("warning: ignoring unparsable log directive {directive:?}");
            continue;
        };
        dispatch = match target {
            Some(target) => dispatch.level_for(target.to_owned(), level),
            None => {
                has_default_level = true;
                dispatch.level(level)
            }
        };
    }
    // An env_logger `RUST_LOG` containing only `crate=level` pairs still
    // defaults unmatched targets to plain errors; fern alone would allow
    // everything, so apply the default here when no bare level was given.
    if !has_default_level {
        dispatch = dispatch.level(log::LevelFilter::Error);
    }

    // The checkbox's MYTHIC_DEBUG behavior, live: pass everything the
    // level config allows when debug mode is on; otherwise deny
    // anything above plain errors. With `RUST_LOG` set the directives
    // rule alone (like `MYTHIC_DEBUG` being ignored before).
    let rust_log_set = rust_log.is_some();
    dispatch = dispatch.filter(move |metadata| {
        rust_log_set || debug_logging_enabled() || metadata.level() <= log::Level::Error
    });

    match open_log_file(&logs_dir()) {
        Ok(file) => dispatch = dispatch.chain(file),
        Err(e) => {
            eprintln!("warning: file logging disabled: {e}");
        }
    }
    dispatch = dispatch.chain(io::stderr());

    let _ = dispatch.apply();
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
    fn drop_empty_previous_log_removes_only_newest_when_empty() {
        let dir =
            std::env::temp_dir().join(format!("mythic-log-test-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Newest file is empty (a session that logged nothing); it is the
        // only thing that should disappear.
        fs::write(dir.join(stamp(2026, 10, 1, 14, 5, 0)), "").unwrap();
        // Real content from an earlier session: kept.
        fs::write(dir.join(stamp(2026, 10, 1, 14, 4, 0)), "real logs").unwrap();
        fs::write(dir.join(stamp(2026, 10, 1, 14, 3, 0)), "").unwrap();

        drop_empty_previous_log(&dir);

        let remaining: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(!remaining.contains(&stamp(2026, 10, 1, 14, 5, 0)));
        assert!(remaining.contains(&stamp(2026, 10, 1, 14, 4, 0)));
        // Only the newest file is ever considered: this empty older one
        // survives.
        assert!(remaining.contains(&stamp(2026, 10, 1, 14, 3, 0)));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn drop_empty_previous_log_keeps_non_empty_recent_file() {
        let dir = std::env::temp_dir().join(format!("mythic-log-test-keep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        fs::write(dir.join(stamp(2026, 10, 1, 14, 5, 0)), "real logs").unwrap();
        fs::write(dir.join(stamp(2026, 10, 1, 14, 4, 0)), "real logs too").unwrap();

        drop_empty_previous_log(&dir);

        let remaining: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(remaining.len(), 2);

        fs::remove_dir_all(&dir).unwrap();
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
