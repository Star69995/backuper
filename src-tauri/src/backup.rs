//! Backup orchestration on top of the copy engine: dated folders, full vs incremental,
//! retention of old backups and validation of task paths.
//!
//! Layout: <destination>\<folder_name> YYYY-MM-DD HH-mm   (the date the backup was created)
//! While a full backup is being written (or if it failed) the folder carries a ".partial" suffix.

use crate::engine::{CopyEngine, CopyJob, EngineEvent, Outcome};
use crate::model::{BackupFolder, BackupMode, FilterRule, RunStatus, Task};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

const DATE_FMT: &str = "%Y-%m-%d %H-%M";
const DATE_FMT_SECS: &str = "%Y-%m-%d %H-%M-%S";
const PARTIAL: &str = ".partial";

/// Characters Windows forbids in file names.
pub fn sanitize_folder_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    s.trim().trim_end_matches('.').to_string()
}

fn normalize(p: &str) -> String {
    let s = p.trim().replace('/', "\\");
    let s = s.trim_end_matches('\\').to_lowercase();
    s + "\\"
}

pub fn validate_task(t: &Task) -> Result<(), String> {
    if t.name.trim().is_empty() {
        return Err("יש לתת שם למשימה".into());
    }
    if t.source.trim().is_empty() || t.destination.trim().is_empty() {
        return Err("יש לבחור תיקיית מקור ותיקיית יעד".into());
    }
    if !Path::new(t.source.trim()).is_absolute() || !Path::new(t.destination.trim()).is_absolute() {
        return Err("נתיבי המקור והיעד חייבים להיות מלאים (למשל D:\\Backup)".into());
    }
    let (s, d) = (normalize(&t.source), normalize(&t.destination));
    if s == d {
        return Err("המקור והיעד לא יכולים להיות אותה תיקייה".into());
    }
    if d.starts_with(&s) {
        return Err("היעד לא יכול להיות בתוך תיקיית המקור".into());
    }
    if s.starts_with(&d) {
        return Err("המקור לא יכול להיות בתוך תיקיית היעד".into());
    }
    if sanitize_folder_name(&t.folder_name).is_empty() {
        return Err("יש לתת שם לתיקיות הגיבוי".into());
    }
    if t.keep_count == 0 {
        return Err("יש לשמור לפחות גיבוי אחד".into());
    }
    crate::filters::validate(&t.filters)?;
    crate::schedule::validate(&t.schedule)
}

fn parse_folder(prefix: &str, name: &str) -> Option<(DateTime<Local>, bool)> {
    let rest = name.strip_prefix(prefix)?.strip_prefix(' ')?;
    let (date, partial) = match rest.strip_suffix(PARTIAL) {
        Some(d) => (d, true),
        None => (rest, false),
    };
    let naive = NaiveDateTime::parse_from_str(date, DATE_FMT_SECS)
        .or_else(|_| NaiveDateTime::parse_from_str(date, DATE_FMT))
        .ok()?;
    Some((Local.from_local_datetime(&naive).earliest()?, partial))
}

/// This task's dated backup folders, newest first.
pub fn list_backups(task: &Task) -> Vec<BackupFolder> {
    let prefix = sanitize_folder_name(&task.folder_name);
    let Ok(entries) = fs::read_dir(task.destination.trim()) else { return Vec::new() };
    let mut out: Vec<BackupFolder> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let (created_at, partial) = parse_folder(&prefix, &name)?;
            Some(BackupFolder { path: e.path().to_string_lossy().to_string(), name, created_at, partial })
        })
        .collect();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    out
}

/// Deletes one backup folder, only if it really is one of this task's dated folders.
pub fn delete_backup(task: &Task, name: &str) -> Result<(), String> {
    let found = list_backups(task).into_iter().find(|b| b.name == name).ok_or("תיקיית הגיבוי לא נמצאה")?;
    fs::remove_dir_all(&found.path).map_err(|e| format!("מחיקה נכשלה: {e}"))
}

