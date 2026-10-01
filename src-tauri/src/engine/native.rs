//! Copies an explicit list of files (the changes of an incremental backup) into a new folder.
//! std::fs::copy uses CopyFileExW on Windows, so attributes and the modified time are preserved.

use super::{CopyResult, CopyStats, EngineEvent, ListedFile, Outcome};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const MAX_ERRORS: usize = 50;

/// Writes a UTF-16LE log, the same encoding robocopy's /UNILOG uses, so one reader handles both.
fn write_log(path: &Path, lines: &[String]) {
    let mut bytes = vec![0xFF, 0xFE];
    for l in lines {
        for u in l.encode_utf16().chain("\r\n".encode_utf16()) {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
    }
    let _ = fs::write(path, bytes);
}

pub fn copy_files(
    source: &Path,
    target: &Path,
    files: &[ListedFile],
    log_file: &Path,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(EngineEvent),
) -> CopyResult {
    let mut stats = CopyStats::default();
    let mut log = vec![
        format!("Backuper - גיבוי אינקרמנטלי"),
        format!("מקור: {}", source.display()),
        format!("יעד:  {}", target.display()),
        format!("קבצים להעתקה: {}", files.len()),
        String::new(),
    ];
    on_event(EngineEvent::Totals {
        files: files.len() as u64,
        bytes: files.iter().map(|f| f.size).sum(),
    });

    for f in files {
        if cancel.load(Ordering::SeqCst) {
            log.push("בוטל על ידי המשתמש".into());
            write_log(log_file, &log);
            return CopyResult {
                outcome: Outcome::Cancelled,
                message: "הגיבוי בוטל על ידי המשתמש".into(),
                stats,
            };
        }
        let from = source.join(&f.rel);
        let to = target.join(&f.rel);
        on_event(EngineEvent::File {
            path: from.to_string_lossy().to_string(),
            size: f.size,
        });
        let copy = || -> std::io::Result<u64> {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&from, &to)
        };
        // One retry: files are often locked only for a moment.
        let result = copy().or_else(|_| {
            std::thread::sleep(Duration::from_secs(2));
            copy()
        });
        match result {
            Ok(n) => {
                stats.files_copied += 1;
                stats.bytes_copied += n;
                log.push(format!("הועתק\t{n}\t{}", from.display()));
            }
            Err(e) => {
                stats.files_failed += 1;
                let entry = format!("{} - {e}", from.display());
                log.push(format!("שגיאה\t{entry}"));
                if stats.errors.len() < MAX_ERRORS {
                    stats.errors.push(entry);
                }
            }
        }
    }
    log.push(String::new());
    log.push(format!("הועתקו: {}, נכשלו: {}", stats.files_copied, stats.files_failed));
    write_log(log_file, &log);

    let (outcome, message) = if stats.files_failed > 0 {
        (
            Outcome::Warning,
            format!("הועתקו {} קבצים, {} קבצים לא הועתקו", stats.files_copied, stats.files_failed),
        )
    } else if stats.files_copied == 0 {
        (Outcome::Success, "אין שינויים מאז הגיבוי הקודם".to_string())
    } else {
        (
            Outcome::Success,
            format!("הועתקו {} קבצים חדשים או שהשתנו", stats.files_copied),
        )
    };
    CopyResult { outcome, message, stats }
}
