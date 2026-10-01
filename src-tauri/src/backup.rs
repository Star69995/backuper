//! Backup orchestration on top of the copy engine: one chain of dated folders per source,
//! full vs incremental, retention, and validation of task paths.
//!
//! Layout, per source, inside the task's destination:
//!   <folderName> 2026-10-01 03-00 מלא            full backup: identical to the source
//!   <folderName> 2026-10-02 03-00 אינקרמנטלי     only files new/changed since the previous backup
//! A folder still being written (or that failed/was cancelled) carries a ".partial" suffix.
//! A full backup plus the incrementals after it form a chain. When a full backup succeeds,
//! chains beyond keep_count are deleted, and so are empty incremental folders.

use crate::engine::{native, CopyEngine, CopyJob, CopyStats, EngineEvent, Filters, ListedFile, Outcome};
use crate::model::{BackupFolder, BackupMode, RunStatus, Source, SourceBackups, SourceRun, Task};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

const DATE_FMT: &str = "%Y-%m-%d %H-%M";
const DATE_FMT_SECS: &str = "%Y-%m-%d %H-%M-%S";
const PARTIAL: &str = ".partial";
const FULL_TAG: &str = "מלא";
const INCREMENTAL_TAG: &str = "אינקרמנטלי";
/// Same tolerance robocopy uses with /FFT (FAT/exFAT store times in 2-second steps).
const MTIME_TOLERANCE_SECS: u64 = 2;

/// Characters Windows forbids in file names.
pub fn sanitize_folder_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c })
        .collect();
    s.trim().trim_end_matches('.').to_string()
}

/// "C:\Users\me\Documents" -> "Documents", "D:\" -> "D".
pub fn default_folder_name(path: &str) -> String {
    let p = path.trim().trim_end_matches('\\');
    let last = p.rsplit('\\').next().unwrap_or(p);
    sanitize_folder_name(last.trim_end_matches(':'))
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
    if t.sources.is_empty() {
        return Err("יש להוסיף לפחות תיקיית מקור אחת".into());
    }
    if t.destination.trim().is_empty() {
        return Err("יש לבחור תיקיית יעד".into());
    }
    if !Path::new(t.destination.trim()).is_absolute() {
        return Err("נתיב היעד חייב להיות מלא (למשל D:\\Backup)".into());
    }
    let d = normalize(&t.destination);
    let mut names: Vec<String> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    for src in &t.sources {
        let path = src.path.trim();
        if path.is_empty() {
            return Err("יש לבחור נתיב לכל תיקיית מקור".into());
        }
        if !Path::new(path).is_absolute() {
            return Err(format!("נתיב המקור חייב להיות מלא: {path}"));
        }
        let s = normalize(path);
        if s == d {
            return Err(format!("המקור והיעד לא יכולים להיות אותה תיקייה: {path}"));
        }
        if d.starts_with(&s) {
            return Err(format!("היעד לא יכול להיות בתוך תיקיית המקור {path}"));
        }
        if s.starts_with(&d) {
            return Err(format!("תיקיית המקור {path} לא יכולה להיות בתוך תיקיית היעד"));
        }
        if paths.contains(&s) {
            return Err(format!("תיקיית המקור {path} מופיעה פעמיים"));
        }
        paths.push(s);
        let name = sanitize_folder_name(&src.folder_name).to_lowercase();
        if name.is_empty() {
            return Err(format!("יש לתת שם לתיקיות הגיבוי של {path}"));
        }
        if names.contains(&name) {
            return Err(format!(
                "לשתי תיקיות מקור יש אותו שם תיקיית גיבוי ({}) - יש לשנות אחד מהם",
                src.folder_name
            ));
        }
        names.push(name);
    }
    if t.keep_count == 0 {
        return Err("יש לשמור לפחות גיבוי אחד".into());
    }
    crate::filters::validate(&t.filters)?;
    crate::schedule::validate(&t.schedule)
}

