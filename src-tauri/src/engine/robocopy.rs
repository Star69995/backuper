//! robocopy-backed engine. Progress is read by tailing robocopy's UTF-16 log (/UNILOG),
//! which keeps non-ASCII (e.g. Hebrew) file names intact, unlike its OEM-codepage stdout.

use super::{CopyEngine, CopyJob, CopyResult, CopyStats, EngineEvent, Filters, ListedFile, Outcome};
use std::ffi::OsString;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MAX_ERRORS: usize = 50;
/// Folders that are never worth backing up (relevant when the source is a drive root).
const ALWAYS_EXCLUDED_DIRS: [&str; 2] = ["$RECYCLE.BIN", "System Volume Information"];

pub struct Robocopy;

/// robocopy mis-parses a quoted path that ends in a backslash, so trim it (except "D:\").
fn arg_path(p: &Path) -> OsString {
    let s = p.to_string_lossy();
    let trimmed = s.trim_end_matches('\\');
    if trimmed.ends_with(':') {
        format!("{trimmed}\\").into()
    } else {
        trimmed.into()
    }
}

struct Args<'a> {
    source: &'a Path,
    target: &'a Path,
    /// false = plain recursive copy/list (no purge).
    mirror: bool,
    copy_empty_dirs: bool,
    filters: &'a Filters,
    list_only: bool,
}

fn build_args(x: &Args, log: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec![arg_path(x.source), arg_path(x.target)];
    let flags: &[&str] = match (x.mirror, x.copy_empty_dirs) {
        (true, true) => &["/MIR"],
        (true, false) => &["/S", "/PURGE"],
        (false, true) => &["/E"],
        (false, false) => &["/S"],
    };
    a.extend(flags.iter().map(OsString::from));
    for f in [
        "/COPY:DAT",
        "/DCOPY:DAT",
        "/R:2",
        "/W:3",
        "/XJ",
        "/FFT",
        "/NP",
        "/NDL",
        "/FP",
        "/BYTES",
    ] {
        a.push(f.into());
    }
    if x.list_only {
        a.push("/L".into());
    }
    let f = x.filters;
    if let Some(max) = f.max_size {
        a.push(format!("/MAX:{max}").into());
    }
    if let Some(days) = f.max_age_days {
        a.push(format!("/MAXAGE:{days}").into());
    }
    let attrs = match (f.exclude_hidden, f.exclude_system) {
        (true, true) => Some("/XA:HS"),
        (true, false) => Some("/XA:H"),
        (false, true) => Some("/XA:S"),
        (false, false) => None,
    };
    a.extend(attrs.map(OsString::from));
    let files: Vec<&String> = f.exclude_files.iter().filter(|s| !s.trim().is_empty()).collect();
    if !files.is_empty() {
        a.push("/XF".into());
        a.extend(files.iter().map(|s| OsString::from(s.trim())));
    }
    a.push("/XD".into());
    a.extend(ALWAYS_EXCLUDED_DIRS.iter().map(OsString::from));
    a.extend(
        f.exclude_dirs
            .iter()
            .filter(|s| !s.trim().is_empty())
            .map(|s| arg_path(Path::new(s.trim()))),
    );
    let mut unilog = OsString::from("/UNILOG:");
    unilog.push(log.as_os_str());
    a.push(unilog);
    a
}

/// Incrementally decodes a UTF-16LE file that another process is still writing.
struct LogTail {
    path: PathBuf,
    pos: u64,
    pending_byte: Option<u8>,
    partial_line: String,
}

impl LogTail {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            pos: 0,
            pending_byte: None,
            partial_line: String::new(),
        }
    }

    fn poll(&mut self, on_line: &mut dyn FnMut(&str)) {
        let Ok(mut f) = File::open(&self.path) else { return };
        if f.seek(SeekFrom::Start(self.pos)).is_err() {
            return;
        }
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_err() || buf.is_empty() {
            return;
        }
        self.pos += buf.len() as u64;
        if let Some(b) = self.pending_byte.take() {
            buf.insert(0, b);
        }
        if buf.len() % 2 == 1 {
            self.pending_byte = buf.pop();
        }
        let units: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        self.partial_line.push_str(&String::from_utf16_lossy(&units));
        while let Some(i) = self.partial_line.find('\n') {
            let line: String = self.partial_line.drain(..=i).collect();
            on_line(line.trim_start_matches('\u{feff}').trim_end_matches(['\r', '\n']));
        }
    }

    fn finish(&mut self, on_line: &mut dyn FnMut(&str)) {
        self.poll(on_line);
        if !self.partial_line.is_empty() {
            let rest = std::mem::take(&mut self.partial_line);
            on_line(rest.trim_end());
        }
    }
}

