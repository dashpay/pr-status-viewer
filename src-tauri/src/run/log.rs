//! A job's output: cleaned lines, the last [`RING_LINES`] in memory, the
//! complete log on disk, and the batches sent to the UI.

use std::collections::VecDeque;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::model::RunPlatform;

/// Lines kept in memory for `run_log`.
pub const RING_LINES: usize = 2000;
/// Longer lines are cut, in memory and in the UI; the file keeps them whole.
pub const MAX_LINE_CHARS: usize = 2000;
/// How often new lines are sent to the UI.
pub const BATCH_INTERVAL: Duration = Duration::from_millis(100);
/// Log files kept in the logs folder; older ones are deleted.
pub const KEEP_LOG_FILES: usize = 10;

const LOG_SUFFIX: &str = ".log";

/// Without terminal escapes (colours, cursor moves) and control characters,
/// cut to [`MAX_LINE_CHARS`].
pub fn clean_line(raw: &str) -> String {
    let stripped = strip_ansi(raw);
    if stripped.chars().count() <= MAX_LINE_CHARS {
        return stripped;
    }
    let mut cut: String = stripped.chars().take(MAX_LINE_CHARS).collect();
    cut.push_str(" [line cut]");
    cut
}

pub fn strip_ansi(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.peek() {
                // CSI: ESC [ params... final byte in @..~
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: ESC ] ... BEL or ESC \
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                // Other escapes: intermediate bytes, then one final byte
                // (e.g. `ESC ( B`).
                Some(_) => {
                    while chars.peek().is_some_and(|c| (' '..='/').contains(c)) {
                        chars.next();
                    }
                    chars.next();
                }
                None => {}
            },
            '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// One job's log while it runs.
pub struct JobLog {
    ring: VecDeque<String>,
    pending: Vec<String>,
    file: Option<BufWriter<fs::File>>,
}

impl JobLog {
    pub fn new(file: Option<fs::File>) -> Self {
        Self {
            ring: VecDeque::new(),
            pending: Vec::new(),
            file: file.map(BufWriter::new),
        }
    }

    pub fn push(&mut self, raw: &str) {
        if let Some(f) = &mut self.file {
            let whole = strip_ansi(raw);
            if writeln!(f, "{whole}").is_err() {
                // A full disk must not end the build.
                self.file = None;
            }
        }
        let line = clean_line(raw);
        if self.ring.len() == RING_LINES {
            self.ring.pop_front();
        }
        self.ring.push_back(line.clone());
        self.pending.push(line);
    }

    /// Lines not yet sent to the UI. Also flushes the file.
    pub fn take_pending(&mut self) -> Vec<String> {
        if let Some(f) = &mut self.file {
            let _ = f.flush();
        }
        std::mem::take(&mut self.pending)
    }

    pub fn lines(&self) -> Vec<String> {
        self.ring.iter().cloned().collect()
    }

    pub fn close(&mut self) {
        if let Some(mut f) = self.file.take() {
            let _ = f.flush();
        }
    }
}

/// `2026-11-22_10-15-03_dashpay-platform-6002_ios.log`: sorts by time.
pub fn log_file_name(at: DateTime<Utc>, pr_id: &str, platform: RunPlatform) -> String {
    let pr: String = pr_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let platform = match platform {
        RunPlatform::Ios => "ios",
        RunPlatform::Android => "android",
    };
    format!(
        "{}_{pr}_{platform}{LOG_SUFFIX}",
        at.format("%Y-%m-%d_%H-%M-%S")
    )
}

/// Deletes the oldest log files so at most `keep` remain. Only touches
/// files named like [`log_file_name`].
pub fn prune_logs(dir: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_log_name)
        })
        .collect();
    logs.sort();
    let excess = logs.len().saturating_sub(keep);
    for old in logs.into_iter().take(excess) {
        if let Err(e) = fs::remove_file(&old) {
            log::debug!("run: could not delete an old log: {e}");
        }
    }
}

