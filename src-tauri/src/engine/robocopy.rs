//! robocopy-backed engine. Progress is read by tailing robocopy's UTF-16 log (/UNILOG),
//! which keeps non-ASCII (e.g. Hebrew) file names intact, unlike its OEM-codepage stdout.

use super::native;
use super::{
    is_disk_full, CopyEngine, CopyJob, CopyResult, CopyStats, EngineEvent, Filters, Outcome, TotalsSource, ALWAYS_EXCLUDED_DIRS,
    DISK_FULL_MESSAGE,
};
use crate::model::FileError;
use std::cell::{Cell, RefCell};
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
/// Errors that mean the destination or the source drive itself is in trouble (full, gone), not one
/// file: a copy with one of these is not a usable backup.
const DRIVE_ERRORS: [u32; 10] = [39, 112, 21, 53, 55, 59, 64, 67, 1167, 1231];

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
    copy_empty_dirs: bool,
    filters: &'a Filters,
    list_only: bool,
}

fn build_args(x: &Args, log: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec![arg_path(x.source), arg_path(x.target)];
    // File specs right after the paths limit the copy (and the purge) to matching names.
    a.extend(
        x.filters
            .include_files
            .iter()
            .filter(|s| !s.trim().is_empty())
            .map(|s| OsString::from(s.trim())),
    );
    let flags: &[&str] = if x.copy_empty_dirs { &["/MIR"] } else { &["/S", "/PURGE"] };
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

enum Exit {
    Code(i32),
    Cancelled,
    /// `abort` said stop (the destination is full): robocopy was killed.
    Aborted,
}

/// Where robocopy's own console output goes. With /UNILOG the log holds everything it does, but a command line
/// it rejects (exit 16, "ERROR : Invalid Parameter #57") never gets that far, so this is the only place that says why.
fn stdout_path(log: &Path) -> PathBuf {
    log.with_extension("stdout")
}

/// The "ERROR ..." lines of robocopy's console output (it is in the OEM code page, hence the lossy decoding;
/// the parameter and the reason are plain ASCII, only a Hebrew name inside it may come out garbled).
fn read_stdout_errors(log: &Path) -> Vec<String> {
    let text = std::fs::read(stdout_path(log)).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    let _ = std::fs::remove_file(stdout_path(log));
    text.lines().map(str::trim).filter(|l| l.starts_with("ERROR")).map(str::to_string).collect()
}

/// Runs robocopy to completion. `on_tick` runs every poll (~150 ms), also while robocopy writes
/// nothing (e.g. in the middle of one huge file). `abort` is checked after each poll.
fn run_process(
    args: &[OsString],
    log: &Path,
    cancel: &AtomicBool,
    abort: &dyn Fn() -> bool,
    on_line: &mut dyn FnMut(&str),
    on_tick: &mut dyn FnMut(),
) -> Result<Exit, String> {
    let _ = std::fs::remove_file(log);
    let stdout = File::create(stdout_path(log)).map_or_else(|_| Stdio::null(), Stdio::from);
    let mut child = Command::new("robocopy")
        .args(args)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("לא ניתן להפעיל את robocopy: {e}"))?;
    let mut tail = LogTail::new(log);
    loop {
        tail.poll(on_line);
        on_tick();
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            tail.finish(on_line);
            return Ok(Exit::Cancelled);
        }
        if abort() {
            let _ = child.kill();
            let _ = child.wait();
            tail.finish(on_line);
            return Ok(Exit::Aborted);
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                tail.finish(on_line);
                on_tick();
                return Ok(Exit::Code(status.code().unwrap_or(-1)));
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

/// "2026/10/01 03:00:00 ERROR 32 (0x00000020) Copying File C:\src\a.db" + the next line (the
/// reason) -> the Win32 code, the bare path (without robocopy's action words) and the reason.
fn parse_error(line: &str, reason: &str) -> FileError {
    let after = line.split_once(" ERROR ").map_or(line, |(_, r)| r);
    let code = after.split_whitespace().next().and_then(|c| c.parse().ok());
    let rest = after.split_once(')').map_or("", |(_, r)| r).trim();
    let path = rest
        .find(":\\")
        .and_then(|i| i.checked_sub(1))
        .or_else(|| rest.find("\\\\"))
        .map_or(rest, |i| &rest[i..]);
    FileError {
        path: path.to_string(),
        code,
        message: reason.trim().to_string(),
        debug: String::new(),
    }
}

/// The arguments as a command line, for the debug text (quotes the ones with spaces).
fn command_line(args: &[OsString]) -> String {
    args.iter()
        .map(|a| {
            let s = a.to_string_lossy();
            if s.contains(' ') { format!("\"{s}\"") } else { s.into_owned() }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn mirror_args(job: &CopyJob, list_only: bool) -> Args<'_> {
    Args {
        source: &job.source,
        target: &job.target,
        copy_empty_dirs: job.copy_empty_dirs,
        filters: &job.filters,
        list_only,
    }
}

impl CopyEngine for Robocopy {
    fn mirror(&self, job: &CopyJob, cancel: &AtomicBool, on_event: &mut dyn FnMut(EngineEvent)) -> CopyResult {
        let fail = |message: String| CopyResult {
            outcome: Outcome::Failed,
            message,
            stats: CopyStats::default(),
        };

        let on_event = RefCell::new(on_event);
        // Pass 1 (only when asked for): list-only scan, so progress can show an exact percentage.
        // It walks source and target once more, which takes minutes on a big drive.
        match job.totals {
            TotalsSource::Scan => {
                let scan_log = job.log_file.with_extension("scan.log");
                let mut scan = Summary::default();
                match run_process(
                    &build_args(&mirror_args(job, true), &scan_log),
                    &scan_log,
                    cancel,
                    &|| false,
                    &mut |l| scan.feed(l),
                    &mut || {},
                ) {
                    Ok(Exit::Cancelled) => {
                        let _ = std::fs::remove_file(&scan_log);
                        return CopyResult {
                            outcome: Outcome::Cancelled,
                            message: "בוטל".into(),
                            stats: CopyStats::default(),
                        };
                    }
                    Err(e) => return fail(e),
                    Ok(_) => {}
                }
                let _ = std::fs::remove_file(&scan_log);
                let _ = std::fs::remove_file(stdout_path(&scan_log));
                if let (Some(f), Some(b)) = (scan.files(), scan.bytes()) {
                    (on_event.borrow_mut())(EngineEvent::Totals {
                        files: f[1],
                        bytes: b[1],
                        exact: true,
                    });
                }
            }
            TotalsSource::Estimate(files, bytes) => {
                if job.late_totals.is_some() {
                    (on_event.borrow_mut())(EngineEvent::TotalsPending);
                }
                (on_event.borrow_mut())(EngineEvent::Totals { files, bytes, exact: false });
            }
            TotalsSource::Unknown => {
                if job.late_totals.is_some() {
                    (on_event.borrow_mut())(EngineEvent::TotalsPending);
                }
                (on_event.borrow_mut())(EngineEvent::Phase("copying"));
            }
        }

        // Pass 2: the real copy.
        let mut stats = CopyStats::default();
        let args = build_args(&mirror_args(job, false), &job.log_file);
        stats.debug.push(format!("robocopy {}", command_line(&args)));
        let mut summary = Summary::default();
        let mut last_error: Option<String> = None;
        // robocopy logs a file's line again on every retry (/R), so a file that keeps failing (a
        // locked pagefile.sys on a system drive) would be counted up to three times.
        let mut last_file = String::new();
        // A full destination fails every further file too (after robocopy's retries, for each one),
        // so the first such error ends the run.
        let disk_full = Cell::new(false);
        let code = run_process(
            &args,
            &job.log_file,
            cancel,
            &|| disk_full.get(),
            &mut |l| {
                summary.feed(l);
                if is_error_line(l) {
                    if is_disk_full(parse_error(l, "").code) {
                        disk_full.set(true);
                    }
                    last_error = Some(l.to_string());
                    return;
                }
                if let Some(err) = last_error.take() {
                    // The line after an ERROR line is the human-readable reason.
                    let entry = parse_error(&err, l);
                    if stats.errors.len() < MAX_ERRORS && !stats.errors.contains(&entry) {
                        stats.errors.push(entry);
                    }
                }
                match classify(l) {
                    Line::Copy { size, path } => {
                        if path != last_file {
                            last_file.clone_from(&path);
                            (on_event.borrow_mut())(EngineEvent::File { path, size });
                        }
                    }
                    Line::Extra | Line::Other => {}
                }
            },
            // A background scan's result is passed on as soon as it arrives.
            &mut || {
                if let Some(rx) = &job.late_totals {
                    while let Ok(late) = rx.try_recv() {
                        (on_event.borrow_mut())(match late {
                            Some((files, bytes)) => EngineEvent::Totals { files, bytes, exact: true },
                            None => EngineEvent::TotalsUnavailable,
                        });
                    }
                }
            },
        );

        // The reason line may not have been written before robocopy was killed.
        if let Some(err) = last_error.take() {
            let entry = parse_error(&err, "");
            if stats.errors.len() < MAX_ERRORS && !stats.errors.contains(&entry) {
                stats.errors.push(entry);
            }
        }
        let console_errors = read_stdout_errors(&job.log_file);
        let code = match code {
            Err(e) => return fail(e),
            Ok(Exit::Cancelled) => {
                return CopyResult {
                    outcome: Outcome::Cancelled,
                    message: "הגיבוי בוטל על ידי המשתמש".into(),
                    stats,
                }
            }
            Ok(Exit::Aborted) => {
                stats.files_failed = stats.errors.len() as u64;
                return CopyResult {
                    outcome: Outcome::Failed,
                    message: DISK_FULL_MESSAGE.into(),
                    stats,
                };
            }
            Ok(Exit::Code(c)) => c,
        };
        stats.exit_code = Some(code);
        let mut files_total = 0;
        if let Some(f) = summary.files() {
            files_total = f[0];
            stats.files_copied = f[1];
            stats.files_failed = f[4];
            stats.files_deleted = f[5];
        }
        if let Some(b) = summary.bytes() {
            stats.bytes_copied = b[1];
        }
        stats.debug.push(format!(
            "exit code {code}; summary files total/copied/skipped/mismatch/failed/extras: {:?}; errors logged: {}",
            summary.files(),
            stats.errors.len()
        ));

        // Entries in the error list whose file did get copied on robocopy's retry, or can be saved by a
        // plain read and write: they are not failures. Failures robocopy counted itself remain.
        let streamed = native::settle_errors(&job.source, &job.target, &mut stats, cancel, &mut |e| (on_event.borrow_mut())(e));
        // Everything robocopy called a failure is accounted for: its "some failed" bit (8) no longer applies.
        let code = if streamed > 0 && stats.files_failed == 0 && code < 16 { code & !8 } else { code };

        // robocopy exit codes are bit flags: 1 copied, 2 extras, 4 mismatch, 8 failures, 16 fatal.
        stats.debug.extend(console_errors.iter().map(|l| format!("robocopy console: {l}")));
        let (outcome, message) = if code >= 16 {
            (Outcome::Failed, fatal_message(&console_errors))
        } else if code >= 8 && stats.errors.iter().any(|e| is_disk_full(e.code)) {
            (Outcome::Failed, DISK_FULL_MESSAGE.to_string())
        } else if code >= 8 && !mostly_copied(&stats, files_total) {
            (Outcome::Failed, format!("{} קבצים לא הועתקו", stats.files_failed.max(1)))
        } else if code >= 8 {
            // A few files that couldn't be read (locked, no access) don't make the whole copy worthless:
            // it completes with a warning, like an incremental does, instead of staying half-done and
            // being "continued" (and failing on the same files) on every run.
            (
                Outcome::Warning,
                format!(
                    "הגיבוי הסתיים, אבל {} קבצים לא הועתקו (פירוט ברשימת השגיאות)",
                    stats.files_failed.max(1)
                ),
            )
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

/// What a fatal robocopy exit (16) says to the user. A rejected parameter is a problem in the rules, not in the
/// drives; anything else robocopy printed is quoted; with nothing at all, the usual suspects.
fn fatal_message(console_errors: &[String]) -> String {
    if let Some(l) = console_errors.iter().find(|l| l.contains("Invalid Parameter")) {
        let value = l.split_once(" : ").map_or("", |(_, r)| r.trim());
        return format!(
            "robocopy דחה את אחד מכללי הסינון ולכן לא התחיל להעתיק: {value}. בדקו את הכללים של המשימה ואת הכללים הכלליים (כלל עם * או ? ונתיב), תקנו או הסירו אותו והריצו שוב."
        );
    }
    let mut m = String::from("robocopy נכשל לגמרי (קוד 16) ולא העתיק דבר. ");
    if console_errors.is_empty() {
        m.push_str("בדקו שהמקור והיעד מחוברים ושיש הרשאה אליהם, ושכללי הסינון תקינים, והריצו שוב. פרטים ביומן המפורט.");
    } else {
        m.push_str(&format!("robocopy דיווח: {}. פרטים ב\"העתקת השגיאות\".", console_errors.join("; ")));
    }
    m
}

/// Some files failed, but nothing points at the drives and almost everything else was copied (or
/// was already there): at most 5% of the files failed.
fn mostly_copied(stats: &CopyStats, files_total: u64) -> bool {
    let drive_trouble = stats.errors.iter().any(|e| e.code.is_some_and(|c| DRIVE_ERRORS.contains(&c)));
    !drive_trouble && files_total > 0 && stats.files_failed * 20 <= files_total
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rule robocopy rejects is reported as a rule problem, with the rule, not as "drives not reachable".
    #[test]
    fn rejected_rule_is_named_in_the_failure() {
        let dir = std::env::temp_dir().join(format!("backuper-reject-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let job = CopyJob {
            source: dir.join("src"),
            target: dir.join("dst"),
            copy_empty_dirs: false,
            filters: crate::engine::Filters { exclude_dirs: vec![r"*\bad\rule".into()], ..Default::default() },
            log_file: dir.join("run.log"),
            totals: crate::engine::TotalsSource::Unknown,
            late_totals: None,
        };
        let r = Robocopy.mirror(&job, &AtomicBool::new(false), &mut |_| {});
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(r.outcome, Outcome::Failed);
        assert!(r.message.contains(r"*\bad\rule") && r.message.contains("כללי הסינון"), "{}", r.message);
        assert!(r.stats.debug.iter().any(|l| l.contains("Invalid Parameter")));
    }

    #[test]
    fn parses_error_lines() {
        let e = parse_error(
            "2026/10/01 03:00:00 ERROR 32 (0x00000020) Copying File C:\\src\\נעול.db",
            "The process cannot access the file because it is being used by another process.",
        );
        assert_eq!(e.code, Some(32));
        assert_eq!(e.path, "C:\\src\\נעול.db");
        assert!(e.message.starts_with("The process"));
        let e = parse_error(
            "2026/10/01 03:00:00 ERROR 53 (0x00000035) Accessing Source Directory \\\\nas\\x\\",
            "",
        );
        assert_eq!((e.code, e.path.as_str()), (Some(53), "\\\\nas\\x\\"));
    }

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
    fn a_few_failed_files_still_complete_the_backup() {
        let stats = |failed: u64, code: u32| CopyStats {
            files_failed: failed,
            errors: vec![FileError { path: "C:\\x".into(), code: Some(code), message: String::new(), debug: String::new() }],
            ..Default::default()
        };
        assert!(mostly_copied(&stats(26, 32), 21_101));
        assert!(!mostly_copied(&stats(26, 112), 21_101), "destination full");
        assert!(!mostly_copied(&stats(26, 53), 21_101), "network path gone");
        assert!(!mostly_copied(&stats(600, 5), 1_000), "most files failed");
        assert!(!mostly_copied(&stats(1, 32), 0));
    }

    #[test]
    fn trims_trailing_backslash() {
        assert_eq!(arg_path(Path::new("D:\\")), OsString::from("D:\\"));
        assert_eq!(arg_path(Path::new("D:\\My Files\\")), OsString::from("D:\\My Files"));
    }
}