/// Runs robocopy to completion. Ok(None) = cancelled.
fn run_process(args: &[OsString], log: &Path, cancel: &AtomicBool, on_line: &mut dyn FnMut(&str)) -> Result<Option<i32>, String> {
    let _ = std::fs::remove_file(log);
    let mut child = Command::new("robocopy")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("לא ניתן להפעיל את robocopy: {e}"))?;
    let mut tail = LogTail::new(log);
    loop {
        tail.poll(on_line);
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            tail.finish(on_line);
            return Ok(None);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                tail.finish(on_line);
                return Ok(status.code());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(150)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

enum Line {
    Copy { size: u64, path: String },
    Extra,
    Other,
}

/// File lines look like "\t    New File  \t\t    1234\tC:\full\path" (with /FP /BYTES /NP).
fn classify(line: &str) -> Line {
    let parts: Vec<&str> = line.split('\t').map(str::trim).filter(|p| !p.is_empty()).collect();
    if parts.len() < 3 {
        return Line::Other;
    }
    let path = parts[parts.len() - 1];
    let looks_like_path = path.get(1..3) == Some(":\\") || path.starts_with("\\\\");
    let Ok(size) = parts[parts.len() - 2].parse::<u64>() else {
        return Line::Other;
    };
    if !looks_like_path {
        return Line::Other;
    }
    if parts[0].contains("EXTRA") {
        Line::Extra
    } else {
        Line::Copy {
            size,
            path: path.to_string(),
        }
    }
}

/// Summary rows ("Files :  10  4  6  0  0  2"): the 6 numbers after the label, in table order.
fn summary_row(line: &str) -> Option<[u64; 6]> {
    let (_, rest) = line.split_once(':')?;
    let nums: Vec<u64> = rest.split_whitespace().map_while(|t| t.parse().ok()).collect();
    (nums.len() == 6 && rest.split_whitespace().count() == 6).then(|| nums.try_into().unwrap())
}

#[derive(Default)]
struct Summary {
    rows: Vec<[u64; 6]>,
}

impl Summary {
    fn feed(&mut self, line: &str) {
        if let Some(r) = summary_row(line) {
            self.rows.push(r);
        }
    }
    /// Rows are Dirs, Files, Bytes; columns Total, Copied, Skipped, Mismatch, FAILED, Extras.
    fn files(&self) -> Option<[u64; 6]> {
        self.rows.get(self.rows.len().checked_sub(3)? + 1).copied()
    }
    fn bytes(&self) -> Option<[u64; 6]> {
        self.rows.last().copied()
    }
}

fn is_error_line(line: &str) -> bool {
    line.contains(" ERROR ") && line.contains("(0x")
}

fn mirror_args(job: &CopyJob, list_only: bool) -> Args<'_> {
    Args {
        source: &job.source,
        target: &job.target,
        mirror: true,
        copy_empty_dirs: job.copy_empty_dirs,
        filters: &job.filters,
        list_only,
    }
}

impl CopyEngine for Robocopy {
    fn list_files(
        &self,
        source: &Path,
        filters: &Filters,
        log_file: &Path,
        cancel: &AtomicBool,
    ) -> Result<Option<Vec<ListedFile>>, String> {
        // Listing against a target that doesn't exist reports every file that passes the filters.
        let nowhere = std::env::temp_dir().join(format!("backuper-list-{}", uuid::Uuid::new_v4()));
        let args = Args {
            source,
            target: &nowhere,
            mirror: false,
            copy_empty_dirs: false,
            filters,
            list_only: true,
        };
        let root = arg_path(source).to_string_lossy().trim_end_matches('\\').to_string() + "\\";
        let mut files = Vec::new();
        let code = run_process(&build_args(&args, log_file), log_file, cancel, &mut |l| {
            if let Line::Copy { size, path } = classify(l) {
                let under_root = path.is_char_boundary(root.len()) && path[..root.len()].eq_ignore_ascii_case(&root);
                if under_root {
                    files.push(ListedFile {
                        rel: path[root.len()..].to_string(),
                        size,
                    });
                }
            }
        });
        let _ = std::fs::remove_file(log_file);
        match code? {
            None => Ok(None),
            Some(c) if c >= 8 => Err("לא ניתן לסרוק את תיקיית המקור".into()),
            Some(_) => Ok(Some(files)),
        }
    }