fn parse_folder(prefix: &str, name: &str) -> Option<(DateTime<Local>, BackupMode, bool)> {
    let rest = name.strip_prefix(prefix)?.strip_prefix(' ')?;
    let (rest, partial) = match rest.strip_suffix(PARTIAL) {
        Some(r) => (r, true),
        None => (rest, false),
    };
    // Folders from before the type tag existed are full backups.
    let (date, kind) = if let Some(d) = rest.strip_suffix(INCREMENTAL_TAG).and_then(|d| d.strip_suffix(' ')) {
        (d, BackupMode::Incremental)
    } else if let Some(d) = rest.strip_suffix(FULL_TAG).and_then(|d| d.strip_suffix(' ')) {
        (d, BackupMode::Full)
    } else {
        (rest, BackupMode::Full)
    };
    let naive = NaiveDateTime::parse_from_str(date, DATE_FMT_SECS)
        .or_else(|_| NaiveDateTime::parse_from_str(date, DATE_FMT))
        .ok()?;
    Some((Local.from_local_datetime(&naive).earliest()?, kind, partial))
}

/// One source's dated backup folders in `dest`, newest first.
pub fn list_backups(dest: &str, folder_name: &str) -> Vec<BackupFolder> {
    let prefix = sanitize_folder_name(folder_name);
    let Ok(entries) = fs::read_dir(dest.trim()) else {
        return Vec::new();
    };
    let mut out: Vec<BackupFolder> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let (created_at, kind, partial) = parse_folder(&prefix, &name)?;
            Some(BackupFolder {
                path: e.path().to_string_lossy().to_string(),
                name,
                created_at,
                kind,
                partial,
            })
        })
        .collect();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| b.name.cmp(&a.name)));
    out
}

pub fn list_task_backups(task: &Task) -> Vec<SourceBackups> {
    task.sources
        .iter()
        .map(|s| SourceBackups {
            source: s.path.clone(),
            folder_name: s.folder_name.clone(),
            backups: list_backups(&task.destination, &s.folder_name),
        })
        .collect()
}

/// Deletes one backup folder, only if it really is one of this task's dated folders.
pub fn delete_backup(task: &Task, folder_name: &str, name: &str) -> Result<(), String> {
    if !task.sources.iter().any(|s| s.folder_name == folder_name) {
        return Err("תיקיית המקור לא נמצאה במשימה".into());
    }
    let found = list_backups(&task.destination, folder_name)
        .into_iter()
        .find(|b| b.name == name)
        .ok_or("תיקיית הגיבוי לא נמצאה")?;
    fs::remove_dir_all(&found.path).map_err(|e| format!("מחיקה נכשלה: {e}"))
}

fn new_folder_path(dest: &Path, prefix: &str, now: DateTime<Local>, kind: BackupMode) -> PathBuf {
    let tag = if kind == BackupMode::Full { FULL_TAG } else { INCREMENTAL_TAG };
    let taken = |n: &str| dest.join(n).exists() || dest.join(format!("{n}{PARTIAL}")).exists();
    let name = format!("{prefix} {} {tag}", now.format(DATE_FMT));
    if !taken(&name) {
        return dest.join(name);
    }
    dest.join(format!("{prefix} {} {tag}", now.format(DATE_FMT_SECS)))
}