fn new_folder_path(task: &Task, now: DateTime<Local>) -> PathBuf {
    let dest = Path::new(task.destination.trim());
    let prefix = sanitize_folder_name(&task.folder_name);
    let mut name = format!("{prefix} {}", now.format(DATE_FMT));
    let taken = |n: &str| dest.join(n).exists() || dest.join(format!("{n}{PARTIAL}")).exists();
    if taken(&name) {
        name = format!("{prefix} {}", now.format(DATE_FMT_SECS));
    }
    dest.join(name)
}

fn with_partial(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(PARTIAL);
    PathBuf::from(s)
}

/// Windows can briefly hold a folder (indexer, antivirus); retry renames a few times.
fn rename_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = Ok(());
    for i in 0..10 {
        last = fs::rename(from, to);
        if last.is_ok() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(300 * (i + 1)));
    }
    last
}

pub struct RunOutput {
    pub status: RunStatus,
    pub mode: BackupMode,
    pub message: String,
    pub target_folder: Option<String>,
    pub stats: crate::engine::CopyStats,
}

impl RunOutput {
    fn failed(mode: BackupMode, message: impl Into<String>) -> Self {
        Self {
            status: RunStatus::Failed,
            mode,
            message: message.into(),
            target_folder: None,
            stats: Default::default(),
        }
    }
}

fn status_of(o: Outcome) -> RunStatus {
    match o {
        Outcome::Success => RunStatus::Success,
        Outcome::Warning => RunStatus::Warning,
        Outcome::Failed => RunStatus::Failed,
        Outcome::Cancelled => RunStatus::Cancelled,
    }
}

/// Decides whether this run is full or incremental (incremental needs a complete base folder).
pub fn effective_mode(task: &Task, requested: BackupMode, now: DateTime<Local>) -> (BackupMode, Option<String>) {
    if requested == BackupMode::Full {
        return (BackupMode::Full, None);
    }
    let latest = list_backups(task).into_iter().find(|b| !b.partial);
    match latest {
        None => (BackupMode::Full, Some("אין גיבוי מלא קודם - מבוצע גיבוי מלא".into())),
        Some(b) if task.full_every_days > 0
            && (now - b.created_at).num_days() >= task.full_every_days as i64 =>
        {
            (BackupMode::Full, Some(format!("עברו {} ימים מהגיבוי המלא האחרון - מבוצע גיבוי מלא", task.full_every_days)))
        }
        Some(_) => (BackupMode::Incremental, None),
    }
}