/// `YYYY-MM-DD_HH-MM-SS_..._....log`.
fn is_log_name(name: &str) -> bool {
    let b = name.as_bytes();
    name.ends_with(LOG_SUFFIX)
        && b.len() > 20
        && b[..19].iter().enumerate().all(|(i, c)| match i {
            4 | 7 | 13 | 16 => *c == b'-',
            10 => *c == b'_',
            _ => c.is_ascii_digit(),
        })
        && b[19] == b'_'
}

/// Makes room for and creates a new log file in `dir`.
pub fn create_log_file(dir: &Path, name: &str) -> std::io::Result<(PathBuf, fs::File)> {
    fs::create_dir_all(dir)?;
    prune_logs(dir, KEEP_LOG_FILES.saturating_sub(1));
    let path = dir.join(name);
    let file = fs::File::create(&path)?;
    Ok((path, file))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn ansi_and_controls_are_stripped() {
        assert_eq!(
            strip_ansi("\u{1b}[1;32m   Compiling\u{1b}[0m foo v1\u{7}"),
            "   Compiling foo v1"
        );
        assert_eq!(
            strip_ansi("\u{1b}]8;;http://x\u{1b}\\link\u{1b}]8;;\u{7}"),
            "link"
        );
        assert_eq!(strip_ansi("a\tb\u{1b}(Bc"), "a\tbc");
    }

    #[test]
    fn long_lines_are_cut_in_memory_but_whole_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.log");
        let mut log = JobLog::new(Some(fs::File::create(&path).unwrap()));
        let long = "é".repeat(MAX_LINE_CHARS + 10);
        log.push(&long);
        let lines = log.take_pending();
        assert_eq!(
            lines[0].chars().count(),
            MAX_LINE_CHARS + " [line cut]".len()
        );
        log.close();
        assert_eq!(fs::read_to_string(&path).unwrap(), format!("{long}\n"));
    }

    #[test]
    fn ring_keeps_the_last_2000_lines() {
        let mut log = JobLog::new(None);
        for i in 0..2500 {
            log.push(&format!("line {i}"));
        }
        let lines = log.lines();
        assert_eq!(lines.len(), RING_LINES);
        assert_eq!(lines[0], "line 500");
        assert_eq!(lines[RING_LINES - 1], "line 2499");
        // Pending holds everything not yet sent, then nothing.
        assert_eq!(log.take_pending().len(), 2500);
        assert!(log.take_pending().is_empty());
    }

    #[test]
    fn file_names_sort_by_time() {
        let at = Utc.with_ymd_and_hms(2026, 11, 22, 9, 5, 3).unwrap();
        let name = log_file_name(at, "dashpay/platform#6002", RunPlatform::Ios);
        assert_eq!(name, "2026-11-22_09-05-03_dashpay-platform-6002_ios.log");
        assert!(is_log_name(&name));
        assert!(!is_log_name("notes.log"));
        assert!(!is_log_name("2026-11-22_09-05-03_x.txt"));
    }

    #[test]
    fn only_the_newest_ten_logs_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..12 {
            let at = Utc.with_ymd_and_hms(2026, 11, 22, 10, i, 0).unwrap();
            fs::write(
                dir.path()
                    .join(log_file_name(at, "a/b#1", RunPlatform::Android)),
                "",
            )
            .unwrap();
        }
        fs::write(dir.path().join("keep-me.txt"), "").unwrap();
        let at = Utc.with_ymd_and_hms(2026, 11, 22, 11, 0, 0).unwrap();
        let (path, _f) =
            create_log_file(dir.path(), &log_file_name(at, "a/b#1", RunPlatform::Ios)).unwrap();
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), KEEP_LOG_FILES + 1);
        assert!(names.contains(&"keep-me.txt".to_owned()));
        // The three oldest are gone; the new one is there.
        assert_eq!(names[0], "2026-11-22_10-03-00_a-b-1_android.log");
        assert!(path.exists());
    }
}