fn with_partial(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(PARTIAL);
    PathBuf::from(s)
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
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

fn has_files(path: &Path) -> bool {
    let Ok(rd) = fs::read_dir(path) else { return false };
    rd.flatten().any(|e| match e.file_type() {
        Ok(t) if t.is_dir() => has_files(&e.path()),
        Ok(_) => true,
        Err(_) => false,
    })
}

/// Deletes old backups of one source. Keeps the newest `keep_fulls` complete full backups
/// (besides `protect`) together with their non-empty incrementals; everything else goes:
/// older chains, partial folders and empty incremental folders.
fn prune(dest: &str, prefix: &str, keep_fulls: usize, protect: Option<&str>, notes: &mut Vec<String>) -> u64 {
    let all: Vec<BackupFolder> = list_backups(dest, prefix)
        .into_iter()
        .filter(|b| Some(b.name.as_str()) != protect)
        .collect();
    let kept_fulls: Vec<&BackupFolder> = all
        .iter()
        .filter(|b| b.kind == BackupMode::Full && !b.partial)
        .take(keep_fulls)
        .collect();
    let oldest_kept = kept_fulls.last().map(|b| b.created_at);
    let mut removed_empty = 0;
    let mut deleted = 0;
    for b in &all {
        let keep = match b.kind {
            BackupMode::Full => kept_fulls.iter().any(|k| k.name == b.name),
            BackupMode::Incremental => {
                let in_kept_chain = !b.partial && oldest_kept.is_some_and(|d| b.created_at > d);
                in_kept_chain && has_files(Path::new(&b.path))
            }
        };
        if keep {
            continue;
        }
        let empty = b.kind == BackupMode::Incremental && !has_files(Path::new(&b.path));
        match fs::remove_dir_all(&b.path) {
            Ok(()) if empty => removed_empty += 1,
            Ok(()) => {
                deleted += 1;
                notes.push(format!("נמחק גיבוי קודם: {}", b.name));
            }
            Err(e) => notes.push(format!("לא ניתן למחוק את {}: {e}", b.name)),
        }
    }
    if removed_empty > 0 {
        notes.push(format!("נמחקו {removed_empty} תיקיות אינקרמנטליות ריקות"));
    }
    deleted
}

/// Decides whether this source's run is full or incremental (incremental needs a complete full).
fn effective_mode(
    task: &Task,
    backups: &[BackupFolder],
    requested: BackupMode,
    now: DateTime<Local>,
) -> (BackupMode, Option<String>) {
    if requested == BackupMode::Full {
        return (BackupMode::Full, None);
    }
    match backups.iter().find(|b| b.kind == BackupMode::Full && !b.partial) {
        None => (BackupMode::Full, Some("אין גיבוי מלא קודם - מבוצע גיבוי מלא".into())),
        Some(b) if task.full_every_days > 0 && (now - b.created_at).num_days() >= task.full_every_days as i64 => (
            BackupMode::Full,
            Some(format!(
                "עברו {} ימים מהגיבוי המלא האחרון - מבוצע גיבוי מלא",
                task.full_every_days
            )),
        ),
        Some(_) => (BackupMode::Incremental, None),
    }
}

/// What is already backed up: rel path (lowercase) -> (size, modified), from the latest complete
/// full backup overlaid with every incremental after it.
fn chain_index(backups: &[BackupFolder]) -> HashMap<String, (u64, SystemTime)> {
    fn walk(root: &Path, dir: &Path, out: &mut HashMap<String, (u64, SystemTime)>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                walk(root, &p, out);
            } else if let (Ok(m), Ok(rel)) = (e.metadata(), p.strip_prefix(root)) {
                out.insert(
                    rel.to_string_lossy().to_lowercase(),
                    (m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)),
                );
            }
        }
    }
    let mut index = HashMap::new();
    let Some(base) = backups.iter().find(|b| b.kind == BackupMode::Full && !b.partial) else {
        return index;
    };
    let mut chain: Vec<&BackupFolder> = backups
        .iter()
        .filter(|b| b.kind == BackupMode::Incremental && b.created_at > base.created_at)
        .collect();
    chain.push(base);
    chain.reverse(); // oldest first, so newer copies override
    for b in chain {
        let root = Path::new(&b.path);
        walk(root, root, &mut index);
    }
    index
}

fn changed_since(source: &Path, listed: Vec<ListedFile>, index: &HashMap<String, (u64, SystemTime)>) -> Vec<ListedFile> {
    listed
        .into_iter()
        .filter(|f| {
            let Some((size, mtime)) = index.get(&f.rel.to_lowercase()) else {
                return true;
            };
            if *size != f.size {
                return true;
            }
            let Ok(src_mtime) = fs::metadata(source.join(&f.rel)).and_then(|m| m.modified()) else {
                return true;
            };
            let diff = src_mtime.duration_since(*mtime).or_else(|_| mtime.duration_since(src_mtime));
            diff.map(|d| d.as_secs() > MTIME_TOLERANCE_SECS).unwrap_or(true)
        })
        .collect()
}

fn status_of(o: Outcome) -> RunStatus {
    match o {
        Outcome::Success => RunStatus::Success,
        Outcome::Warning => RunStatus::Warning,
        Outcome::Failed => RunStatus::Failed,
        Outcome::Cancelled => RunStatus::Cancelled,
    }
}

fn apply_stats(run: &mut SourceRun, stats: CopyStats) {
    run.files_copied = stats.files_copied;
    run.bytes_copied = stats.bytes_copied;
    run.files_deleted = stats.files_deleted;
    run.files_failed = stats.files_failed;
    run.errors = stats.errors;
    run.exit_code = stats.exit_code;
}

pub struct RunContext<'a> {
    pub engine: &'a dyn CopyEngine,
    pub filters: &'a Filters,
    /// The same timestamp for every source of the run (folder names share the date).
    pub now: DateTime<Local>,
    pub cancel: &'a AtomicBool,
}

