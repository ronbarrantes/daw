//! A short log of what went wrong, and a few milestones (the app starting,
//! the audio device it opened), for Help > Diagnostics and bug reports.
//! The last lines are kept in memory; the desktop app also appends them to
//! a small file, cut back when it grows past `MAX_FILE` bytes. UI thread
//! only: the audio engine never logs (it reports through counters).

use std::collections::VecDeque;
use std::sync::Mutex;

/// Lines kept in memory.
const KEEP: usize = 200;
/// The file is trimmed to its newer half past this size.
#[cfg(not(any(target_arch = "wasm32", test)))]
const MAX_FILE: u64 = 256 * 1024;

static LINES: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// Adds a line, with the time.
pub fn add(message: &str) {
    let line = format!("{}  {message}", now());
    // (Tests leave the real file alone.)
    #[cfg(not(any(target_arch = "wasm32", test)))]
    append_to_file(&line);
    if let Ok(mut lines) = LINES.lock() {
        if lines.len() >= KEEP {
            lines.pop_front();
        }
        lines.push_back(line);
    }
}

/// The lines in memory, oldest first.
pub fn lines() -> Vec<String> {
    LINES
        .lock()
        .map(|l| l.iter().cloned().collect())
        .unwrap_or_default()
}

/// Where the desktop app keeps the log file.
#[cfg(not(target_arch = "wasm32"))]
pub fn path() -> Option<std::path::PathBuf> {
    Some(dirs::config_dir()?.join("daw").join("daw.log"))
}

/// Appends a line to the file; failing to is harmless.
#[cfg(not(any(target_arch = "wasm32", test)))]
fn append_to_file(line: &str) {
    use std::io::Write;
    let Some(path) = path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let big = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_FILE);
    if big {
        if let Ok(text) = std::fs::read_to_string(&path) {
            let keep = &text[text.len() / 2..];
            // Start at a whole line.
            let keep = keep.find('\n').map_or("", |i| &keep[i + 1..]);
            let _ = std::fs::write(&path, keep);
        }
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{line}");
    }
}

/// The time now, as "2026-10-29 13:45:07" (UTC).
#[cfg(not(target_arch = "wasm32"))]
fn now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    stamp(secs)
}

#[cfg(target_arch = "wasm32")]
fn now() -> String {
    stamp((js_sys::Date::now() / 1000.0) as u64)
}

/// Seconds since 1970 as a UTC date and time.
fn stamp(secs: u64) -> String {
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    // Days to a date (Howard Hinnant's civil_from_days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_read_as_dates() {
        assert_eq!(stamp(0), "1970-01-01 00:00:00");
        assert_eq!(stamp(1_709_210_096), "2024-02-29 12:34:56", "a leap day");
        assert_eq!(stamp(1_793_282_400), "2026-10-29 14:00:00");
    }

    #[test]
    fn keeps_the_latest_lines() {
        for i in 0..KEEP + 5 {
            add(&format!("line {i}"));
        }
        let lines = lines();
        assert_eq!(lines.len(), KEEP);
        assert!(lines
            .last()
            .unwrap()
            .ends_with(&format!("line {}", KEEP + 4)));
    }
}