pub fn run_backup(
    engine: &dyn CopyEngine,
    task: &Task,
    requested: BackupMode,
    global_filters: &[FilterRule],
    log_file: PathBuf,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(EngineEvent),
) -> RunOutput {
    let now = Local::now();
    let source = PathBuf::from(task.source.trim());
    let dest_root = PathBuf::from(task.destination.trim());
    if !source.is_dir() {
        return RunOutput::failed(requested, format!("תיקיית המקור לא נמצאה: {}", source.display()));
    }
    if let Err(e) = fs::create_dir_all(&dest_root) {
        return RunOutput::failed(requested, format!("היעד לא זמין ({}): {e}", dest_root.display()));
    }

    let (mode, note) = effective_mode(task, requested, now);
    let mut notes: Vec<String> = note.into_iter().collect();
    let existing = list_backups(task);

    let (target, final_path) = match mode {
        BackupMode::Incremental => {
            let base = existing.iter().find(|b| !b.partial).expect("checked by effective_mode");
            (PathBuf::from(&base.path), None)
        }
        BackupMode::Full => {
            let final_path = new_folder_path(task, now);
            let work = with_partial(&final_path);
            // Fast full: reuse the newest folder so only differences are copied.
            let reused = task.reuse_previous
                && task.keep_count == 1
                && existing.first().is_some_and(|prev| match rename_retry(Path::new(&prev.path), &work) {
                    Ok(()) => {
                        notes.push(format!("התיקייה הקודמת ({}) עודכנה למצב הנוכחי", prev.name));
                        true
                    }
                    Err(e) => {
                        notes.push(format!("לא ניתן לשנות את שם הגיבוי הקודם ({e}) - מבוצעת העתקה מלאה"));
                        false
                    }
                });
            if !reused {
                if let Err(e) = fs::create_dir_all(&work) {
                    return RunOutput::failed(mode, format!("לא ניתן ליצור את תיקיית הגיבוי: {e}"));
                }
            }
            (work, Some(final_path))
        }
    };

    let job = CopyJob {
        source,
        target: target.clone(),
        mirror: mode == BackupMode::Full,
        copy_empty_dirs: task.copy_empty_dirs,
        filters: crate::filters::compile(
            task.filters.iter().chain(global_filters.iter().filter(|_| task.use_global_filters)),
        ),
        log_file,
    };
    let result = engine.run(&job, cancel, on_event);
    let mut status = status_of(result.outcome);
    let mut message = result.message;
    let mut target_folder = target.to_string_lossy().to_string();

    if let Some(final_path) = final_path {
        if matches!(status, RunStatus::Success | RunStatus::Warning) {
            match rename_retry(&target, &final_path) {
                Ok(()) => {
                    target_folder = final_path.to_string_lossy().to_string();
                    // Retention: keep the newest keep_count complete backups (including this one).
                    let keep_old = task.keep_count.saturating_sub(1) as usize;
                    let current_name = final_path.file_name().map(|n| n.to_string_lossy().to_string());
                    let mut kept = 0;
                    for b in list_backups(task) {
                        if Some(&b.name) == current_name.as_ref() {
                            continue;
                        }
                        if !b.partial && kept < keep_old {
                            kept += 1;
                            continue;
                        }
                        match fs::remove_dir_all(&b.path) {
                            Ok(()) => notes.push(format!("נמחק גיבוי קודם: {}", b.name)),
                            Err(e) => notes.push(format!("לא ניתן למחוק את {}: {e}", b.name)),
                        }
                    }
                }
                Err(e) => {
                    status = RunStatus::Warning;
                    message = format!("{message}. לא ניתן לסמן את הגיבוי כמושלם ({e})");
                }
            }
        }
    }
    if !notes.is_empty() {
        message = format!("{message}. {}", notes.join(". "));
    }
    RunOutput { status, mode, message, target_folder: Some(target_folder), stats: result.stats }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(src: &str, dst: &str) -> Task {
        Task {
            name: "t".into(),
            folder_name: "t".into(),
            source: src.into(),
            destination: dst.into(),
            ..Default::default()
        }
    }

    #[test]
    fn rejects_nested_paths() {
        assert!(validate_task(&task("C:\\Data", "C:\\Data\\Backup")).is_err());
        assert!(validate_task(&task("D:\\Backup\\x", "D:\\Backup")).is_err());
        assert!(validate_task(&task("C:\\Data", "c:\\data\\")).is_err());
        assert!(validate_task(&task("C:\\Data", "D:\\Backup")).is_ok());
        assert!(validate_task(&task("C:\\Data", "C:\\Data2")).is_ok());
    }

    #[test]
    fn parses_folder_names() {
        let (_, partial) = parse_folder("Docs", "Docs 2026-10-01 03-00").unwrap();
        assert!(!partial);
        let (_, partial) = parse_folder("Docs", "Docs 2026-10-01 03-00-15.partial").unwrap();
        assert!(partial);
        assert!(parse_folder("Docs", "Docs old").is_none());
        assert!(parse_folder("Docs", "Docs2 2026-10-01 03-00").is_none());
    }

    /// End-to-end against the real robocopy.
    #[test]
    fn full_incremental_full_cycle() {
        use crate::engine::robocopy::Robocopy;
        let root = std::env::temp_dir().join(format!("backuper-test-{}", uuid::Uuid::new_v4()));
        let (src, dst) = (root.join("src"), root.join("dst"));
        fs::create_dir_all(src.join("תיקייה")).unwrap();
        fs::create_dir_all(src.join("empty")).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        fs::write(src.join("תיקייה").join("קובץ.txt"), "שלום").unwrap();
        let t = Task {
            id: "x".into(),
            name: "Docs".into(),
            folder_name: "Docs".into(),
            source: src.to_string_lossy().into(),
            destination: dst.to_string_lossy().into(),
            ..Default::default()
        };
        let cancel = AtomicBool::new(false);
        let mut files = 0;
        let run = |t: &Task, mode, files: &mut u32| {
            let log = root.join(format!("{}.log", uuid::Uuid::new_v4()));
            run_backup(&Robocopy, t, mode, &[], log, &cancel, &mut |e| {
                if let EngineEvent::File { .. } = e {
                    *files += 1
                }
            })
        };

        // Incremental with no base -> full.
        let out = run(&t, BackupMode::Incremental, &mut files);
        assert_eq!(out.status, RunStatus::Success, "{}", out.message);
        assert_eq!(out.mode, BackupMode::Full);
        assert_eq!(out.stats.files_copied, 2);
        assert_eq!(files, 2);
        let first = list_backups(&t);
        assert_eq!(first.len(), 1);
        assert!(!first[0].partial);
        let base = PathBuf::from(&first[0].path);
        assert_eq!(fs::read_to_string(base.join("תיקייה").join("קובץ.txt")).unwrap(), "שלום");
        assert!(!base.join("empty").exists(), "empty dirs are skipped by default");

        // Incremental copies only the new file and keeps deleted ones.
        fs::write(src.join("b.txt"), "b").unwrap();
        fs::remove_file(src.join("a.txt")).unwrap();
        let out = run(&t, BackupMode::Incremental, &mut files);
        assert_eq!(out.mode, BackupMode::Incremental);
        assert_eq!(out.stats.files_copied, 1);
        assert!(base.join("b.txt").exists() && base.join("a.txt").exists());

        // Full: new dated folder identical to source, previous one deleted (keep_count = 1).
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let out = run(&t, BackupMode::Full, &mut files);
        assert_eq!(out.status, RunStatus::Success, "{}", out.message);
        let after = list_backups(&t);
        assert_eq!(after.len(), 1, "{after:?}");
        assert_ne!(after[0].name, first[0].name);
        let newest = PathBuf::from(&after[0].path);
        assert!(newest.join("b.txt").exists() && !newest.join("a.txt").exists());

        // Fast full: the previous folder is renamed and mirrored, so unchanged files aren't copied.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        fs::write(src.join("c.txt"), "c").unwrap();
        let fast = Task { reuse_previous: true, ..t.clone() };
        let out = run(&fast, BackupMode::Full, &mut files);
        assert_eq!(out.status, RunStatus::Success, "{}", out.message);
        assert_eq!(out.stats.files_copied, 1);
        let after = list_backups(&t);
        assert_eq!(after.len(), 1);
        assert!(PathBuf::from(&after[0].path).join("c.txt").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn filter_rules_exclude_files() {
        use crate::engine::robocopy::Robocopy;
        let root = std::env::temp_dir().join(format!("backuper-filter-{}", uuid::Uuid::new_v4()));
        let src = root.join("src");
        fs::create_dir_all(src.join("node_modules")).unwrap();
        fs::create_dir_all(src.join("docs")).unwrap();
        fs::write(src.join("keep.txt"), "k").unwrap();
        fs::write(src.join("junk.tmp"), "t").unwrap();
        fs::write(src.join("docs").join("~$draft.docx"), "d").unwrap();
        fs::write(src.join("node_modules").join("lib.js"), "x").unwrap();
        fs::write(src.join("big.bin"), vec![0u8; 2 * 1024 * 1024]).unwrap();
        let t = Task {
            id: "f".into(),
            name: "F".into(),
            folder_name: "F".into(),
            source: src.to_string_lossy().into(),
            destination: root.join("dst").to_string_lossy().into(),
            filters: vec![
                FilterRule::Extension { value: "tmp".into() },
                FilterRule::Folder { value: "node_modules".into() },
                FilterRule::LargerThan { mb: 1 },
            ],
            ..Default::default()
        };
        let global = [FilterRule::Pattern { value: "~$*".into() }];
        let out = run_backup(&Robocopy, &t, BackupMode::Full, &global, root.join("r.log"), &AtomicBool::new(false), &mut |_| {});
        assert_eq!(out.status, RunStatus::Success, "{}", out.message);
        let b = PathBuf::from(&list_backups(&t)[0].path);
        assert!(b.join("keep.txt").exists());
        assert!(!b.join("junk.tmp").exists());
        assert!(!b.join("node_modules").exists());
        assert!(!b.join("big.bin").exists());
        assert!(!b.join("docs").join("~$draft.docx").exists(), "global filter applies");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize_folder_name(" a:b/c. "), "a_b_c");
    }
}