/// Backs up one source of a task into its own chain of dated folders.
pub fn backup_source(
    ctx: &RunContext,
    task: &Task,
    src: &Source,
    requested: BackupMode,
    log_file: PathBuf,
    on_event: &mut dyn FnMut(EngineEvent),
) -> SourceRun {
    let mut run = SourceRun {
        source: src.path.clone(),
        folder_name: src.folder_name.clone(),
        ..Default::default()
    };
    let fail = |mut run: SourceRun, msg: String| {
        run.status = Some(RunStatus::Failed);
        run.message = msg;
        run
    };
    let source = PathBuf::from(src.path.trim());
    let dest = PathBuf::from(task.destination.trim());
    let prefix = sanitize_folder_name(&src.folder_name);
    if !source.is_dir() {
        return fail(run, format!("תיקיית המקור לא נמצאה: {}", source.display()));
    }
    if let Err(e) = fs::create_dir_all(&dest) {
        return fail(run, format!("היעד לא זמין ({}): {e}", dest.display()));
    }

    let backups = list_backups(&task.destination, &prefix);
    let (mode, note) = effective_mode(task, &backups, requested, ctx.now);
    run.mode = Some(mode);
    let mut notes: Vec<String> = note.into_iter().collect();
    let final_path = new_folder_path(&dest, &prefix, ctx.now, mode);
    let work = with_partial(&final_path);

    let result = match mode {
        BackupMode::Full => {
            // Fast full: reuse the newest full folder so only differences are copied.
            let latest_full = backups.iter().find(|b| b.kind == BackupMode::Full && !b.partial);
            let reused = task.reuse_previous
                && task.keep_count == 1
                && latest_full.is_some_and(|prev| match rename_retry(Path::new(&prev.path), &work) {
                    Ok(()) => {
                        notes.push(format!("הגיבוי המלא הקודם ({}) עודכן למצב הנוכחי", prev.name));
                        true
                    }
                    Err(e) => {
                        notes.push(format!("לא ניתן לשנות את שם הגיבוי הקודם ({e}) - מבוצעת העתקה מלאה"));
                        false
                    }
                });
            if task.delete_before {
                on_event(EngineEvent::Phase("deleting"));
                let deleted = prune(
                    &task.destination,
                    &prefix,
                    task.keep_count as usize - 1,
                    Some(&file_name(&work)),
                    &mut notes,
                );
                if deleted > 0 {
                    notes.push("הגיבויים הקודמים נמחקו לפני תחילת הגיבוי (לפי הגדרות המשימה)".into());
                }
            }
            if !reused {
                if let Err(e) = fs::create_dir_all(&work) {
                    return fail(run, format!("לא ניתן ליצור את תיקיית הגיבוי: {e}"));
                }
            }
            on_event(EngineEvent::Phase("scanning"));
            let job = CopyJob {
                source: source.clone(),
                target: work.clone(),
                copy_empty_dirs: task.copy_empty_dirs,
                filters: ctx.filters.clone(),
                log_file: log_file.clone(),
            };
            ctx.engine.mirror(&job, ctx.cancel, &mut |e| on_event(e))
        }
        BackupMode::Incremental => {
            on_event(EngineEvent::Phase("scanning"));
            let listed = match ctx
                .engine
                .list_files(&source, ctx.filters, &log_file.with_extension("scan.log"), ctx.cancel)
            {
                Ok(Some(l)) => l,
                Ok(None) => {
                    run.status = Some(RunStatus::Cancelled);
                    run.message = "הגיבוי בוטל על ידי המשתמש".into();
                    return run;
                }
                Err(e) => return fail(run, e),
            };
            let changed = changed_since(&source, listed, &chain_index(&backups));
            if let Err(e) = fs::create_dir_all(&work) {
                return fail(run, format!("לא ניתן ליצור את תיקיית הגיבוי: {e}"));
            }
            native::copy_files(&source, &work, &changed, &log_file, ctx.cancel, &mut |e| on_event(e))
        }
    };

    let mut status = status_of(result.outcome);
    let mut message = result.message;
    apply_stats(&mut run, result.stats);
    run.target_folder = Some(work.to_string_lossy().to_string());
    if log_file.exists() {
        run.log_file = Some(log_file.to_string_lossy().to_string());
    }

    if matches!(status, RunStatus::Success | RunStatus::Warning) {
        match rename_retry(&work, &final_path) {
            Ok(()) => {
                run.target_folder = Some(final_path.to_string_lossy().to_string());
                // Only a completed full removes older backups (unless the user chose delete-before).
                if mode == BackupMode::Full {
                    prune(
                        &task.destination,
                        &prefix,
                        task.keep_count as usize - 1,
                        Some(&file_name(&final_path)),
                        &mut notes,
                    );
                }
            }
            Err(e) => {
                status = RunStatus::Warning;
                message = format!("{message}. לא ניתן לסמן את הגיבוי כמושלם ({e})");
            }
        }
    }
    if !notes.is_empty() {
        message = format!("{message}. {}", notes.join(". "));
    }
    run.status = Some(status);
    run.message = message;
    run
}

