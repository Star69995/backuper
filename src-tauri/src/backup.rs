//! Backup orchestration on top of the copy engine: one chain of dated folders per source,
//! full vs incremental, retention, and validation of task paths.
//!
//! Layout, per source, inside the task's destination:
//!   <folderName> 2026-10-01 03h00 מלא            full backup: identical to the source
//!   <folderName> 2026-10-02 03h00 אינקרמנטלי     only files new/changed since the previous backup
//! Older versions wrote the time with hyphens ("03-00"). Those names are still read, and
//! `modernize_names` renames them at the start of the task's next run.
//! A folder still being written (or that failed/was stopped) carries a ".partial" suffix.
//! The next run of the same kind carries on in it instead of starting over (see `backup_source`).
//! A full backup plus the incrementals after it form a chain. When a full backup succeeds,
//! chains outside the task's retention (KeepMode) are deleted, and so are empty incremental folders.

use crate::engine::{native, CopyEngine, CopyJob, CopyStats, EngineEvent, Filters, ListedFile, Outcome, TotalsSource};
use crate::manifest;
use crate::model::{BackupFolder, BackupMode, FullSpace, KeepMode, RunRecord, RunStatus, Source, SourceBackups, SourceRun, Task};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};

/// "03h15" (not "03;15" or "03_15", which show as "15;03" inside Hebrew text: only "-", "." and
/// "," keep the two numbers together; a letter does too).
const DATE_FMT: &str = "%Y-%m-%d %Hh%M";
const DATE_FMT_SECS: &str = "%Y-%m-%d %Hh%Mm%Ss";
/// The time format of older versions ("03-00"): still read, renamed by `modernize_names`.
const OLD_DATE_FMT: &str = "%Y-%m-%d %H-%M";
const OLD_DATE_FMT_SECS: &str = "%Y-%m-%d %H-%M-%S";
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

pub fn normalize(p: &str) -> String {
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

/// A dated folder's name, read.
struct Dated {
    at: NaiveDateTime,
    kind: BackupMode,
    partial: bool,
    /// The time has seconds ("03h00m15s").
    seconds: bool,
    /// Written by an older version: hyphens in the time, or no type tag.
    old: bool,
}

fn parse_dated(prefix: &str, name: &str) -> Option<Dated> {
    let rest = name.strip_prefix(prefix)?.strip_prefix(' ')?;
    let (rest, partial) = match rest.strip_suffix(PARTIAL) {
        Some(r) => (r, true),
        None => (rest, false),
    };
    let (date, kind, tagged) = if let Some(d) = rest.strip_suffix(INCREMENTAL_TAG).and_then(|d| d.strip_suffix(' ')) {
        (d, BackupMode::Incremental, true)
    } else if let Some(d) = rest.strip_suffix(FULL_TAG).and_then(|d| d.strip_suffix(' ')) {
        (d, BackupMode::Full, true)
    } else {
        // Folders from before the type tag existed are full backups. They only ever had hyphens in
        // the time, so no other format is read without a tag (it could be one of Cobian's folders).
        (rest, BackupMode::Full, false)
    };
    // (format, has seconds)
    let parse = |fmts: [(&str, bool); 2]| {
        fmts.into_iter()
            .find_map(|(fmt, secs)| NaiveDateTime::parse_from_str(date, fmt).ok().map(|at| (at, secs)))
    };
    let current = if tagged { parse([(DATE_FMT_SECS, true), (DATE_FMT, false)]) } else { None };
    let (at, seconds, old) = match current {
        Some((at, secs)) => (at, secs, !tagged),
        None => {
            let (at, secs) = parse([(OLD_DATE_FMT_SECS, true), (OLD_DATE_FMT, false)])?;
            (at, secs, true)
        }
    };
    Some(Dated { at, kind, partial, seconds, old })
}

pub fn parse_folder(prefix: &str, name: &str) -> Option<(DateTime<Local>, BackupMode, bool)> {
    let d = parse_dated(prefix, name)?;
    Some((Local.from_local_datetime(&d.at).earliest()?, d.kind, d.partial))
}

/// The current name of a folder named by an older version (`None` = already current, or not one of
/// ours). Same date, time and type; an untagged legacy folder gets the full tag.
fn modern_name(prefix: &str, name: &str) -> Option<String> {
    let d = parse_dated(prefix, name).filter(|d| d.old)?;
    let new = dated_folder_name(prefix, d.at, d.kind, d.seconds);
    Some(if d.partial { format!("{new}{PARTIAL}") } else { new })
}

/// Renames this source's folders in `dest` that still carry an older version's name ("03-00") to
/// the current format ("03h00"); a completed folder's index is updated with it, so it isn't walked
/// again. Best effort: a folder that can't be renamed now (open in another program, the name is
/// taken) keeps its old name, which is still read, and gets another try on the next run.
pub fn modernize_names(dest: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(dest) else { return };
    for e in entries.flatten() {
        if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        let Some(new) = modern_name(prefix, &name) else { continue };
        let to = dest.join(&new);
        if to.exists() || fs::rename(e.path(), &to).is_err() {
            continue;
        }
        if !new.ends_with(PARTIAL) {
            manifest::renamed(&to, &name);
        }
    }
}

/// Where a folder recorded under `name` in `dir` is now: `name` itself, or its current name when
/// only that exists (renamed by `modernize_names`).
pub fn current_name(dir: &Path, name: &str) -> String {
    if !dir.join(name).exists() {
        let modern = modernize_any(name);
        if let Some(m) = modern.filter(|m| dir.join(m).exists()) {
            return m;
        }
    }
    name.to_string()
}

/// `modern_name` without knowing the prefix: the date is right before the tag.
fn modernize_any(name: &str) -> Option<String> {
    let base = name.strip_suffix(PARTIAL).unwrap_or(name);
    let (head, _) = base.rsplit_once(' ')?;
    // "2026-10-01 03-00-15" or "2026-10-01 03-00".
    [19, 16].into_iter().find_map(|len| {
        let cut = head.len().checked_sub(len)?;
        let prefix = head.get(..cut)?.strip_suffix(' ')?;
        modern_name(prefix, name)
    })
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
                bytes: None,
            })
        })
        .collect();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then_with(|| b.name.cmp(&a.name)));
    out
}

/// The `.partial` folder of `kind` to continue. With several (a run that started over, or a new one
/// begun before this was fixed) the one holding the most data, so the most work is kept; sizes are
/// only read then, as one walk per candidate. The newest wins a tie.
fn fullest_partial(backups: &[BackupFolder], kind: BackupMode) -> Option<&BackupFolder> {
    let mut candidates = backups.iter().filter(|b| b.partial && b.kind == kind);
    let first = candidates.next()?;
    let rest: Vec<&BackupFolder> = candidates.collect();
    if rest.is_empty() {
        return Some(first);
    }
    let size = |b: &BackupFolder| manifest::scan(Path::new(&b.path)).0.iter().map(|f| f.size).sum::<u64>();
    let mut best = (size(first), first);
    for b in rest {
        let s = size(b);
        if s > best.0 {
            best = (s, b);
        }
    }
    Some(best.1)
}

/// Whether any of the task's sources has a folder left over from a paused or failed run.
pub fn has_partial(task: &Task) -> bool {
    task.sources
        .iter()
        .any(|s| list_backups(&task.destination, &s.folder_name).iter().any(|b| b.partial))
}

/// Gives up on a paused backup: deletes the task's `.partial` folders. A partial full is left alone
/// when no complete full sits next to it in that source (it may be the previous full, updated in
/// place, i.e. the only backup there is). Returns how many folders were left, so the caller knows
/// whether anything is still there to continue.
pub fn discard_partials(task: &Task) -> usize {
    let mut left = 0;
    for s in &task.sources {
        let backups = list_backups(&task.destination, &s.folder_name);
        let has_full = backups.iter().any(|b| !b.partial && b.kind == BackupMode::Full);
        for b in backups.iter().filter(|b| b.partial) {
            let safe = b.kind == BackupMode::Incremental || has_full;
            if !safe || fs::remove_dir_all(&b.path).is_err() {
                left += 1;
            }
        }
    }
    left
}