    fn mirror(&self, job: &CopyJob, cancel: &AtomicBool, on_event: &mut dyn FnMut(EngineEvent)) -> CopyResult {
        let fail = |message: String| CopyResult {
            outcome: Outcome::Failed,
            message,
            stats: CopyStats::default(),
        };

        // Pass 1: list-only scan, so progress can show a real percentage.
        let scan_log = job.log_file.with_extension("scan.log");
        let mut scan = Summary::default();
        match run_process(&build_args(&mirror_args(job, true), &scan_log), &scan_log, cancel, &mut |l| {
            scan.feed(l)
        }) {
            Ok(None) => {
                let _ = std::fs::remove_file(&scan_log);
                return CopyResult {
                    outcome: Outcome::Cancelled,
                    message: "בוטל".into(),
                    stats: CopyStats::default(),
                };
            }
            Err(e) => return fail(e),
            Ok(Some(_)) => {}
        }
        let _ = std::fs::remove_file(&scan_log);
        if let (Some(f), Some(b)) = (scan.files(), scan.bytes()) {
            on_event(EngineEvent::Totals {
                files: f[1],
                bytes: b[1],
            });
        }

        // Pass 2: the real copy.
        let mut stats = CopyStats::default();
        let mut summary = Summary::default();
        let mut last_error: Option<String> = None;
        let code = run_process(
            &build_args(&mirror_args(job, false), &job.log_file),
            &job.log_file,
            cancel,
            &mut |l| {
                summary.feed(l);
                if is_error_line(l) {
                    last_error = Some(l.to_string());
                    return;
                }
                if let Some(err) = last_error.take() {
                    // The line after an ERROR line is the human-readable reason.
                    let path = err.split(")").skip(1).collect::<Vec<_>>().join(")").trim().to_string();
                    let entry = format!("{path} - {}", l.trim());
                    if stats.errors.len() < MAX_ERRORS && !stats.errors.contains(&entry) {
                        stats.errors.push(entry);
                    }
                }
                match classify(l) {
                    Line::Copy { size, path } => on_event(EngineEvent::File { path, size }),
                    Line::Extra | Line::Other => {}
                }
            },
        );

        let code = match code {
            Err(e) => return fail(e),
            Ok(None) => {
                return CopyResult {
                    outcome: Outcome::Cancelled,
                    message: "הגיבוי בוטל על ידי המשתמש".into(),
                    stats,
                }
            }
            Ok(Some(c)) => c,
        };
        stats.exit_code = Some(code);
        if let Some(f) = summary.files() {
            stats.files_copied = f[1];
            stats.files_failed = f[4];
            stats.files_deleted = f[5];
        }
        if let Some(b) = summary.bytes() {
            stats.bytes_copied = b[1];
        }

        // robocopy exit codes are bit flags: 1 copied, 2 extras, 4 mismatch, 8 failures, 16 fatal.
        let (outcome, message) = if code >= 16 {
            (Outcome::Failed, "שגיאה חמורה - ייתכן שהמקור או היעד אינם נגישים".to_string())
        } else if code >= 8 {
            (Outcome::Failed, format!("{} קבצים לא הועתקו", stats.files_failed.max(1)))
        } else if code & 4 != 0 {
            (Outcome::Warning, "הגיבוי הסתיים, אך נמצאו פריטים לא תואמים".to_string())
        } else if code & 1 != 0 {
            (Outcome::Success, format!("הועתקו {} קבצים", stats.files_copied))
        } else {
            (Outcome::Success, "אין שינויים - הגיבוי מעודכן".to_string())
        };
        CopyResult { outcome, message, stats }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_file_lines() {
        match classify("\t    New File  \t\t    1234\tC:\\src\\קובץ.txt") {
            Line::Copy { size, path } => {
                assert_eq!(size, 1234);
                assert_eq!(path, "C:\\src\\קובץ.txt");
            }
            _ => panic!("expected copy line"),
        }
        assert!(matches!(
            classify("\t*EXTRA File \t\t      10\tD:\\dst\\old.txt"),
            Line::Extra
        ));
        assert!(matches!(classify("   Source : C:\\src\\"), Line::Other));
    }

    #[test]
    fn parses_summary() {
        let mut s = Summary::default();
        for l in [
            "               Total    Copied   Skipped  Mismatch    FAILED    Extras",
            "    Dirs :         3         1         2         0         0         0",
            "   Files :        10         4         6         0         0         2",
            "   Bytes :      5000      2000      3000         0         0       100",
            "   Times :   0:00:01   0:00:00                       0:00:00   0:00:00",
        ] {
            s.feed(l);
        }
        assert_eq!(s.files().unwrap()[1], 4);
        assert_eq!(s.bytes().unwrap()[1], 2000);
    }

    #[test]
    fn trims_trailing_backslash() {
        assert_eq!(arg_path(Path::new("D:\\")), OsString::from("D:\\"));
        assert_eq!(arg_path(Path::new("D:\\My Files\\")), OsString::from("D:\\My Files"));
    }
}