/// Runs every source of a task. on_event gets the 0-based source index.
pub fn run_task(
    ctx: &RunContext,
    task: &Task,
    requested: BackupMode,
    log_file_for: &dyn Fn(usize) -> PathBuf,
    on_event: &mut dyn FnMut(usize, EngineEvent),
) -> Vec<SourceRun> {
    let mut out = Vec::new();
    for (i, src) in task.sources.iter().enumerate() {
        if ctx.cancel.load(Ordering::SeqCst) {
            out.push(SourceRun {
                source: src.path.clone(),
                folder_name: src.folder_name.clone(),
                status: Some(RunStatus::Cancelled),
                message: "לא בוצע - הגיבוי בוטל".into(),
                ..Default::default()
            });
            continue;
        }
        on_event(i, EngineEvent::SourceStarted);
        out.push(backup_source(ctx, task, src, requested, log_file_for(i), &mut |e| {
            on_event(i, e)
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::robocopy::Robocopy;

    fn task(srcs: &[&str], dst: &str) -> Task {
        Task {
            name: "t".into(),
            sources: srcs
                .iter()
                .map(|s| Source {
                    path: s.to_string(),
                    folder_name: default_folder_name(s),
                })
                .collect(),
            destination: dst.into(),
            ..Default::default()
        }
    }

    #[test]
    fn rejects_nested_and_duplicate_paths() {
        assert!(validate_task(&task(&["C:\\Data"], "C:\\Data\\Backup")).is_err());
        assert!(validate_task(&task(&["D:\\Backup\\x"], "D:\\Backup")).is_err());
        assert!(validate_task(&task(&["C:\\Data"], "c:\\data\\")).is_err());
        assert!(validate_task(&task(&["C:\\Data"], "D:\\Backup")).is_ok());
        assert!(validate_task(&task(&["C:\\Data"], "C:\\Data2")).is_ok());
        assert!(
            validate_task(&task(&["C:\\A\\Docs", "D:\\Docs"], "E:\\B")).is_err(),
            "same folder name"
        );
        assert!(
            validate_task(&task(&["C:\\Docs", "C:\\docs\\"], "E:\\B")).is_err(),
            "same source twice"
        );
        assert!(validate_task(&task(&["C:\\Docs", "D:\\Photos"], "E:\\B")).is_ok());
    }

    #[test]
    fn folder_names() {
        assert_eq!(default_folder_name("C:\\Users\\me\\Documents\\"), "Documents");
        assert_eq!(default_folder_name("D:\\"), "D");
        assert_eq!(sanitize_folder_name(" a:b/c. "), "a_b_c");
        let (_, kind, partial) = parse_folder("Docs", "Docs 2026-10-01 03-00 אינקרמנטלי").unwrap();
        assert_eq!((kind, partial), (BackupMode::Incremental, false));
        let (_, kind, partial) = parse_folder("Docs", "Docs 2026-10-01 03-00-15 מלא.partial").unwrap();
        assert_eq!((kind, partial), (BackupMode::Full, true));
        let (_, kind, _) = parse_folder("Docs", "Docs 2026-10-01 03-00").unwrap();
        assert_eq!(kind, BackupMode::Full, "legacy name without a tag");
        assert!(parse_folder("Docs", "Docs old").is_none());
        assert!(parse_folder("Docs", "Docs2 2026-10-01 03-00 מלא").is_none());
    }

    struct Env {
        root: PathBuf,
        cancel: AtomicBool,
        filters: Filters,
    }

    impl Env {
        fn new() -> Self {
            Self::with_filters(Filters::default())
        }
        fn with_filters(filters: Filters) -> Self {
            let root = std::env::temp_dir().join(format!("backuper-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            Env {
                root,
                cancel: AtomicBool::new(false),
                filters,
            }
        }
        fn run(&self, t: &Task, mode: BackupMode, at: &str) -> Vec<SourceRun> {
            self.run_with(t, mode, at, &mut |_, _| {})
        }
        fn run_with(&self, t: &Task, mode: BackupMode, at: &str, on_event: &mut dyn FnMut(usize, EngineEvent)) -> Vec<SourceRun> {
            let now = Local
                .from_local_datetime(&NaiveDateTime::parse_from_str(at, "%Y-%m-%d %H:%M").unwrap())
                .unwrap();
            let ctx = RunContext {
                engine: &Robocopy,
                filters: &self.filters,
                now,
                cancel: &self.cancel,
            };
            let logs = self.root.clone();
            run_task(
                &ctx,
                t,
                mode,
                &|i| logs.join(format!("{}-{i}.log", uuid::Uuid::new_v4())),
                on_event,
            )
        }
        fn names(&self, t: &Task, i: usize) -> Vec<String> {
            let mut v: Vec<String> = list_backups(&t.destination, &t.sources[i].folder_name)
                .into_iter()
                .map(|b| b.name)
                .collect();
            v.reverse();
            v
        }
    }

    impl Drop for Env {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// End-to-end against the real robocopy: two sources, same dates, incrementals in their own folders.
    #[test]
    fn chain_of_full_and_incrementals() {
        let env = Env::new();
        let (docs, pics, dst) = (env.root.join("מסמכים"), env.root.join("Pics"), env.root.join("dst"));
        fs::create_dir_all(docs.join("תיקייה")).unwrap();
        fs::create_dir_all(&pics).unwrap();
        fs::write(docs.join("a.txt"), "a").unwrap();
        fs::write(docs.join("תיקייה").join("קובץ.txt"), "שלום").unwrap();
        fs::write(pics.join("p.jpg"), "p").unwrap();
        let t = task(&[docs.to_str().unwrap(), pics.to_str().unwrap()], dst.to_str().unwrap());

        // First incremental -> full for both sources, same date in both folder names.
        let r = env.run(&t, BackupMode::Incremental, "2026-10-01 03:00");
        assert!(r.iter().all(|s| s.status == Some(RunStatus::Success)), "{r:?}");
        assert!(r.iter().all(|s| s.mode == Some(BackupMode::Full)));
        assert_eq!(env.names(&t, 0), ["מסמכים 2026-10-01 03-00 מלא"]);
        assert_eq!(env.names(&t, 1), ["Pics 2026-10-01 03-00 מלא"]);
        assert_eq!(r[0].files_copied, 2);

        // Incremental: a new folder with only the new/changed file.
        fs::write(docs.join("b.txt"), "b").unwrap();
        fs::write(docs.join("a.txt"), "a changed").unwrap();
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!(r[0].mode, Some(BackupMode::Incremental));
        assert_eq!(r[0].files_copied, 2, "{r:?}");
        let inc = dst.join("מסמכים 2026-10-02 03-00 אינקרמנטלי");
        assert!(inc.join("a.txt").exists() && inc.join("b.txt").exists());
        assert!(!inc.join("תיקייה").exists(), "unchanged files are not copied again");
        // Pics had no changes: its incremental folder exists, but is empty.
        let empty = dst.join("Pics 2026-10-02 03-00 אינקרמנטלי");
        assert!(empty.is_dir() && !has_files(&empty));

        // A second incremental only sees changes since the previous incremental.
        let r = env.run(&t, BackupMode::Incremental, "2026-10-03 03:00");
        assert_eq!(r[0].files_copied, 0, "{r:?}");

        // Full: new full folder, the whole previous chain (incl. empty incrementals) is deleted.
        fs::remove_file(docs.join("b.txt")).unwrap();
        let r = env.run(&t, BackupMode::Full, "2026-10-04 03:00");
        assert!(r.iter().all(|s| s.status == Some(RunStatus::Success)), "{r:?}");
        assert_eq!(env.names(&t, 0), ["מסמכים 2026-10-04 03-00 מלא"]);
        assert_eq!(env.names(&t, 1), ["Pics 2026-10-04 03-00 מלא"]);
        assert!(!dst.join("מסמכים 2026-10-04 03-00 מלא").join("b.txt").exists());
    }

    #[test]
    fn keep_two_chains_drops_only_empty_incrementals() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let t = Task {
            keep_count: 2,
            ..task(&[src.to_str().unwrap()], dst.to_str().unwrap())
        };
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        env.run(&t, BackupMode::Incremental, "2026-10-02 03:00"); // empty
        fs::write(src.join("b.txt"), "b").unwrap();
        env.run(&t, BackupMode::Incremental, "2026-10-03 03:00"); // has b.txt
        env.run(&t, BackupMode::Full, "2026-10-04 03:00");
        assert_eq!(
            env.names(&t, 0),
            [
                "Src 2026-10-01 03-00 מלא",
                "Src 2026-10-03 03-00 אינקרמנטלי",
                "Src 2026-10-04 03-00 מלא"
            ]
        );
        env.run(&t, BackupMode::Full, "2026-10-05 03:00");
        assert_eq!(env.names(&t, 0), ["Src 2026-10-04 03-00 מלא", "Src 2026-10-05 03-00 מלא"]);
    }

    #[test]
    fn delete_before_frees_space_first() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let t = Task {
            delete_before: true,
            ..task(&[src.to_str().unwrap()], dst.to_str().unwrap())
        };
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        let old = dst.join("Src 2026-10-01 03-00 מלא");
        assert!(old.is_dir());
        // When copying starts, the previous backup must already be gone.
        let mut old_existed_at_copy = None;
        let r = env.run_with(&t, BackupMode::Full, "2026-10-02 03:00", &mut |_, e| {
            if matches!(e, EngineEvent::Totals { .. }) {
                old_existed_at_copy = Some(old.exists());
            }
        });
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        assert_eq!(old_existed_at_copy, Some(false));
        assert_eq!(env.names(&t, 0), ["Src 2026-10-02 03-00 מלא"]);

        // Without delete-before, the old backup survives until the new one is complete.
        let t = Task {
            delete_before: false,
            ..t
        };
        let prev = dst.join("Src 2026-10-02 03-00 מלא");
        let mut prev_existed_at_copy = None;
        env.run_with(&t, BackupMode::Full, "2026-10-03 03:00", &mut |_, e| {
            if matches!(e, EngineEvent::Totals { .. }) {
                prev_existed_at_copy = Some(prev.exists());
            }
        });
        assert_eq!(prev_existed_at_copy, Some(true));
        assert_eq!(env.names(&t, 0), ["Src 2026-10-03 03-00 מלא"]);
    }

    #[test]
    fn fast_full_reuses_previous_folder() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let t = Task {
            reuse_previous: true,
            ..task(&[src.to_str().unwrap()], dst.to_str().unwrap())
        };
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        fs::write(src.join("c.txt"), "c").unwrap();
        let r = env.run(&t, BackupMode::Full, "2026-10-02 03:00");
        assert_eq!(r[0].files_copied, 1, "only the difference is copied: {r:?}");
        assert_eq!(env.names(&t, 0), ["Src 2026-10-02 03-00 מלא"]);
    }

    #[test]
    fn filter_rules_exclude_files() {
        let env = Env::with_filters(crate::filters::compile(&[
            crate::model::FilterRule::Extension { value: "tmp".into() },
            crate::model::FilterRule::Folder {
                value: "node_modules".into(),
            },
            crate::model::FilterRule::LargerThan { mb: 1 },
            crate::model::FilterRule::Pattern { value: "~$*".into() },
        ]));
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(src.join("node_modules")).unwrap();
        fs::create_dir_all(src.join("docs")).unwrap();
        fs::write(src.join("keep.txt"), "k").unwrap();
        fs::write(src.join("junk.tmp"), "t").unwrap();
        fs::write(src.join("docs").join("~$draft.docx"), "d").unwrap();
        fs::write(src.join("node_modules").join("lib.js"), "x").unwrap();
        fs::write(src.join("big.bin"), vec![0u8; 2 * 1024 * 1024]).unwrap();
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        let check = |b: &Path| {
            assert!(b.join("keep.txt").exists());
            assert!(!b.join("junk.tmp").exists() && !b.join("node_modules").exists() && !b.join("big.bin").exists());
            assert!(!b.join("docs").join("~$draft.docx").exists());
        };
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        check(&dst.join("Src 2026-10-01 03-00 מלא"));
        // The incremental listing applies the same filters.
        fs::write(src.join("new.txt"), "n").unwrap();
        fs::write(src.join("new.tmp"), "n").unwrap();
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!(r[0].files_copied, 1, "{r:?}");
        assert!(dst.join("Src 2026-10-02 03-00 אינקרמנטלי").join("new.txt").exists());
    }
}