/// Every source's backups on disk, each with its size: from its index when it has one, else one
/// fast listing (never per-file stats). A partial folder has no index and may still be growing,
/// so its size is left for the user to ask for.
pub fn list_task_backups(task: &Task) -> Vec<SourceBackups> {
    task.sources
        .iter()
        .map(|s| {
            let mut backups = list_backups(&task.destination, &s.folder_name);
            std::thread::scope(|scope| {
                for b in backups.iter_mut().filter(|b| !b.partial) {
                    scope.spawn(move || {
                        let folder = Path::new(&b.path);
                        b.bytes = Some(match manifest::read(folder) {
                            Some(files) => files.iter().map(|f| f.size).sum(),
                            None => manifest::scan(folder).0.iter().map(|f| f.size).sum(),
                        });
                    });
                }
            });
            SourceBackups { source: s.path.clone(), folder_name: s.folder_name.clone(), backups }
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

/// A dated folder's name: "<prefix> 2026-10-01 03h00 מלא" (with seconds: "... 03h00m15s מלא").
pub fn dated_folder_name(prefix: &str, at: NaiveDateTime, kind: BackupMode, seconds: bool) -> String {
    let tag = if kind == BackupMode::Full { FULL_TAG } else { INCREMENTAL_TAG };
    let fmt = if seconds { DATE_FMT_SECS } else { DATE_FMT };
    format!("{prefix} {} {tag}", at.format(fmt))
}

fn new_folder_path(dest: &Path, prefix: &str, now: DateTime<Local>, kind: BackupMode) -> PathBuf {
    let taken = |n: &str| dest.join(n).exists() || dest.join(format!("{n}{PARTIAL}")).exists();
    let name = dated_folder_name(prefix, now.naive_local(), kind, false);
    if !taken(&name) {
        return dest.join(name);
    }
    dest.join(dated_folder_name(prefix, now.naive_local(), kind, true))
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
pub fn rename_retry(from: &Path, to: &Path) -> std::io::Result<()> {
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

/// What `prune` deletes from `all` (newest first): keeps the complete full backups that `keep`
/// selects together with their incrementals; everything else goes: older chains, partial
/// folders, and (with `delete_empty`) incremental folders that are empty.
fn prune_victims(all: &[BackupFolder], keep: Keep, delete_empty: bool) -> Vec<&BackupFolder> {
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
    all.iter()
        .filter(|b| match b.kind {
            BackupMode::Full => !kept_fulls.iter().any(|k| k.name == b.name),
            BackupMode::Incremental => {
                let in_kept_chain = !b.partial && oldest_kept.is_some_and(|d| b.created_at > d);
                !in_kept_chain || (delete_empty && !has_files(Path::new(&b.path)))
            }
        })
        .collect()
}

/// Deletes old backups of one source (`prune_victims`, besides `protect`).
/// Returns how many backups were deleted and how many bytes that freed.
fn prune(dest: &str, prefix: &str, keep: Keep, protect: Option<&str>, delete_empty: bool, notes: &mut Vec<String>) -> (u64, u64) {
    let all: Vec<BackupFolder> = list_backups(dest, prefix)
        .into_iter()
        .filter(|b| Some(b.name.as_str()) != protect)
        .collect();
    let mut removed_empty = 0;
    let mut deleted = 0;
    let mut freed = 0;
    for b in prune_victims(&all, keep, delete_empty) {
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

/// A full backup renames the previous full folder and updates it in place (when no include
/// filter stands in the way, which the run checks) instead of copying everything again.
fn reuses_previous_full(task: &Task) -> bool {
    task.reuse_previous && task.keep_mode == KeepMode::Count && task.keep_count == 1
}

/// The space the task's next full backup needs on the destination, to compare with what is free
/// (`None` = no full backup is expected, or nothing to compare with). A full backup needs about
/// what the previous full weighed, less what delete-before removes first. Sizes come from the
/// index files only, never a walk (this is polled by the UI). `full_next` = the user asked for the
/// next run to be a full one (`TaskState.full_next`).
pub fn full_space(task: &Task, now: DateTime<Local>, full_next: bool) -> Option<FullSpace> {
    // Incremental-only tasks make a full backup just when a source has none yet (or when asked for one).
    let only_incremental = task.mode == BackupMode::Incremental && task.full_schedule.is_none() && !full_next;
    let (mut needed, mut freed) = (0u64, 0u64);
    for s in &task.sources {
        let all = list_backups(&task.destination, &s.folder_name);
        let Some(prev) = all.iter().find(|b| b.kind == BackupMode::Full && !b.partial) else {
            continue; // the first full: nothing to compare with
        };
        if only_incremental || reuses_previous_full(task) {
            continue;
        }
        let size = |b: &BackupFolder| manifest::read(Path::new(&b.path)).map_or(0, |f| f.iter().map(|f| f.size).sum());
        needed += size(prev);
        if task.delete_before {
            let victims = prune_victims(&all, Keep::of(task, now, true), task.delete_empty_incrementals);
            freed += victims.into_iter().filter(|b| !b.partial).map(size).sum::<u64>();
        }
    }
    (needed > 0).then_some(FullSpace { needed, freed })
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
        .filter(|b| b.kind == BackupMode::Incremental && !b.partial && b.created_at > base.created_at)
        .collect();
    chain.push(base);
    chain.reverse(); // oldest first, so newer copies override
    for b in chain {
        add_to_index(&mut index, manifest::load_or_build(Path::new(&b.path)));
    }
    index
}

fn add_to_index(index: &mut HashMap<String, (u64, u64)>, files: impl IntoIterator<Item = ListedFile>) {
    for f in files {
        index.insert(f.rel.to_lowercase(), (f.size, f.modified));
    }
}

/// New or changed compared with what is already there (size, or modified time beyond the tolerance).
fn differs(f: &ListedFile, index: &HashMap<String, (u64, u64)>) -> bool {
    match index.get(&f.rel.to_lowercase()) {
        None => true,
        Some(&(size, modified)) => size != f.size || modified.abs_diff(f.modified) > MTIME_TOLERANCE,
    }
}

fn changed_since(listed: Vec<ListedFile>, index: &HashMap<String, (u64, u64)>) -> Vec<ListedFile> {
    listed.into_iter().filter(|f| differs(f, index)).collect()
}

/// Resuming an incremental in its partial folder: files already there that match the source are
/// done (returned count); anything else in it (an older version, a file that no longer counts as
/// changed) is removed, so the finished folder holds exactly the changes.
fn skip_done(folder: &Path, changed: Vec<ListedFile>) -> (Vec<ListedFile>, u64) {
    let (present, _) = manifest::scan(folder);
    let wanted: HashMap<String, (u64, u64)> = changed
        .iter()
        .map(|f| (f.rel.to_lowercase(), (f.size, f.modified)))
        .collect();
    let mut done = HashSet::new();
    for f in present {
        let key = f.rel.to_lowercase();
        match wanted.get(&key) {
            Some(&(size, modified)) if size == f.size && modified.abs_diff(f.modified) <= MTIME_TOLERANCE => {
                done.insert(key);
            }
            _ => {
                let path = folder.join(&f.rel);
                crate::tasklist::set_attributes(&path, crate::tasklist::NORMAL);
                let _ = fs::remove_file(path);
            }
        }
    }
    let count = done.len() as u64;
    let left = changed.into_iter().filter(|f| !done.contains(&f.rel.to_lowercase())).collect();
    (left, count)
}

/// robocopy gives the backup folder the attributes of the source folder, and a drive root is
/// hidden + system: Explorer would not show the backup folder at all. Clear them.
fn unhide(folder: &Path) {
    if folder.is_dir() {
        crate::tasklist::set_attributes(folder, crate::tasklist::NORMAL);
    }
}

/// A full stop (as opposed to a pause) throws the half-done folder away, so there is nothing to
/// continue. Not when `safe` is false: that folder is then the last complete backup, a previous
/// full updated in place. Returns whether the folder is gone.
fn discard_stopped(ctx: &RunContext, work: &Path, safe: bool) -> bool {
    if !safe || !ctx.discard.load(Ordering::SeqCst) {
        return false;
    }
    if work.exists() {
        let _ = fs::remove_dir_all(work);
    }
    !work.exists()
}

/// What a stopped run says. When its folder is left behind, it can be continued.
fn stopped_message(work: &Path, discarded: bool) -> String {
    if discarded {
        "הגיבוי נעצר. ההעתקה החלקית נמחקה".into()
    } else if work.is_dir() {
        "הגיבוי הושהה. אפשר להמשיך אותו מאותה נקודה".into()
    } else {
        "הגיבוי הושהה".into()
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

fn apply_stats(run: &mut SourceRun, stats: CopyStats) {
    run.files_copied = stats.files_copied;
    run.bytes_copied = stats.bytes_copied;
    run.files_deleted = stats.files_deleted;
    run.files_failed = stats.files_failed;
    run.errors = stats.errors;
    run.exit_code = stats.exit_code;
    run.debug = stats.debug;
}

pub struct RunContext<'a> {
    pub engine: &'a dyn CopyEngine,
    pub filters: &'a Filters,
    /// The same timestamp for every source of the run (folder names share the date).
    pub now: DateTime<Local>,
    pub cancel: &'a AtomicBool,
    /// With `cancel`: stop for good (delete the half-done folder) instead of pausing.
    pub discard: &'a AtomicBool,
    /// Don't continue a leftover `.partial` folder: start a new one ("start over").
    pub fresh: bool,
    /// A scheduled or catch-up run: starts a new cycle, so it leaves a `.partial` folder alone - unless
    /// there is no complete full backup yet, when that folder is all the progress there is.
    pub scheduled: bool,
    /// Set by the user: stop (and don't start) the scan that finds a full copy's totals.
    pub skip_scan: &'a AtomicBool,
    /// `Settings.exact_progress`: always run the list-only pass before a full copy.
    pub exact_progress: bool,
}

/// What a background scan compares the source with, to find how much a full copy will do.
enum Late {
    /// The target folder is new and empty: everything in the source gets copied.
    All,
    /// The target is the previous full, updated in place: what differs from that backup's index.
    Index(HashMap<String, (u64, u64)>),
    /// The target already holds files (a resumed copy, or a previous full without an index): what
    /// differs from what is in it. It is read while robocopy writes there, so the files robocopy
    /// has copied meanwhile (`Copied`) count as work too, whichever state the scan saw them in.
    Folder(PathBuf),
}

/// Files (relative to the source, lowercase) robocopy has started copying in this run, recorded
/// while a `Late::Folder` scan is still running.
#[derive(Default)]
struct Copied {
    files: Mutex<HashSet<String>>,
    /// The scan is done: nothing more to record.
    done: AtomicBool,
}

impl Copied {
    fn record(&self, source: &Path, event: &EngineEvent) {
        if let EngineEvent::File { path, .. } = event {
            if !self.done.load(Ordering::SeqCst) {
                if let Some(rel) = rel_under(source, path) {
                    self.files.lock().unwrap().insert(rel);
                }
            }
        }
    }
}

/// `path` relative to `root`, lowercase (the same key `chain_index` uses), if it is under it.
fn rel_under(root: &Path, path: &str) -> Option<String> {
    let norm = |s: &str| s.replace('/', "\\").to_lowercase();
    let root = norm(&root.to_string_lossy());
    let path = norm(path);
    let rest = path.strip_prefix(root.trim_end_matches('\\'))?;
    Some(rest.strip_prefix('\\')?.to_string())
}

/// How a full copy learns its progress totals. A list-only pass before the copy walks the whole
/// tree an extra time, so it only runs when asked for (`exact_progress`). Otherwise the copy starts
/// at once and a scan of the source runs beside it (`Late`), however long that takes, and reports
/// the exact totals when done: into a new folder, everything in the source; into a folder that
/// already holds files, what differs from them (from the previous full's index when there is one,
/// else from reading the folder). A new folder is estimated from the previous full's index until
/// the scan is done.
fn full_totals(
    ctx: &RunContext,
    previous: Option<Vec<ListedFile>>,
    fresh_folder: bool,
    work: &Path,
) -> (TotalsSource, Option<Late>) {
    if ctx.exact_progress {
        return (TotalsSource::Scan, None);
    }
    if fresh_folder {
        let source = match previous {
            Some(files) => TotalsSource::Estimate(files.len() as u64, files.iter().map(|f| f.size).sum()),
            None => TotalsSource::Unknown,
        };
        return (source, Some(Late::All));
    }
    let late = match previous {
        Some(files) => {
            let mut index = HashMap::new();
            add_to_index(&mut index, files);
            Late::Index(index)
        }
        None => Late::Folder(work.to_path_buf()),
    };
    (TotalsSource::Unknown, Some(late))
}

/// (files, bytes) a full copy will do, from a scan of the source. None = stopped or unreadable
/// (the copy itself reports a source problem).
fn scan_totals(source: &Path, filters: &Filters, late: Late, copied: &Copied, stop: &AtomicBool) -> Option<(u64, u64)> {
    let list_source = || native::list_files(source, filters, stop).ok().flatten();
    let files: Vec<ListedFile> = match late {
        Late::All => list_source()?.files,
        Late::Index(index) => changed_since(list_source()?.files, &index),
        Late::Folder(folder) => {
            // Usually two different drives: read both at once.
            let (listing, present) = std::thread::scope(|scope| {
                let present = scope.spawn(|| manifest::scan_until(&folder, stop));
                let listing = list_source();
                (listing, present.join().ok().flatten())
            });
            let mut index = HashMap::new();
            add_to_index(&mut index, present?.0);
            let listing = listing?;
            let copied = copied.files.lock().unwrap();
            listing
                .files
                .into_iter()
                .filter(|f| differs(f, &index) || copied.contains(&f.rel.to_lowercase()))
                .collect()
        }
    };
    Some((files.len() as u64, files.iter().map(|f| f.size).sum()))
}

/// Bytes per second of this task's last real copy, to start the time estimate before the current
/// run has a speed of its own. Runs that copied little say nothing about the speed.
pub fn typical_rate(history: &[RunRecord], task_id: &str) -> Option<u64> {
    const MIN_BYTES: u64 = 64 << 20;
    history
        .iter()
        .filter(|r| r.task_id == task_id && matches!(r.status, RunStatus::Success | RunStatus::Warning))
        .find_map(|r| {
            let secs = (r.finished_at - r.started_at).num_seconds();
            (r.bytes_copied >= MIN_BYTES && secs >= 10).then(|| r.bytes_copied / secs as u64)
        })
}

/// Bytes each source of a task is expected to copy in a run of this mode: what its last good run
/// of the same mode copied (None = no such run, nothing to go by). Lets the time estimate cover
/// the sources still to come, not only the current one.
pub fn source_estimates(history: &[RunRecord], task: &Task, mode: BackupMode) -> Vec<Option<u64>> {
    let key = |p: &str| p.trim_end_matches('\\').to_lowercase();
    task.sources
        .iter()
        .map(|src| {
            history
                .iter()
                .filter(|r| r.task_id == task.id)
                .flat_map(|r| r.sources.iter())
                .find(|s| {
                    key(&s.source) == key(&src.path)
                        && s.mode == Some(mode)
                        && matches!(s.status, Some(RunStatus::Success | RunStatus::Warning))
                })
                .map(|s| s.bytes_copied)
        })
        .collect()
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

    modernize_names(&dest, &prefix);
    let backups = list_backups(&task.destination, &prefix);
    let (mode, note) = effective_mode(&backups, requested);
    run.mode = Some(mode);
    let mut notes: Vec<String> = note.into_iter().collect();
    let final_path = new_folder_path(&dest, &prefix, ctx.now, mode);
    // A folder left by a stopped or failed run of the same kind: carry on in it instead of starting
    // over (not for a scheduled run, which starts the new cycle). It keeps its name until the run
    // completes, and only then gets the new date.
    let new_cycle = ctx.fresh || (ctx.scheduled && backups.iter().any(|b| !b.partial && b.kind == BackupMode::Full));
    let resume = if new_cycle { None } else { fullest_partial(&backups, mode) };
    let work = match resume {
        Some(b) => PathBuf::from(&b.path),
        None => with_partial(&final_path),
    };
    if let Some(b) = resume {
        notes.push(format!("הגיבוי המשיך מגיבוי שנעצר באמצע ({})", b.name));
    }

    // Whether a full stop may delete `work`: always for an incremental (it holds new files only).
    // For a full, set below.
    let mut discard_safe = mode == BackupMode::Incremental;
    let result = match mode {
        BackupMode::Full => {
            // Fast full: reuse the newest full folder so only differences are copied.
            // Not with an include rule: robocopy only purges names matching it, so files that
            // no longer match would stay in the "full" folder.
            let latest_full = backups.iter().find(|b| b.kind == BackupMode::Full && !b.partial);
            // Read before the reuse below removes the index, and before delete-before removes the folder.
            let previous_files = if resume.is_none() && !ctx.exact_progress {
                latest_full.and_then(|b| manifest::read(Path::new(&b.path)))
            } else {
                None
            };
            let reused = resume.is_none()
                && reuses_previous_full(task)
                && ctx.filters.include_files.is_empty()
                && ctx.filters.include_regex.is_empty()
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
            // Not safe when `work` is the previous full, updated in place, or may be (a resumed
            // folder with no complete full next to it).
            discard_safe = !reused && (resume.is_none() || latest_full.is_some());
            let (totals, late) = full_totals(ctx, previous_files, !reused && resume.is_none(), &work);
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
            // A folder that already holds files (the previous full, or a resumed copy) may hold
            // items the rules skip now; robocopy's mirror leaves those alone, so remove them here.
            if (reused || resume.is_some()) && *ctx.filters != Filters::default() {
                on_event(EngineEvent::Phase("pruning"));
                match native::prune_filtered(&source, &work, ctx.filters, ctx.cancel) {
                    Ok(Some(p)) => {
                        if p.files > 0 {
                            notes.push(format!("נמחקו {} קבצים שלא נכללים עוד בסינון (לפי כללי הסינון הנוכחיים)", p.files));
                            run.freed_bytes += p.bytes;
                        }
                        if p.failed > 0 {
                            notes.push(format!("{} פריטים שסוננו החוצה לא נמחקו מהגיבוי (בשימוש או ללא הרשאה)", p.failed));
                        }
                    }
                    Ok(None) => {
                        run.status = Some(RunStatus::Cancelled);
                        run.message = stopped_message(&work, discard_stopped(ctx, &work, discard_safe));
                        return run;
                    }
                    Err(e) => notes.push(format!("לא ניתן לנקות מהגיבוי פריטים שסוננו החוצה: {e}")),
                }
            }
            on_event(EngineEvent::Phase("scanning"));
            let filters = match native::resolve_regex(&source, ctx.filters, ctx.cancel) {
                Ok(Some(f)) => f,
                Ok(None) => {
                    run.status = Some(RunStatus::Cancelled);
                    run.message = stopped_message(&work, discard_stopped(ctx, &work, discard_safe));
                    return run;
                }
                Err(e) => return fail(run, e),
            };
            let job = CopyJob {
                source: source.clone(),
                target: work.clone(),
                copy_empty_dirs: task.copy_empty_dirs,
                filters,
                log_file: log_file.clone(),
                totals,
                late_totals: None,
            };
            // The scan that finds the totals runs beside the copy and is dropped when the copy ends
            // (a cancel ends the copy, which stops the scan too).
            let stop = AtomicBool::new(false);
            let finished = AtomicBool::new(false);
            let copied = Copied::default();
            let late = late.filter(|_| !ctx.skip_scan.load(Ordering::SeqCst));
            let record = matches!(late, Some(Late::Folder(_)));
            std::thread::scope(|scope| {
                let mut job = job;
                if let Some(late) = late {
                    let (tx, rx) = mpsc::channel();
                    job.late_totals = Some(rx);
                    let (source_ref, filters, copied_ref, stop_ref) = (&source, job.filters.clone(), &copied, &stop);
                    scope.spawn(move || {
                        let _ = tx.send(scan_totals(source_ref, &filters, late, copied_ref, stop_ref));
                        copied_ref.done.store(true, Ordering::SeqCst);
                    });
                    // The user's "skip" ends the scan, not the copy.
                    let (skip, finished_ref, stop_ref) = (ctx.skip_scan, &finished, &stop);
                    scope.spawn(move || {
                        while !finished_ref.load(Ordering::SeqCst) {
                            if skip.load(Ordering::SeqCst) {
                                stop_ref.store(true, Ordering::SeqCst);
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                    });
                }
                // robocopy applies the attributes when copying starts: clear them right then, so the
                // folder is visible during the run, and again below once it has finished.
                let mut unhidden = false;
                let result = ctx.engine.mirror(&job, ctx.cancel, &mut |e| {
                    if !unhidden && matches!(e, EngineEvent::File { .. }) {
                        unhide(&work);
                        unhidden = true;
                    }
                    if record {
                        copied.record(&source, &e);
                    }
                    on_event(e)
                });
                finished.store(true, Ordering::SeqCst);
                stop.store(true, Ordering::SeqCst);
                result
            })
        }
        BackupMode::Incremental => {
            on_event(EngineEvent::Phase("scanning"));
            let listing = match native::list_files(&source, ctx.filters, ctx.cancel) {
                Ok(Some(l)) => l,
                Ok(None) => {
                    run.status = Some(RunStatus::Cancelled);
                    run.message = stopped_message(&work, discard_stopped(ctx, &work, true));
                    return run;
                }
                Err(e) => {
                    return fail(
                        run,
                        format!("לא ניתן לסרוק את תיקיית המקור: {e}. ודאו שהכונן מחובר ושיש הרשאה לקרוא את התיקייה, והריצו את הגיבוי שוב."),
                    )
                }
            };
            let mut changed = changed_since(listing.files, &chain_index(&backups));
            if resume.is_some() {
                let (left, done) = skip_done(&work, changed);
                changed = left;
                if done > 0 {
                    notes.push(format!("{done} קבצים כבר הועתקו קודם ולא הועתקו שוב"));
                }
            }
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

    unhide(&work);
    let mut status = status_of(result.outcome);
    let mut message = if result.outcome == Outcome::Cancelled {
        stopped_message(&work, discard_stopped(ctx, &work, discard_safe))
    } else {
        result.message
    };
    apply_stats(&mut run, result.stats);
    run.debug.insert(
        0,
        format!(
            "Backuper {}, {mode:?}; source {} ({}); destination {} ({})",
            env!("CARGO_PKG_VERSION"),
            source.display(),
            native::volume_info(&source),
            work.display(),
            native::volume_info(&work),
        ),
    );
    if work.exists() {
        run.target_folder = Some(work.to_string_lossy().to_string());
    }
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
    if run.backup_bytes > 0 && (mode == BackupMode::Full || run.files_copied > 0 || resume.is_some()) {
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
        let (_, kind, partial) = parse_folder("Docs", "Docs 2026-10-01 03h00 אינקרמנטלי").unwrap();
        assert_eq!((kind, partial), (BackupMode::Incremental, false));
        let (_, kind, partial) = parse_folder("Docs", "Docs 2026-10-01 03h00m15s מלא.partial").unwrap();
        assert_eq!((kind, partial), (BackupMode::Full, true));
        let (_, kind, _) = parse_folder("Docs", "Docs 2026-10-01 03-00").unwrap();
        assert_eq!(kind, BackupMode::Full, "legacy name without a tag");
        assert!(parse_folder("Docs", "Docs old").is_none());
        assert!(parse_folder("Docs", "Docs2 2026-10-01 03h00 מלא").is_none());
        // Older versions' time format is still read.
        let (_, kind, _) = parse_folder("Docs", "Docs 2026-10-01 03-00 אינקרמנטלי").unwrap();
        assert_eq!(kind, BackupMode::Incremental);
        assert!(parse_folder("Docs", "Docs 2026-10-01 03-00-15 מלא.partial").is_some());
        // Untagged names are only read in the old format.
        assert!(parse_folder("Docs", "Docs 2026-10-01 03h00").is_none());
        assert!(parse_folder("Docs", "Docs 2026-10-01 03;00;15").is_none());
        assert_eq!(
            dated_folder_name(
                "Docs",
                parse_dated("Docs", "Docs 2026-10-01 03-00 מלא").unwrap().at,
                BackupMode::Full,
                false
            ),
            "Docs 2026-10-01 03h00 מלא"
        );
    }

    #[test]
    fn renames_old_folder_names() {
        let env = Env::new();
        let dest = env.root.clone();
        for name in [
            "Docs 2026-10-01 03-00 מלא",
            "Docs 2026-10-02 03-00-15 אינקרמנטלי",
            "Docs 2026-10-03 03-00 אינקרמנטלי.partial",
            "Docs 2026-09-01 03-00",
            "Docs 2026-10-04 03h00 מלא",
            "Docs 2026-09-02 03;00;00",
            "Other 2026-10-01 03-00 מלא",
        ] {
            fs::create_dir_all(dest.join(name)).unwrap();
        }
        // Name already taken in the new format: left as it is.
        fs::create_dir_all(dest.join("Docs 2026-10-05 03-00 מלא")).unwrap();
        fs::create_dir_all(dest.join("Docs 2026-10-05 03h00 מלא")).unwrap();
        let full = dest.join("Docs 2026-10-01 03-00 מלא");
        fs::write(full.join("a.txt"), "x").unwrap();
        let files = vec![ListedFile {
            rel: "a.txt".into(),
            size: 1,
            modified: 1,
        }];
        manifest::write(&full, &files);

        modernize_names(&dest, "Docs");
        let mut names: Vec<String> = fs::read_dir(&dest)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "Docs 2026-09-01 03h00 מלא",
                "Docs 2026-09-02 03;00;00",
                "Docs 2026-10-01 03h00 מלא",
                "Docs 2026-10-02 03h00m15s אינקרמנטלי",
                "Docs 2026-10-03 03h00 אינקרמנטלי.partial",
                "Docs 2026-10-04 03h00 מלא",
                "Docs 2026-10-05 03-00 מלא",
                "Docs 2026-10-05 03h00 מלא",
                "Other 2026-10-01 03-00 מלא",
            ]
        );
        let renamed = dest.join("Docs 2026-10-01 03h00 מלא");
        assert_eq!(
            manifest::read(&renamed).map(|f| f.len()),
            Some(1),
            "the index follows the folder"
        );
        assert_eq!(current_name(&dest, "Docs 2026-10-01 03-00 מלא"), "Docs 2026-10-01 03h00 מלא");
        assert_eq!(current_name(&dest, "Docs 2026-10-05 03-00 מלא"), "Docs 2026-10-05 03-00 מלא");
        assert_eq!(list_backups(dest.to_str().unwrap(), "Docs").len(), 7);
    }

    struct Env {
        root: PathBuf,
        cancel: AtomicBool,
        discard: AtomicBool,
        fresh: AtomicBool,
        scheduled: AtomicBool,
        skip_scan: AtomicBool,
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
                discard: AtomicBool::new(false),
                fresh: AtomicBool::new(false),
                scheduled: AtomicBool::new(false),
                skip_scan: AtomicBool::new(false),
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
                discard: &self.discard,
                fresh: self.fresh.load(Ordering::SeqCst),
                scheduled: self.scheduled.load(Ordering::SeqCst),
                skip_scan: &self.skip_scan,
                exact_progress: false,
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
        /// A run that is stopped as soon as it starts scanning (before any file is copied).
        fn run_stopped(&self, t: &Task, mode: BackupMode, at: &str) -> Vec<SourceRun> {
            self.cancel.store(false, Ordering::SeqCst);
            let r = self.run_with(t, mode, at, &mut |_, e| {
                if matches!(e, EngineEvent::Phase("scanning")) {
                    self.cancel.store(true, Ordering::SeqCst);
                }
            });
            self.cancel.store(false, Ordering::SeqCst);
            r
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
        assert_eq!(env.names(&t, 0), ["מסמכים 2026-10-01 03h00 מלא"]);
        assert_eq!(env.names(&t, 1), ["Pics 2026-10-01 03h00 מלא"]);
        assert_eq!(r[0].files_copied, 2);
        assert_eq!((r[0].backup_bytes, r[0].freed_bytes), (1 + 8, 0), "a.txt + קובץ.txt");

        // Incremental: a new folder with only the new/changed file.
        fs::write(docs.join("b.txt"), "b").unwrap();
        fs::write(docs.join("a.txt"), "a changed").unwrap();
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!(r[0].mode, Some(BackupMode::Incremental));
        assert_eq!(r[0].files_copied, 2, "{r:?}");
        let inc = dst.join("מסמכים 2026-10-02 03h00 אינקרמנטלי");
        assert!(inc.join("a.txt").exists() && inc.join("b.txt").exists());
        assert!(!inc.join("תיקייה").exists(), "unchanged files are not copied again");
        // Pics had no changes: its incremental folder exists, but is empty.
        let empty = dst.join("Pics 2026-10-02 03h00 אינקרמנטלי");
        assert!(empty.is_dir() && !has_files(&empty));
        // The backups list gives each folder's own size (full 9, incremental 1 + 9, empty 0).
        let listed: Vec<(BackupMode, Option<u64>)> =
            list_task_backups(&t)[0].backups.iter().map(|b| (b.kind, b.bytes)).collect();
        assert_eq!(listed, [(BackupMode::Incremental, Some(1 + 9)), (BackupMode::Full, Some(9))]);
        assert_eq!(list_task_backups(&t)[1].backups[0].bytes, Some(0));

        // A second incremental only sees changes since the previous incremental.
        let r = env.run(&t, BackupMode::Incremental, "2026-10-03 03:00");
        assert_eq!(r[0].files_copied, 0, "{r:?}");

        // Full: new full folder, the whole previous chain (incl. empty incrementals) is deleted.
        fs::remove_file(docs.join("b.txt")).unwrap();
        let r = env.run(&t, BackupMode::Full, "2026-10-04 03:00");
        assert!(r.iter().all(|s| s.status == Some(RunStatus::Success)), "{r:?}");
        assert_eq!(env.names(&t, 0), ["מסמכים 2026-10-04 03h00 מלא"]);
        assert_eq!(env.names(&t, 1), ["Pics 2026-10-04 03h00 מלא"]);
        assert!(!dst.join("מסמכים 2026-10-04 03h00 מלא").join("b.txt").exists());
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
        assert!(dst.join("Src 2026-10-01 03h00 מלא").join("sys.dat").exists());

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
        assert_eq!(env.names(&t, 0), ["Src 2026-10-03 03h00 מלא"]);
        let full = dst.join("Src 2026-10-03 03h00 מלא");
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
                "Src 2026-10-01 03h00 מלא",
                "Src 2026-10-03 03h00 אינקרמנטלי",
                "Src 2026-10-04 03h00 מלא"
            ]
        );
        env.run(&t, BackupMode::Full, "2026-10-05 03:00");
        assert_eq!(env.names(&t, 0), ["Src 2026-10-04 03h00 מלא", "Src 2026-10-05 03h00 מלא"]);
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
                "Src 2026-10-01 03h00 מלא",
                "Src 2026-10-02 03h00 אינקרמנטלי",
                "Src 2026-10-03 03h00 מלא",
                "Src 2026-10-04 03h00 מלא"
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
                "Src 2026-10-01 03h00 מלא",
                "Src 2026-10-02 03h00 אינקרמנטלי",
                "Src 2026-10-05 03h00 מלא",
                "Src 2026-10-10 03h00 מלא"
            ]
        );
        // Cutoff 10-06: the 10-05 full covers it, so the 10-01 chain goes - even on an incremental.
        fs::write(src.join("c.txt"), "c").unwrap();
        env.run(&t, BackupMode::Incremental, "2026-10-13 03:00");
        assert_eq!(
            env.names(&t, 0),
            [
                "Src 2026-10-05 03h00 מלא",
                "Src 2026-10-10 03h00 מלא",
                "Src 2026-10-13 03h00 אינקרמנטלי"
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
                "Src 2026-10-01 03h00 מלא",
                "Src 2026-10-02 03h00 אינקרמנטלי",
                "Src 2026-10-03 03h00 מלא"
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
        let old = dst.join("Src 2026-10-01 03h00 מלא");
        assert!(old.is_dir());
        // When copying starts, the previous backup must already be gone.
        let mut old_existed_at_copy = None;
        let r = env.run_with(&t, BackupMode::Full, "2026-10-02 03:00", &mut |_, e| {
            if matches!(e, EngineEvent::TotalsPending) {
                old_existed_at_copy = Some(old.exists());
            }
        });
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        assert_eq!(old_existed_at_copy, Some(false));
        assert_eq!(env.names(&t, 0), ["Src 2026-10-02 03h00 מלא"]);

        // Without delete-before, the old backup survives until the new one is complete.
        let t = Task {
            delete_before: false,
            ..t
        };
        let prev = dst.join("Src 2026-10-02 03h00 מלא");
        let mut prev_existed_at_copy = None;
        env.run_with(&t, BackupMode::Full, "2026-10-03 03:00", &mut |_, e| {
            if matches!(e, EngineEvent::TotalsPending) {
                prev_existed_at_copy = Some(prev.exists());
            }
        });
        assert_eq!(prev_existed_at_copy, Some(true));
        assert_eq!(env.names(&t, 0), ["Src 2026-10-03 03h00 מלא"]);
    }

    #[test]
    fn full_space_counts_what_delete_before_frees() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), vec![b'a'; 1000]).unwrap();
        let base = Task {
            mode: BackupMode::Full,
            keep_count: 2,
            ..task(&[src.to_str().unwrap()], dst.to_str().unwrap())
        };
        let now = Local::now();
        // Nothing to compare with before the first full backup.
        assert_eq!(full_space(&base, now, false), None);
        env.run(&base, BackupMode::Full, "2026-10-01 03:00");
        env.run(&base, BackupMode::Full, "2026-10-02 03:00");
        assert_eq!(full_space(&base, now, false), Some(FullSpace { needed: 1000, freed: 0 }));

        // Delete-before with keep 2: the new full counts as one, so the older of the two goes first.
        let before = Task { delete_before: true, ..base.clone() };
        assert_eq!(full_space(&before, now, false), Some(FullSpace { needed: 1000, freed: 1000 }));
        // Updating the previous full in place needs no new room.
        let reuse = Task { reuse_previous: true, keep_count: 1, ..base.clone() };
        assert_eq!(full_space(&reuse, now, false), None);
        // An incremental-only task makes no full backup while one exists.
        let inc = Task { mode: BackupMode::Incremental, ..base.clone() };
        assert_eq!(full_space(&inc, now, false), None);
        // ...unless the user asked for the next run to be a full one.
        assert_eq!(full_space(&inc, now, true), Some(FullSpace { needed: 1000, freed: 0 }));
        // ...unless it also has a full schedule (combined mode).
        let combined = Task { full_schedule: Some(crate::model::Schedule::Daily { time: "03:00".into() }), ..inc };
        assert_eq!(full_space(&combined, now, false), Some(FullSpace { needed: 1000, freed: 0 }));
    }

    /// Without the exact-progress setting no list-only pass runs before the copy: the totals come
    /// from a scan of the source beside it (an estimate from the previous index until then).
    #[test]
    fn full_totals_come_from_a_scan_beside_the_copy() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        // (pending seen, every Totals event as (files, bytes, exact))
        let totals = |env: &Env, t: &Task, at: &str| {
            let (mut pending, mut seen) = (false, Vec::new());
            env.run_with(t, BackupMode::Full, at, &mut |_, e| match e {
                EngineEvent::TotalsPending => pending = true,
                EngineEvent::Totals { files, bytes, exact } => seen.push((files, bytes, exact)),
                _ => {}
            });
            (pending, seen)
        };
        // First backup: nothing to estimate from; the scan of the source gives the exact totals.
        assert_eq!(totals(&env, &t, "2026-10-01 03:00"), (true, vec![(1, 1, true)]));
        // Second one: the previous index says 1 file (estimate), then the scan finds the new one.
        fs::write(src.join("c.txt"), "cc").unwrap();
        assert_eq!(totals(&env, &t, "2026-10-02 03:00"), (true, vec![(1, 1, false), (2, 3, true)]));
        // A reused folder only copies the difference: the scan compares the source with the index.
        let reuse = Task { reuse_previous: true, ..t };
        fs::write(src.join("d.txt"), "ddd").unwrap();
        fs::write(src.join("a.txt"), "changed").unwrap();
        assert_eq!(totals(&env, &reuse, "2026-10-03 03:00"), (true, vec![(2, 10, true)]));
        // With no index to compare with (a folder made by an older version), the folder itself is read.
        fs::write(src.join("e.txt"), "e").unwrap();
        manifest::remove(&dst.join("Src 2026-10-03 03h00 מלא"));
        assert_eq!(totals(&env, &reuse, "2026-10-04 03:00"), (true, vec![(1, 1, true)]));
    }

    /// The user can give up on the scan: the copy then carries on without totals.
    #[test]
    fn skipped_scan_leaves_the_copy_without_totals() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        env.skip_scan.store(true, Ordering::SeqCst);
        let mut events = Vec::new();
        let r = env.run_with(&t, BackupMode::Full, "2026-10-01 03:00", &mut |_, e| {
            if matches!(e, EngineEvent::Totals { .. } | EngineEvent::TotalsPending) {
                events.push(());
            }
        });
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        assert_eq!(r[0].files_copied, 1);
        assert!(events.is_empty(), "no scan was started");
    }

    /// A resumed full compares the source with what its partial folder already holds.
    #[test]
    fn resumed_full_totals_count_what_is_left() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(src.join("sub")).unwrap();
        for (name, body) in [("a.txt", "a"), ("sub\\b.txt", "bb")] {
            fs::write(src.join(name), body).unwrap();
        }
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        // Make that backup look like a copy that was paused: still partial, no index.
        let done = dst.join("Src 2026-10-01 03h00 מלא");
        let partial = dst.join("Src 2026-10-01 03h00 מלא.partial");
        fs::rename(&done, &partial).unwrap();
        manifest::remove(&partial);
        fs::write(src.join("c.txt"), "ccc").unwrap();
        fs::write(src.join("sub").join("d.txt"), "dddd").unwrap();
        let mut seen = Vec::new();
        let r = env.run_with(&t, BackupMode::Full, "2026-10-02 03:00", &mut |_, e| {
            if let EngineEvent::Totals { files, bytes, exact } = e {
                seen.push((files, bytes, exact));
            }
        });
        assert_eq!(r[0].files_copied, 2, "{r:?}");
        assert_eq!(seen, vec![(2, 7, true)], "only the two new files are left to copy");
    }

    /// The folder is read while robocopy writes into it: a file it has already copied looks "done" to
    /// the scan, but was still work of this run.
    #[test]
    fn folder_scan_counts_files_copied_meanwhile() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        for (name, body) in [("a.txt", "aa"), ("b.txt", "bbb"), ("c.txt", "c")] {
            fs::write(src.join(name), body).unwrap();
        }
        // The folder already holds a copy of a and b (same size and time); c is missing.
        for name in ["a.txt", "b.txt"] {
            fs::copy(src.join(name), dst.join(name)).unwrap();
        }
        let scan = |copied: &Copied| {
            scan_totals(&src, &Filters::default(), Late::Folder(dst.clone()), copied, &AtomicBool::new(false))
        };
        let copied = Copied::default();
        assert_eq!(scan(&copied), Some((1, 1)), "only c is left");
        // robocopy copied a during the scan: the file event is recorded, so a counts as work too.
        copied.record(&src, &EngineEvent::File { path: src.join("a.txt").to_string_lossy().to_string(), size: 2 });
        assert_eq!(scan(&copied), Some((2, 3)));
        copied.done.store(true, Ordering::SeqCst);
        copied.record(&src, &EngineEvent::File { path: src.join("b.txt").to_string_lossy().to_string(), size: 3 });
        assert_eq!(scan(&copied), Some((2, 3)), "nothing is recorded once the scan is done");
    }

    #[test]
    fn paths_are_made_relative_to_the_source() {
        let root = Path::new("D:\\Mine\\Docs");
        assert_eq!(rel_under(root, "d:\\mine\\docs\\Sub\\A.txt").as_deref(), Some("sub\\a.txt"));
        assert_eq!(rel_under(Path::new("D:\\"), "D:\\x.txt").as_deref(), Some("x.txt"));
        assert_eq!(rel_under(root, "D:\\Mine\\Docs2\\a.txt"), None, "a sibling folder with the same start");
        assert_eq!(rel_under(root, "E:\\Mine\\Docs\\a.txt"), None);
    }

    #[test]
    fn typical_rate_comes_from_the_last_real_copy() {
        let run = |task: &str, status, bytes: u64, secs: i64| RunRecord {
            id: String::new(),
            task_id: task.into(),
            task_name: String::new(),
            trigger: crate::model::Trigger::Manual,
            mode: BackupMode::Full,
            started_at: Local::now() - chrono::Duration::seconds(secs),
            finished_at: Local::now(),
            status,
            message: String::new(),
            files_copied: 0,
            bytes_copied: bytes,
            backup_bytes: 0,
            freed_bytes: 0,
            files_deleted: 0,
            files_failed: 0,
            sources: vec![],
        };
        let mb = 1 << 20;
        // Newest first: a tiny run, a failed run, another task's run, then the one that counts.
        let history = [
            run("1", RunStatus::Success, mb, 60),
            run("1", RunStatus::Failed, 900 * mb, 60),
            run("2", RunStatus::Success, 900 * mb, 60),
            run("1", RunStatus::Success, 600 * mb, 60),
        ];
        assert_eq!(typical_rate(&history, "1"), Some(10 * mb as u64));
        assert_eq!(typical_rate(&history, "3"), None);
    }

    #[test]
    fn source_estimates_come_from_the_last_good_run_of_the_mode() {
        let src = |path: &str, mode, status, bytes| SourceRun {
            source: path.into(),
            mode: Some(mode),
            status: Some(status),
            bytes_copied: bytes,
            ..Default::default()
        };
        let run = |sources| RunRecord {
            id: String::new(),
            task_id: "1".into(),
            task_name: String::new(),
            trigger: crate::model::Trigger::Manual,
            mode: BackupMode::Full,
            started_at: Local::now(),
            finished_at: Local::now(),
            status: RunStatus::Success,
            message: String::new(),
            files_copied: 0,
            bytes_copied: 0,
            backup_bytes: 0,
            freed_bytes: 0,
            files_deleted: 0,
            files_failed: 0,
            sources,
        };
        // Newest first.
        let history = [
            run(vec![src("C:\\A", BackupMode::Incremental, RunStatus::Success, 5)]),
            run(vec![src("C:\\A", BackupMode::Full, RunStatus::Failed, 7)]),
            run(vec![
                src("c:\\a\\", BackupMode::Full, RunStatus::Success, 100),
                src("C:\\B", BackupMode::Full, RunStatus::Warning, 200),
            ]),
        ];
        let t = Task {
            id: "1".into(),
            ..task(&["C:\\A", "C:\\B", "C:\\C"], "E:\\Backup")
        };
        assert_eq!(source_estimates(&history, &t, BackupMode::Full), [Some(100), Some(200), None]);
        assert_eq!(source_estimates(&history, &t, BackupMode::Incremental), [Some(5), None, None]);
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
        assert_eq!(env.names(&t, 0), ["Src 2026-10-02 03h00 מלא"]);
        let index = manifest::read(&dst.join("Src 2026-10-02 03h00 מלא")).unwrap();
        assert_eq!(index.len(), 2, "the reused folder gets a fresh index");
    }

    /// A fast full makes the reused folder match the source and the current rules: what was deleted
    /// from the source or is now filtered out goes too.
    #[test]
    fn fast_full_removes_deleted_and_newly_filtered_items() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        for d in ["gone", "cache", "keep"] {
            fs::create_dir_all(src.join(d)).unwrap();
            fs::write(src.join(d).join("f.txt"), "x").unwrap();
        }
        fs::write(src.join("junk.tmp"), "t").unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let t = Task {
            reuse_previous: true,
            ..task(&[src.to_str().unwrap()], dst.to_str().unwrap())
        };
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        fs::remove_dir_all(src.join("gone")).unwrap();
        let with_rules = Env {
            root: env.root.clone(),
            cancel: AtomicBool::new(false),
            discard: AtomicBool::new(false),
            fresh: AtomicBool::new(false),
            scheduled: AtomicBool::new(false),
            skip_scan: AtomicBool::new(false),
            filters: crate::filters::compile(&[
                crate::model::FilterRule::Folder { value: "cache".into() },
                crate::model::FilterRule::Extension { value: "tmp".into() },
            ]),
        };
        let r = with_rules.run(&t, BackupMode::Full, "2026-10-02 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        let b = dst.join("Src 2026-10-02 03h00 מלא");
        assert!(b.join("keep").join("f.txt").exists() && b.join("a.txt").exists());
        assert!(!b.join("gone").exists(), "deleted from the source");
        assert!(!b.join("cache").exists(), "folder now filtered out");
        assert!(!b.join("junk.tmp").exists(), "file now filtered out");
    }

    /// Continuing a partial full applies the rules as they are now: what they skip is removed from it.
    #[test]
    fn resumed_full_removes_newly_filtered_items() {
        let env = Env::new();
        let (src, dst) = partial_setup(&env, "Src 2026-10-01 03h00 מלא.partial", &["a.txt", "b.txt"]);
        fs::create_dir_all(src.join("cache")).unwrap();
        fs::write(src.join("cache").join("f.txt"), "x").unwrap();
        let partial = dst.join("Src 2026-10-01 03h00 מלא.partial");
        fs::create_dir_all(partial.join("cache")).unwrap();
        fs::write(partial.join("cache").join("f.txt"), "x").unwrap();
        let with_rule = Env {
            root: env.root.clone(),
            cancel: AtomicBool::new(false),
            discard: AtomicBool::new(false),
            fresh: AtomicBool::new(false),
            scheduled: AtomicBool::new(false),
            skip_scan: AtomicBool::new(false),
            filters: crate::filters::compile(&[
                crate::model::FilterRule::Folder { value: "cache".into() },
                crate::model::FilterRule::Pattern { value: "b.txt".into() },
            ]),
        };
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        let r = with_rule.run(&t, BackupMode::Full, "2026-10-02 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        let b = dst.join("Src 2026-10-02 03h00 מלא");
        assert!(b.join("a.txt").exists() && b.join("c.txt").exists());
        assert!(!b.join("cache").exists() && !b.join("b.txt").exists(), "{r:?}");
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
        let full = dst.join("Src 2026-10-01 03h00 מלא");
        let mut index: Vec<String> = manifest::read(&full).unwrap().into_iter().map(|f| f.rel).collect();
        index.sort();
        assert_eq!(index, ["a.txt", "sub\\b.txt"]);

        // A folder without an index (older version) is walked, and gets one.
        fs::remove_file(full.join(manifest::FILE_NAME)).unwrap();
        fs::write(src.join("c.txt"), "c").unwrap();
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!(r[0].files_copied, 1, "{r:?}");
        assert!(manifest::read(&full).is_some());
        let inc = dst.join("Src 2026-10-02 03h00 אינקרמנטלי");
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

    /// A full backup where a few files can't be read (locked) completes with a warning instead of staying
    /// a `.partial` folder, the failure is listed, and robocopy's retry lines count the file once.
    #[test]
    fn full_backup_with_a_locked_file_completes_with_a_warning() {
        use std::os::windows::fs::OpenOptionsExt;
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        for i in 0..40 {
            fs::write(src.join(format!("f{i}.txt")), "x").unwrap();
        }
        fs::write(src.join("locked.txt"), "l").unwrap();
        // Held open without sharing: robocopy gets ERROR 32 on every retry.
        let _lock = fs::OpenOptions::new().read(true).write(true).share_mode(0).open(src.join("locked.txt")).unwrap();

        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        let mut locked_starts = 0;
        let r = env.run_with(&t, BackupMode::Full, "2026-10-01 03:00", &mut |_, e| {
            if let EngineEvent::File { path, .. } = e {
                if path.ends_with("locked.txt") {
                    locked_starts += 1;
                }
            }
        });
        assert_eq!(r[0].status, Some(RunStatus::Warning), "{r:?}");
        assert_eq!(r[0].files_failed, 1, "{r:?}");
        assert!(r[0].errors.iter().any(|e| e.path.ends_with("locked.txt") && e.code == Some(32)), "{:?}", r[0].errors);
        assert_eq!(locked_starts, 1, "the retries of one file are one file");
        assert_eq!(env.names(&t, 0), ["Src 2026-10-01 03h00 מלא"], "finished, not .partial");
        assert!(dst.join("Src 2026-10-01 03h00 מלא").join("f39.txt").exists());
    }

    /// A single file excluded by its full path (the "skip this file" rule from a run's errors) is left out
    /// of both a full backup (robocopy /XF) and an incremental (native listing).
    #[test]
    fn full_path_file_rule_skips_one_file() {
        let mut env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("pagefile.sys"), "p").unwrap();
        fs::write(src.join("sub").join("pagefile.sys"), "p2").unwrap();
        fs::write(src.join("keep.txt"), "k").unwrap();
        let rule = crate::model::FilterRule::File { value: src.join("pagefile.sys").display().to_string() };
        env.filters = crate::filters::compile(&[rule]);
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());

        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        let full = dst.join("Src 2026-10-01 03h00 מלא");
        assert!(full.join("keep.txt").exists());
        assert!(!full.join("pagefile.sys").exists());
        assert!(full.join("sub").join("pagefile.sys").exists(), "same name in another folder is not the file");

        fs::write(src.join("new.txt"), "n").unwrap();
        fs::write(src.join("pagefile.sys"), "changed").unwrap();
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!(r[0].files_copied, 1, "{r:?}");
        let inc = dst.join("Src 2026-10-02 03h00 אינקרמנטלי");
        assert!(inc.join("new.txt").exists() && !inc.join("pagefile.sys").exists());
    }

    /// A plain config imported in another split keeps the folder names (and so the chains in the
    /// destination): the new tasks continue with an incremental, not a new full. Two folders with
    /// the same last name get " (2)" in every split, and no split has a (destination, folderName) twice.
    #[test]
    fn plain_config_chains_survive_a_different_split() {
        use crate::simpleconfig::{parse, Split};
        let env = Env::new();
        let (a, b, dst) = (env.root.join("A").join("Docs"), env.root.join("B").join("Docs"), env.root.join("dst"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("a.txt"), "a").unwrap();
        fs::write(b.join("b.txt"), "b").unwrap();
        let json = serde_json::json!({
            "copy_sources": {"src_0": [a.to_str().unwrap()], "src_1": [b.to_str().unwrap()]},
            "copy_destinations": [dst.to_str().unwrap()],
        })
        .to_string();
        let tasks = |split| parse(json.as_bytes(), split).unwrap().unwrap().tasks.into_iter().map(|i| i.task).collect::<Vec<_>>();

        for split in [Split::One, Split::Group, Split::Source] {
            let mut keys: Vec<_> = tasks(split)
                .iter()
                .flat_map(|t| t.sources.iter().map(|s| (t.destination.to_lowercase(), s.folder_name.to_lowercase())))
                .collect();
            keys.sort();
            assert_eq!(keys.len(), 2);
            keys.dedup();
            assert_eq!(keys.len(), 2, "{split:?}: folder names collide");
        }

        // Full backup as one task, then each folder as its own task.
        let one = &tasks(Split::One)[0];
        env.run(one, BackupMode::Full, "2026-10-01 03:00");
        assert_eq!(env.names(one, 0), ["Docs 2026-10-01 03h00 מלא"]);
        assert_eq!(env.names(one, 1), ["Docs (2) 2026-10-01 03h00 מלא"]);
        fs::write(a.join("a2.txt"), "a2").unwrap();
        fs::write(b.join("b2.txt"), "b2").unwrap();
        for t in tasks(Split::Source) {
            let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
            assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
            assert_eq!(r[0].files_copied, 1, "{}: only the new file, no new full: {r:?}", t.name);
            assert!(r[0].message.is_empty() || !r[0].message.contains("מלא"), "{}", r[0].message);
        }
        let split = tasks(Split::Source);
        assert_eq!(env.names(&split[0], 0), ["Docs 2026-10-01 03h00 מלא", "Docs 2026-10-02 03h00 אינקרמנטלי"]);
        assert_eq!(
            env.names(&split[1], 0),
            ["Docs (2) 2026-10-01 03h00 מלא", "Docs (2) 2026-10-02 03h00 אינקרמנטלי"]
        );
    }

    /// Cobian's folders, taken over: the next incremental copies only what changed after Cobian's last backup.
    #[test]
    fn continues_from_cobian_backups() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Docs"), env.root.join("dst"));
        fs::create_dir_all(src.join("sub")).unwrap();
        for f in ["a.txt", "b.txt", "sub\\c.txt"] {
            fs::write(src.join(f), f).unwrap();
        }
        // What Cobian left: a full, then an incremental with b.txt changed (copies keep the file times).
        let full = dst.join("Docs 2026-09-28 03;00;00");
        fs::create_dir_all(full.join("sub")).unwrap();
        for f in ["a.txt", "b.txt", "sub\\c.txt"] {
            fs::copy(src.join(f), full.join(f)).unwrap();
        }
        fs::write(src.join("b.txt"), "b changed").unwrap();
        let inc = dst.join("Docs 2026-09-29 03;00;00");
        fs::create_dir_all(&inc).unwrap();
        fs::copy(src.join("b.txt"), inc.join("b.txt")).unwrap();

        let t = Task {
            id: "t".into(),
            ..task(&[src.to_str().unwrap()], dst.to_str().unwrap())
        };
        let r = crate::cobianmigrate::apply(&t, &[]);
        assert_eq!((r.renamed, r.errors.len()), (2, 0), "{r:?}");
        assert_eq!(env.names(&t, 0), ["Docs 2026-09-28 03h00 מלא", "Docs 2026-09-29 03h00 אינקרמנטלי"]);

        // Changed after Cobian's last backup: a new file only.
        fs::write(src.join("d.txt"), "d").unwrap();
        let r = env.run(&t, BackupMode::Incremental, "2026-10-01 03:00");
        assert_eq!(r[0].mode, Some(BackupMode::Incremental), "{r:?}");
        assert_eq!(r[0].files_copied, 1, "{r:?}");
        assert!(dst.join("Docs 2026-10-01 03h00 אינקרמנטלי").join("d.txt").exists());
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
        let full = dst.join("Src 2026-10-01 03h00 מלא");
        assert_eq!(fs::read_to_string(full.join(manifest::FILE_NAME)).unwrap(), "user data");
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
    }

    /// Stopping an incremental midway keeps its folder; the next run finishes it instead of starting over.
    #[test]
    fn stopped_incremental_resumes_in_its_folder() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("base.txt"), "base").unwrap();
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        for i in 1..=5 {
            fs::write(src.join(format!("new{i}.txt")), format!("n{i}")).unwrap();
        }

        // Stop when the third file starts: it and the two before it get copied.
        let mut started = 0;
        let r = env.run_with(&t, BackupMode::Incremental, "2026-10-02 03:00", &mut |_, e| {
            if let EngineEvent::File { .. } = e {
                started += 1;
                if started == 3 {
                    env.cancel.store(true, Ordering::SeqCst);
                }
            }
        });
        env.cancel.store(false, Ordering::SeqCst);
        assert_eq!(r[0].status, Some(RunStatus::Cancelled), "{r:?}");
        assert!(r[0].message.contains("להמשיך אותו"), "{}", r[0].message);
        let partial = dst.join("Src 2026-10-02 03h00 אינקרמנטלי.partial");
        assert!(partial.is_dir());
        let done = fs::read_dir(&partial).unwrap().count();
        assert_eq!(done, 3);
        // Left over from an older state of the source: must not end up in the finished backup.
        fs::write(partial.join("stale.txt"), "old").unwrap();

        // Next day: the same folder is finished, and only the missing files are copied.
        let r = env.run(&t, BackupMode::Incremental, "2026-10-03 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        assert_eq!(r[0].files_copied, 2, "{r:?}");
        assert!(r[0].message.contains("3 קבצים כבר הועתקו"), "{}", r[0].message);
        assert_eq!(env.names(&t, 0), ["Src 2026-10-01 03h00 מלא", "Src 2026-10-03 03h00 אינקרמנטלי"]);
        let inc = dst.join("Src 2026-10-03 03h00 אינקרמנטלי");
        for i in 1..=5 {
            assert!(inc.join(format!("new{i}.txt")).exists());
        }
        assert!(!inc.join("stale.txt").exists());
        assert_eq!(manifest::read(&inc).unwrap().len(), 5);
    }

    /// A full stop deletes the half-done folder (a pause keeps it), but never the last complete backup.
    #[test]
    fn full_stop_discards_the_partial_folder() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        fs::write(src.join("b.txt"), "b").unwrap();

        // Stopped for good: the new full's folder goes, the old full stays.
        env.discard.store(true, Ordering::SeqCst);
        let r = env.run_stopped(&t, BackupMode::Full, "2026-10-02 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Cancelled), "{r:?}");
        assert!(r[0].message.contains("נמחקה"), "{}", r[0].message);
        assert_eq!(env.names(&t, 0), ["Src 2026-10-01 03h00 מלא"]);
        assert!(!has_partial(&t));

        // Paused: the folder stays, and says the next run continues in it.
        env.discard.store(false, Ordering::SeqCst);
        let r = env.run_stopped(&t, BackupMode::Full, "2026-10-03 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Cancelled), "{r:?}");
        assert!(r[0].message.contains("להמשיך אותו"), "{}", r[0].message);
        assert!(has_partial(&t));

        // The paused folder is resumed, then stopped for good: a complete full exists, so it goes.
        env.discard.store(true, Ordering::SeqCst);
        env.run_stopped(&t, BackupMode::Full, "2026-10-04 03:00");
        assert!(!has_partial(&t));
        assert_eq!(env.names(&t, 0), ["Src 2026-10-01 03h00 מלא"]);
    }

    /// With "reuse the previous full" the half-done folder IS the previous full: a full stop keeps it.
    #[test]
    fn full_stop_keeps_a_reused_previous_full() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let t = Task {
            reuse_previous: true,
            ..task(&[src.to_str().unwrap()], dst.to_str().unwrap())
        };
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        env.discard.store(true, Ordering::SeqCst);
        let r = env.run_stopped(&t, BackupMode::Full, "2026-10-02 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Cancelled), "{r:?}");
        let partial = dst.join("Src 2026-10-02 03h00 מלא.partial");
        assert!(partial.join("a.txt").exists(), "the previous backup must not be deleted");
        assert!(r[0].message.contains("להמשיך אותו"), "{}", r[0].message);
    }

    /// A full stop of an incremental deletes its half-done folder.
    #[test]
    fn full_stop_of_an_incremental() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("base.txt"), "base").unwrap();
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        for i in 1..=4 {
            fs::write(src.join(format!("new{i}.txt")), "n").unwrap();
        }
        let mut started = 0;
        let r = env.run_with(&t, BackupMode::Incremental, "2026-10-02 03:00", &mut |_, e| {
            if let EngineEvent::File { .. } = e {
                started += 1;
                if started == 2 {
                    env.discard.store(true, Ordering::SeqCst);
                    env.cancel.store(true, Ordering::SeqCst);
                }
            }
        });
        assert_eq!(r[0].status, Some(RunStatus::Cancelled), "{r:?}");
        assert!(!has_partial(&t));
        assert_eq!(env.names(&t, 0), ["Src 2026-10-01 03h00 מלא"]);
    }

    /// A drive root is hidden + system, and robocopy copies that onto the backup folder: it must stay visible.
    #[test]
    fn backup_folder_of_a_hidden_system_source_is_visible() {
        use std::os::windows::fs::MetadataExt;
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let ok = std::process::Command::new("attrib").args(["+H", "+S"]).arg(&src).status().unwrap();
        assert!(ok.success());
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        let mut during = None;
        let r = env.run_with(&t, BackupMode::Full, "2026-10-01 03:00", &mut |_, e| {
            if matches!(e, EngineEvent::File { .. }) {
                std::thread::sleep(std::time::Duration::from_millis(300)); // let the attributes land
                during = fs::metadata(dst.join("Src 2026-10-01 03h00 מלא.partial")).ok().map(|m| m.file_attributes());
            }
        });
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        let hidden_or_system = |a: u32| a & 0x6 != 0;
        let full = fs::metadata(dst.join("Src 2026-10-01 03h00 מלא")).unwrap().file_attributes();
        assert!(!hidden_or_system(full), "finished folder: {full:#x}");
        assert!(during.is_some_and(|a| !hidden_or_system(a)), "folder during the copy: {during:?}");
        let _ = std::process::Command::new("attrib").args(["-H", "-S"]).arg(&src).status();
    }

    /// A partial full folder is finished by the next full run: robocopy skips what is already there.
    #[test]
    fn partial_full_is_continued() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        let partial = dst.join("Src 2026-10-01 03h00 מלא.partial");
        fs::create_dir_all(&partial).unwrap();
        for f in ["a.txt", "b.txt", "c.txt"] {
            fs::write(src.join(f), f).unwrap();
        }
        // Two of the three were copied before the stop (a copy keeps the file times).
        for f in ["a.txt", "b.txt"] {
            fs::copy(src.join(f), partial.join(f)).unwrap();
        }
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        let r = env.run(&t, BackupMode::Full, "2026-10-02 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        assert_eq!(r[0].files_copied, 1, "only the missing file is copied: {r:?}");
        assert_eq!(env.names(&t, 0), ["Src 2026-10-02 03h00 מלא"]);
        let full = dst.join("Src 2026-10-02 03h00 מלא");
        assert!(["a.txt", "b.txt", "c.txt"].iter().all(|f| full.join(f).exists()));
        assert_eq!(r[0].backup_bytes, 15);
    }

    /// A scheduled run (or "start over") leaves the paused folder alone and starts a new one.
    #[test]
    fn fresh_run_does_not_continue_a_partial_folder() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        let partial = dst.join("Src 2026-10-01 03h00 מלא.partial");
        fs::create_dir_all(&partial).unwrap();
        for f in ["a.txt", "b.txt", "c.txt"] {
            fs::write(src.join(f), f).unwrap();
        }
        for f in ["a.txt", "b.txt"] {
            fs::copy(src.join(f), partial.join(f)).unwrap();
        }
        env.fresh.store(true, Ordering::SeqCst);
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        let r = env.run(&t, BackupMode::Full, "2026-10-02 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        assert_eq!(r[0].files_copied, 3, "everything is copied again: {r:?}");
        assert!(!r[0].message.contains("המשיך מגיבוי"), "{}", r[0].message);
        assert_eq!(env.names(&t, 0), ["Src 2026-10-02 03h00 מלא"], "the old partial is cleaned up by the new full");
    }

    /// Builds a source with `a.txt`..`c.txt` and a partial full holding the first `copied` of them.
    fn partial_setup(env: &Env, partial_name: &str, copied: &[&str]) -> (PathBuf, PathBuf) {
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        let partial = dst.join(partial_name);
        fs::create_dir_all(&partial).unwrap();
        for f in ["a.txt", "b.txt", "c.txt"] {
            fs::write(src.join(f), f).unwrap();
        }
        for f in copied {
            fs::copy(src.join(f), partial.join(f)).unwrap();
        }
        (src, dst)
    }

    /// With no complete full yet, a scheduled run (or catch-up) continues the partial folder: it is
    /// all the progress there is, and a new folder would copy everything again.
    #[test]
    fn scheduled_run_continues_a_partial_full_when_no_full_is_complete() {
        let env = Env::new();
        let (src, dst) = partial_setup(&env, "Src 2026-10-01 03h00 מלא.partial", &["a.txt", "b.txt"]);
        env.scheduled.store(true, Ordering::SeqCst);
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        let r = env.run(&t, BackupMode::Full, "2026-10-02 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        assert_eq!(r[0].files_copied, 1, "only the missing file is copied: {r:?}");
        assert!(r[0].message.contains("המשיך מגיבוי"), "{}", r[0].message);
        assert_eq!(env.names(&t, 0), ["Src 2026-10-02 03h00 מלא"]);
    }

    /// Once a complete full exists, a scheduled full starts a new cycle and leaves the partial alone.
    #[test]
    fn scheduled_run_starts_over_when_a_full_is_complete() {
        let env = Env::new();
        let (src, dst) = partial_setup(&env, "Src 2026-10-02 03h00 מלא.partial", &["a.txt"]);
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        env.scheduled.store(true, Ordering::SeqCst);
        let r = env.run(&t, BackupMode::Full, "2026-10-03 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        assert_eq!(r[0].files_copied, 3, "everything is copied again: {r:?}");
        assert!(!r[0].message.contains("המשיך מגיבוי"), "{}", r[0].message);
    }

    /// Of several partial folders the one with the most data is continued, not the newest.
    #[test]
    fn the_fullest_partial_folder_is_continued() {
        let env = Env::new();
        let (src, dst) = partial_setup(&env, "Src 2026-10-01 03h00 מלא.partial", &["a.txt", "b.txt"]);
        let newer = dst.join("Src 2026-10-02 03h00 מלא.partial");
        fs::create_dir_all(&newer).unwrap();
        fs::copy(src.join("a.txt"), newer.join("a.txt")).unwrap();
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        let r = env.run(&t, BackupMode::Full, "2026-10-03 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        assert_eq!(r[0].files_copied, 1, "continued the older, fuller folder: {r:?}");
        assert!(r[0].message.contains("2026-10-01 03h00"), "{}", r[0].message);
    }

    /// A partial folder of the other kind isn't touched by a run (it's cleaned up with the next full).
    #[test]
    fn partial_of_the_other_kind_is_not_resumed() {
        let env = Env::new();
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("a.txt"), "a").unwrap();
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        let other = dst.join("Src 2026-10-01 12h00 אינקרמנטלי.partial");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("x.txt"), "x").unwrap();
        let r = env.run(&t, BackupMode::Full, "2026-10-02 03:00");
        assert_eq!(r[0].status, Some(RunStatus::Success), "{r:?}");
        assert_eq!(env.names(&t, 0), ["Src 2026-10-02 03h00 מלא"], "the old partial is pruned by the full");
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
            discard: AtomicBool::new(false),
            fresh: AtomicBool::new(false),
            scheduled: AtomicBool::new(false),
            skip_scan: AtomicBool::new(false),
            filters: crate::filters::compile(&[crate::model::FilterRule::Include { value: "*.lrcat".into() }]),
        };
        with_rule.run(&t, BackupMode::Full, "2026-10-02 03:00");
        let b = dst.join("Src 2026-10-02 03h00 מלא");
        assert!(b.join("a.lrcat").exists());
        assert!(
            !b.join("big.lrdata").exists(),
            "a file that no longer matches must not stay in the full backup"
        );
        assert_eq!(with_rule.names(&t, 0), ["Src 2026-10-02 03h00 מלא"]);
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
        check(&dst.join("Src 2026-10-01 03h00 מלא"));
        // The incremental listing applies the same filters.
        fs::write(src.join("new.txt"), "n").unwrap();
        fs::write(src.join("new.tmp"), "n").unwrap();
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!(r[0].files_copied, 1, "{r:?}");
        assert!(dst.join("Src 2026-10-02 03h00 אינקרמנטלי").join("new.txt").exists());
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
        let b = dst.join("Src 2026-10-01 03h00 מלא");
        assert!(b.join("main.lrcat").exists() && b.join("sub").join("old.lrcat").exists());
        assert!(b.join("sub").join("notes.txt").exists());
        assert!(!b.join("preview.lrdata").exists() && !b.join("sub").join("other.txt").exists());
        // The incremental listing applies the same rule.
        fs::write(src.join("new.lrcat"), "n").unwrap();
        fs::write(src.join("new.jpg"), "n").unwrap();
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!(r[0].files_copied, 1, "{r:?}");
        assert!(dst.join("Src 2026-10-02 03h00 אינקרמנטלי").join("new.lrcat").exists());
    }

    #[test]
    fn regex_rules_in_full_and_incremental() {
        use crate::model::FilterRule::Regex;
        for include in [false, true] {
            let value = if include { r"^img_\d+\.jpg$" } else { r"^img_\d+\.jpg$" };
            let env = Env::with_filters(crate::filters::compile(&[Regex {
                value: value.into(),
                include,
                folders: false,
            }]));
            let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
            fs::create_dir_all(src.join("sub")).unwrap();
            fs::write(src.join("IMG_001.jpg"), "a").unwrap();
            fs::write(src.join("sub").join("img_2.jpg"), "b").unwrap();
            fs::write(src.join("img_x.jpg"), "c").unwrap();
            fs::write(src.join("notes.txt"), "d").unwrap();
            let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
            let r = env.run(&t, BackupMode::Full, "2026-10-01 03:00");
            let b = dst.join("Src 2026-10-01 03h00 מלא");
            // include: the two numbered images. exclude: the two other files.
            let (kept, skipped) = if include { ("IMG_001.jpg", "img_x.jpg") } else { ("img_x.jpg", "IMG_001.jpg") };
            assert_eq!(r[0].files_copied, 2, "{r:?}");
            assert!(b.join(kept).exists() && !b.join(skipped).exists());
            fs::write(src.join("img_3.jpg"), "n").unwrap();
            fs::write(src.join("other.png"), "n").unwrap();
            let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
            assert_eq!(r[0].files_copied, 1, "{r:?}");
        }
    }

    #[test]
    fn folder_regex_rules_in_full_and_incremental() {
        use crate::model::FilterRule::Regex;
        // A folder regex skips matching folders at any depth, with everything inside. A file regex
        // on the same name pattern must not touch folders (and vice versa).
        let env = Env::with_filters(crate::filters::compile(&[Regex {
            value: r"^(build|cache)\d*$".into(),
            include: false,
            folders: true,
        }]));
        let (src, dst) = (env.root.join("Src"), env.root.join("dst"));
        for p in ["keep.txt", r"build\a.txt", r"build\deep\b.txt", r"sub\Cache2\c.txt", r"sub\ok\d.txt", r"rebuild\e.txt"] {
            fs::create_dir_all(src.join(p).parent().unwrap()).unwrap();
            fs::write(src.join(p), "x").unwrap();
        }
        let t = task(&[src.to_str().unwrap()], dst.to_str().unwrap());
        let r = env.run(&t, BackupMode::Full, "2026-10-01 03:00");
        let b = dst.join("Src 2026-10-01 03h00 מלא");
        assert_eq!(r[0].files_copied, 3, "{r:?}");
        assert!(b.join("keep.txt").exists() && b.join("sub").join("ok").join("d.txt").exists());
        assert!(b.join("rebuild").join("e.txt").exists());
        assert!(!b.join("build").exists() && !b.join("sub").join("Cache2").exists());
        fs::write(src.join(r"build\new.txt"), "n").unwrap();
        fs::write(src.join(r"sub\ok\new.txt"), "n").unwrap();
        let r = env.run(&t, BackupMode::Incremental, "2026-10-02 03:00");
        assert_eq!(r[0].files_copied, 1, "{r:?}");
        assert!(dst.join("Src 2026-10-02 03h00 אינקרמנטלי").join("sub").join("ok").join("new.txt").exists());
    }
}
