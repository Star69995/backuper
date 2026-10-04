//! Backup orchestration on top of the copy engine: one chain of dated folders per source,
//! full vs incremental, retention, and validation of task paths.
//!
//! Layout, per source, inside the task's destination:
//!   <folderName> 2026-10-01 03-00 מלא            full backup: identical to the source
//!   <folderName> 2026-10-02 03-00 אינקרמנטלי     only files new/changed since the previous backup
//! A folder still being written (or that failed/was cancelled) carries a ".partial" suffix.
//! A full backup plus the incrementals after it form a chain. When a full backup succeeds,
//! chains outside the task's retention (KeepMode) are deleted, and so are empty incremental folders.

use crate::engine::{native, CopyEngine, CopyJob, CopyStats, EngineEvent, Filters, ListedFile, Outcome};
use crate::manifest;
use crate::model::{BackupFolder, BackupMode, KeepMode, RunStatus, Source, SourceBackups, SourceRun, Task};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const DATE_FMT: &str = "%Y-%m-%d %H-%M";
const DATE_FMT_SECS: &str = "%Y-%m-%d %H-%M-%S";
const PARTIAL: &str = ".partial";
const FULL_TAG: &str = "מלא";
const INCREMENTAL_TAG: &str = "אינקרמנטלי";
/// Same tolerance robocopy uses with /FFT (FAT/exFAT store times in 2-second steps), in FILETIME ticks.
const MTIME_TOLERANCE: u64 = 2 * 10_000_000;

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
    match t.keep_mode {
        KeepMode::Count if t.keep_count == 0 => return Err("יש לשמור לפחות גיבוי אחד".into()),
        KeepMode::Days if t.keep_days == 0 => return Err("יש לשמור גיבויים לפחות יום אחד".into()),
        _ => {}
    }
    if let Some(fs) = &t.full_schedule {
        if *fs == crate::model::Schedule::Manual {
            return Err("במצב משולב יש לבחור תזמון לגיבוי המלא".into());
        }
        crate::schedule::validate(fs)?;
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

/// Total size of the files under `path` (unreadable parts count as 0).
fn dir_size(path: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(path) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map(|m| m.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

/// Byte size for messages (same format as `fmtBytes` in the UI).
pub fn fmt_bytes(n: u64) -> String {
    if n == 0 {
        return "0 B".into();
    }
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let i = ((n as f64).ln() / 1024f64.ln()).floor().min(4.0) as usize;
    let v = n as f64 / 1024f64.powi(i as i32);
    if v >= 100.0 || i == 0 {
        format!("{} {}", v.round(), UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Whether a backup folder holds any file (its index file doesn't count).
fn has_files(folder: &Path) -> bool {
    fn any_file(dir: &Path, root: bool) -> bool {
        let Ok(rd) = fs::read_dir(dir) else { return false };
        rd.flatten().any(|e| match e.file_type() {
            Ok(t) if t.is_dir() => any_file(&e.path(), false),
            Ok(_) => !(root && e.file_name().eq_ignore_ascii_case(manifest::FILE_NAME)),
            Err(_) => false,
        })
    }
    any_file(folder, true)
}

/// Size of a backup's files, from its index when it has one (no walk of the destination).
fn backup_size(folder: &Path) -> u64 {
    match manifest::read(folder) {
        Some(files) => files.iter().map(|f| f.size).sum(),
        None => dir_size(folder),
    }
}

/// Which complete full backups (and so which chains) `prune` keeps, besides the protected one.
#[derive(Clone, Copy, Debug)]
enum Keep {
    /// The newest n.
    Fulls(usize),
    /// Every full newer than the cutoff, plus the newest one before it (its chain covers the
    /// cutoff moment). A chain goes only once the next full is older than the cutoff too.
    Since(DateTime<Local>),
    All,
}

impl Keep {
    /// `protected` = the new full backup counts as one of the kept ones.
    fn of(task: &Task, now: DateTime<Local>, protected: bool) -> Keep {
        match task.keep_mode {
            KeepMode::Count => Keep::Fulls((task.keep_count as usize).saturating_sub(protected as usize)),
            KeepMode::Days => Keep::Since(now - chrono::Duration::days(task.keep_days as i64)),
            KeepMode::All => Keep::All,
        }
    }
}

/// Deletes old backups of one source. Keeps the complete full backups that `keep` selects
/// (besides `protect`) together with their incrementals; everything else goes: older chains,
/// partial folders, and (with `delete_empty`) incremental folders that are empty.
/// Returns how many backups were deleted and how many bytes that freed.
fn prune(dest: &str, prefix: &str, keep: Keep, protect: Option<&str>, delete_empty: bool, notes: &mut Vec<String>) -> (u64, u64) {
    let all: Vec<BackupFolder> = list_backups(dest, prefix)
        .into_iter()
        .filter(|b| Some(b.name.as_str()) != protect)
        .collect();
    let fulls = all.iter().filter(|b| b.kind == BackupMode::Full && !b.partial);
    let kept_fulls: Vec<&BackupFolder> = match keep {
        Keep::Fulls(n) => fulls.take(n).collect(),
        Keep::All => fulls.collect(),
        Keep::Since(cutoff) => {
            let (newer, older): (Vec<&BackupFolder>, Vec<&BackupFolder>) = fulls.partition(|b| b.created_at >= cutoff);
            newer.into_iter().chain(older.into_iter().take(1)).collect()
        }
    };
    let oldest_kept = kept_fulls.last().map(|b| b.created_at);
    let mut removed_empty = 0;
    let mut deleted = 0;
    let mut freed = 0;
    for b in &all {
        let keep = match b.kind {
            BackupMode::Full => kept_fulls.iter().any(|k| k.name == b.name),
            BackupMode::Incremental => {
                let in_kept_chain = !b.partial && oldest_kept.is_some_and(|d| b.created_at > d);
                in_kept_chain && !(delete_empty && !has_files(Path::new(&b.path)))
            }
        };
        if keep {
            continue;
        }
        let empty = b.kind == BackupMode::Incremental && !has_files(Path::new(&b.path));
        let size = if empty { 0 } else { backup_size(Path::new(&b.path)) };
        match fs::remove_dir_all(&b.path) {
            Ok(()) if empty => removed_empty += 1,
            Ok(()) => {
                deleted += 1;
                freed += size;
                notes.push(format!("נמחק גיבוי קודם: {} ({})", b.name, fmt_bytes(size)));
            }
            Err(e) => notes.push(format!("לא ניתן למחוק את {}: {e}", b.name)),
        }
    }
    if removed_empty > 0 {
        notes.push(format!("נמחקו {removed_empty} תיקיות אינקרמנטליות ריקות"));
    }
    (deleted, freed)
}

/// Decides whether this source's run is full or incremental (incremental needs a complete full).
fn effective_mode(backups: &[BackupFolder], requested: BackupMode) -> (BackupMode, Option<String>) {
    if requested == BackupMode::Full {
        return (BackupMode::Full, None);
    }
    match backups.iter().find(|b| b.kind == BackupMode::Full && !b.partial) {
        None => (BackupMode::Full, Some("אין גיבוי מלא קודם - מבוצע גיבוי מלא".into())),
        Some(_) => (BackupMode::Incremental, None),
    }
}

/// What is already backed up: rel path (lowercase) -> (size, modified), from the latest complete
/// full backup overlaid with every incremental after it, read from each folder's index file.
fn chain_index(backups: &[BackupFolder]) -> HashMap<String, (u64, u64)> {
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
        for f in manifest::load_or_build(Path::new(&b.path)) {
            index.insert(f.rel.to_lowercase(), (f.size, f.modified));
        }
    }
    index
}

fn changed_since(listed: Vec<ListedFile>, index: &HashMap<String, (u64, u64)>) -> Vec<ListedFile> {
    listed
        .into_iter()
        .filter(|f| match index.get(&f.rel.to_lowercase()) {
            None => true,
            Some(&(size, modified)) => size != f.size || modified.abs_diff(f.modified) > MTIME_TOLERANCE,
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
        return fail(
            run,
            format!(
                "תיקיית המקור לא נמצאה: {}. ייתכן שהיא נמחקה, הועברה או ששמה שונה, או שהכונן שלה לא מחובר. חברו את הכונן, או עדכנו את הנתיב בהגדרות המשימה.",
                source.display()
            ),
        );
    }
    if let Err(e) = fs::create_dir_all(&dest) {
        return fail(
            run,
            format!(
                "היעד לא זמין ({}): {e}. ודאו שכונן היעד מחובר ושיש הרשאה לכתוב אליו, והריצו את הגיבוי שוב.",
                dest.display()
            ),
        );
    }

    let backups = list_backups(&task.destination, &prefix);
    let (mode, note) = effective_mode(&backups, requested);
    run.mode = Some(mode);
    let mut notes: Vec<String> = note.into_iter().collect();
    let final_path = new_folder_path(&dest, &prefix, ctx.now, mode);
    let work = with_partial(&final_path);

    let result = match mode {
        BackupMode::Full => {
            // Fast full: reuse the newest full folder so only differences are copied.
            // Not with an include rule: robocopy only purges names matching it, so files that
            // no longer match would stay in the "full" folder.
            let latest_full = backups.iter().find(|b| b.kind == BackupMode::Full && !b.partial);
            let reused = task.reuse_previous
                && task.keep_mode == KeepMode::Count
                && task.keep_count == 1
                && ctx.filters.include_files.is_empty()
                && latest_full.is_some_and(|prev| match rename_retry(Path::new(&prev.path), &work) {
                    Ok(()) => {
                        manifest::remove(&work);
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
                let (deleted, freed) = prune(
                    &task.destination,
                    &prefix,
                    Keep::of(task, ctx.now, true),
                    Some(&file_name(&work)),
                    task.delete_empty_incrementals,
                    &mut notes,
                );
                run.freed_bytes += freed;
                if deleted > 0 {
                    notes.push("הגיבויים הקודמים נמחקו לפני תחילת הגיבוי (לפי הגדרות המשימה)".into());
                }
            }
            if !reused {
                if let Err(e) = fs::create_dir_all(&work) {
                    return fail(
                        run,
                        format!("לא ניתן ליצור את תיקיית הגיבוי: {e}. ודאו שכונן היעד מחובר, שיש בו מקום ושיש הרשאה לכתוב אליו."),
                    );
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
            let listing = match native::list_files(&source, ctx.filters, ctx.cancel) {
                Ok(Some(l)) => l,
                Ok(None) => {
                    run.status = Some(RunStatus::Cancelled);
                    run.message = "הגיבוי בוטל על ידי המשתמש".into();
                    return run;
                }
                Err(e) => {
                    return fail(
                        run,
                        format!("לא ניתן לסרוק את תיקיית המקור: {e}. ודאו שהכונן מחובר ושיש הרשאה לקרוא את התיקייה, והריצו את הגיבוי שוב."),
                    )
                }
            };
            let changed = changed_since(listing.files, &chain_index(&backups));
            if let Err(e) = fs::create_dir_all(&work) {
                return fail(
                    run,
                    format!("לא ניתן ליצור את תיקיית הגיבוי: {e}. ודאו שכונן היעד מחובר, שיש בו מקום ושיש הרשאה לכתוב אליו."),
                );
            }
            let mut result = native::copy_files(&source, &work, &changed, &log_file, ctx.cancel, &mut |e| on_event(e));
            // Unreadable folders in the source: their files weren't backed up.
            if listing.failed_dirs > 0 {
                notes.push(match listing.failed_dirs {
                    1 => "תיקייה אחת במקור לא נסרקה והקבצים שבה לא גובו (פירוט ברשימת השגיאות)".to_string(),
                    n => format!("{n} תיקיות במקור לא נסרקו והקבצים שבהן לא גובו (פירוט ברשימת השגיאות)"),
                });
                let mut errors = listing.errors;
                errors.append(&mut result.stats.errors);
                errors.truncate(50);
                result.stats.errors = errors;
                if result.outcome == Outcome::Success {
                    result.outcome = Outcome::Warning;
                }
            }
            result
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
                let (files, complete) = manifest::scan(&final_path);
                run.backup_bytes = files.iter().map(|f| f.size).sum();
                if complete {
                    manifest::write(&final_path, &files);
                }
                // Only a completed full removes older backups (unless the user chose delete-before).
                // Keeping by days also lets an incremental remove chains that have expired since.
                if mode == BackupMode::Full || task.keep_mode == KeepMode::Days {
                    let (_, freed) = prune(
                        &task.destination,
                        &prefix,
                        Keep::of(task, ctx.now, mode == BackupMode::Full),
                        Some(&file_name(&final_path)),
                        task.delete_empty_incrementals && mode == BackupMode::Full,
                        &mut notes,
                    );
                    run.freed_bytes += freed;
                }
            }
            Err(e) => {
                status = RunStatus::Warning;
                message = format!("{message}. לא ניתן לסמן את הגיבוי כמושלם ({e})");
            }
        }
    }
    // An incremental with no changes stays quiet about its size.
    if run.backup_bytes > 0 && (mode == BackupMode::Full || run.files_copied > 0) {
        notes.push(format!("גודל הגיבוי: {}", fmt_bytes(run.backup_bytes)));
    }
    if run.freed_bytes > 0 {
        notes.push(format!("פונו {} בכונן היעד", fmt_bytes(run.freed_bytes)));
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
        assert_eq!((r[0].backup_bytes, r[0].freed_bytes), (1 + 8, 0), "a.txt + קובץ.txt");

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
        // Size of the new full, and what deleting the old chain freed (full 9 + incremental 10).
        assert_eq!(r[0].backup_bytes, 9 + 8);
        assert_eq!(r[0].freed_bytes, 9 + 10);
        assert!(r[0].message.contains("פונו 19 B"), "{}", r[0].message);
    }

    /// Read-only + hidden + system files: backed up, and their backup folders can still be deleted.
    #[test]
    fn protected_files_are_copied_and_pruned() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        let attrib = |p: &Path, flags: [&str; 3]| {
            let ok = std::process::Command::new("attrib").args(flags).arg(p).status().unwrap();
            assert!(ok.success());
        };
        fs::write(src.join("sys.dat"), "s").unwrap();
        attrib(&src.join("sys.dat"), ["+R", "+H", "+S"]);
        let t = Task {
            reuse_previous: true,
            ..task(&[src.to_str().unwrap()], dst.to_str().unwrap())
        };
        let r = env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        assert!(dst.join("Src 2026-10-01 03-00 מלא").join("sys.dat").exists());

        // Incremental copies a new protected file.
        fs::write(src.join("sys2.dat"), "s2").unwrap();
        attrib(&src.join("sys2.dat"), ["+R", "+H", "+S"]);
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!((r[0].status, r[0].files_copied), (Some(RunStatus::Success), 1), "{r:?}");

        // Reused full: robocopy must delete the protected file that's gone from the source,
        // and pruning must delete the incremental folder that holds a protected file.
        attrib(&src.join("sys.dat"), ["-R", "-H", "-S"]);
        fs::remove_file(src.join("sys.dat")).unwrap();
        let r = env.run(&t, BackupMode::Full, "2026-10-03 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        assert_eq!(env.names(&t, 0), ["Src 2026-10-03 03-00 מלא"]);
        let full = dst.join("Src 2026-10-03 03-00 מלא");
        assert!(!full.join("sys.dat").exists() && full.join("sys2.dat").exists());
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
    fn keep_all_never_deletes_chains() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let t = Task {
            keep_mode: KeepMode::All,
            reuse_previous: true, // ignored: it would overwrite the previous full
            ..task(&[src.to_str().unwrap()], dst.to_str().unwrap())
        };
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        fs::write(src.join("b.txt"), "b").unwrap();
        env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        env.run(&t, BackupMode::Full, "2026-10-03 03:00");
        env.run(&t, BackupMode::Full, "2026-10-04 03:00");
        assert_eq!(
            env.names(&t, 0),
            [
                "Src 2026-10-01 03-00 מלא",
                "Src 2026-10-02 03-00 אינקרמנטלי",
                "Src 2026-10-03 03-00 מלא",
                "Src 2026-10-04 03-00 מלא"
            ]
        );
    }

    #[test]
    fn keep_days_keeps_what_covers_the_window() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let t = Task {
            keep_mode: KeepMode::Days,
            keep_days: 7,
            ..task(&[src.to_str().unwrap()], dst.to_str().unwrap())
        };
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        fs::write(src.join("b.txt"), "b").unwrap();
        env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        env.run(&t, BackupMode::Full, "2026-10-05 03:00");
        // Cutoff 10-03: the 10-01 chain is still needed to restore 10-03 and 10-04.
        env.run(&t, BackupMode::Full, "2026-10-10 03:00");
        assert_eq!(
            env.names(&t, 0),
            [
                "Src 2026-10-01 03-00 מלא",
                "Src 2026-10-02 03-00 אינקרמנטלי",
                "Src 2026-10-05 03-00 מלא",
                "Src 2026-10-10 03-00 מלא"
            ]
        );
        // Cutoff 10-06: the 10-05 full covers it, so the 10-01 chain goes - even on an incremental.
        fs::write(src.join("c.txt"), "c").unwrap();
        env.run(&t, BackupMode::Incremental, "2026-10-13 03:00");
        assert_eq!(
            env.names(&t, 0),
            [
                "Src 2026-10-05 03-00 מלא",
                "Src 2026-10-10 03-00 מלא",
                "Src 2026-10-13 03-00 אינקרמנטלי"
            ]
        );
    }

    #[test]
    fn empty_incrementals_kept_when_option_off() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let t = Task {
            keep_count: 2,
            delete_empty_incrementals: false,
            ..task(&[src.to_str().unwrap()], dst.to_str().unwrap())
        };
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        env.run(&t, BackupMode::Incremental, "2026-10-02 03:00"); // empty
        env.run(&t, BackupMode::Full, "2026-10-03 03:00");
        assert_eq!(
            env.names(&t, 0),
            [
                "Src 2026-10-01 03-00 מלא",
                "Src 2026-10-02 03-00 אינקרמנטלי",
                "Src 2026-10-03 03-00 מלא"
            ]
        );
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
        assert_eq!(r[0].files_deleted, 0, "the old index file isn't reported as a deleted file");
        assert_eq!(env.names(&t, 0), ["Src 2026-10-02 03-00 מלא"]);
        let index = manifest::read(&dst.join("Src 2026-10-02 03-00 מלא")).unwrap();
        assert_eq!(index.len(), 2, "the reused folder gets a fresh index");
    }

    #[test]
    fn incrementals_use_the_backup_index() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        fs::write(src.join("sub").join("b.txt"), "b").unwrap();
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        let r = env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        assert_eq!(r[0].backup_bytes, 2, "the index file isn't counted");
        let full = dst.join("Src 2026-10-01 03-00 מלא");
        let mut index: Vec<String> = manifest::read(&full).unwrap().into_iter().map(|f| f.rel).collect();
        index.sort();
        assert_eq!(index, ["a.txt", "sub\\b.txt"]);

        // A folder without an index (older version) is walked, and gets one.
        fs::remove_file(full.join(manifest::FILE_NAME)).unwrap();
        fs::write(src.join("c.txt"), "c").unwrap();
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!(r[0].files_copied, 1, "{r:?}");
        assert!(manifest::read(&full).is_some());
        let inc = dst.join("Src 2026-10-02 03-00 אינקרמנטלי");
        assert_eq!(manifest::read(&inc).unwrap().len(), 1);

        // A cut-off index doesn't count.
        let path = full.join(manifest::FILE_NAME);
        let text = fs::read_to_string(&path).unwrap();
        crate::tasklist::set_attributes(&path, crate::tasklist::NORMAL);
        fs::write(&path, &text[..text.len() - 8]).unwrap();
        assert!(manifest::read(&full).is_none());
        let r = env.run(&t, BackupMode::Incremental, "2026-10-03 03:00");
        assert_eq!(r[0].files_copied, 0, "{r:?}");
        assert!(manifest::read(&full).is_some(), "rebuilt");
    }

    /// A source file that happens to have the index file's name is backed up as is.
    #[test]
    fn source_file_named_like_the_index_is_kept() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join(manifest::FILE_NAME), "user data").unwrap();
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        let full = dst.join("Src 2026-10-01 03-00 מלא");
        assert_eq!(fs::read_to_string(full.join(manifest::FILE_NAME)).unwrap(), "user data");
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
    }

    #[test]
    fn fast_full_skipped_with_include_rule() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.lrcat"), "c").unwrap();
        fs::write(src.join("big.lrdata"), "p").unwrap();
        let t = Task {
            reuse_previous: true,
            ..task(&[src.to_str().unwrap()], dst.to_str().unwrap())
        };
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        // The rule is added after a full backup without it.
        let with_rule = Env {
            root: env.root.clone(),
            cancel: AtomicBool::new(false),
            filters: crate::filters::compile(&[crate::model::FilterRule::Include { value: "*.lrcat".into() }]),
        };
        with_rule.run(&t, BackupMode::Full, "2026-10-02 03:00");
        let b = dst.join("Src 2026-10-02 03-00 מלא");
        assert!(b.join("a.lrcat").exists());
        assert!(
            !b.join("big.lrdata").exists(),
            "a file that no longer matches must not stay in the full backup"
        );
        assert_eq!(with_rule.names(&t, 0), ["Src 2026-10-02 03-00 מלא"]);
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

    #[test]
    fn include_rule_backs_up_only_matching_files() {
        let env = Env::with_filters(crate::filters::compile(&[crate::model::FilterRule::Include {
            value: "*.lrcat, notes.txt".into(),
        }]));
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("main.lrcat"), "c").unwrap();
        fs::write(src.join("sub").join("old.lrcat"), "c").unwrap();
        fs::write(src.join("sub").join("notes.txt"), "n").unwrap();
        fs::write(src.join("preview.lrdata"), "p").unwrap();
        fs::write(src.join("sub").join("other.txt"), "o").unwrap();
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        let r = env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        assert_eq!(r[0].files_copied, 3, "{r:?}");
        let b = dst.join("Src 2026-10-01 03-00 מלא");
        assert!(b.join("main.lrcat").exists() && b.join("sub").join("old.lrcat").exists());
        assert!(b.join("sub").join("notes.txt").exists());
        assert!(!b.join("preview.lrdata").exists() && !b.join("sub").join("other.txt").exists());
        // The incremental listing applies the same rule.
        fs::write(src.join("new.lrcat"), "n").unwrap();
        fs::write(src.join("new.jpg"), "n").unwrap();
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!(r[0].files_copied, 1, "{r:?}");
        assert!(dst.join("Src 2026-10-02 03-00 אינקרמנטלי").join("new.lrcat").exists());
    }
}
