//! Continuing from Cobian Backup / Reflector: finding its task lists, and a one-time takeover
//! of its existing backup folders so the next run carries on from Cobian's last backup.
//!
//! Cobian keeps, per source, dated folders in the destination named after the source folder:
//! `<source folder> <timestamp>[ <type>]`, e.g. `Documents 2026-10-01 23;35;18` or
//! `Documents 2026-10-01 23;35;18 (Full)`. The timestamp format is a Cobian setting (default
//! `yyyy-mm-dd hh;nn;ss`) and the type is optional, so names are read leniently. A task without
//! "separated backups" writes into one folder named like the source (no timestamp).
//!
//! The takeover runs right before the task's first backup here (`Task.cobian_takeover`, set when the
//! user chooses to continue from Cobian's backups), so until then Cobian still sees its folders.
//! Every rename is recorded (`CobianRename`, `cobian-renames.json`) so `restore_plan` / `restore`
//! can put Cobian's names back. Takeovers from older versions have no record: their names are rebuilt
//! in a format the user picks (`CobianNaming`), only for tasks that never wrote a backup here.
//!
//! The takeover only renames those folders to this app's names (`<folderName> <date> מלא|אינקרמנטלי`,
//! same drive, instant). After that they are ordinary backups here: the incremental diff reads them
//! (an index file is built on first use), retention counts them, the backups dialog lists them.
//! Folders without a type in the name: the oldest is the full backup and the later ones count as
//! incrementals of it. That is always safe for the diff (the newest copy of each file wins) and
//! for retention (a chain is kept whole).

use crate::backup::{self, dated_folder_name, normalize, parse_folder, rename_retry};
use crate::model::{BackupMode, CobianRename, Task};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// A task list file found on this computer.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CobianList {
    pub path: String,
    /// The list's name (file name without .lst).
    pub name: String,
    /// The program folder it was found in ("Cobian Reflector", "Cobian Backup 11"...).
    pub product: String,
    pub modified: Option<DateTime<Local>>,
    /// How many tasks it holds (None if it couldn't be read).
    pub task_count: Option<usize>,
    pub error: Option<String>,
}

/// Folders where Cobian programs keep their lists, under each base folder of a "Cobian*" program.
const LIST_SUBDIRS: [&str; 5] = ["Lists", "DB", "Settings", "Lists\\Backup", ""];

fn base_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData", "APPDATA", "LOCALAPPDATA"] {
        if let Some(v) = std::env::var_os(var) {
            out.push(PathBuf::from(v));
        }
    }
    for fallback in ["C:\\Program Files", "C:\\Program Files (x86)", "C:\\ProgramData"] {
        out.push(PathBuf::from(fallback));
    }
    out
}

/// Cobian program folders on this computer (Reflector first, then newer versions).
pub fn program_dirs() -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    for base in base_dirs() {
        let Ok(rd) = fs::read_dir(&base) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name.starts_with("cobian") && e.path().is_dir() && seen.insert(e.path().to_string_lossy().to_lowercase()) {
                out.push(e.path());
            }
        }
    }
    let rank = |p: &PathBuf| {
        let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        (!n.contains("reflector"), std::cmp::Reverse(n))
    };
    out.sort_by_key(rank);
    out
}

/// The folder a file dialog should open in to pick a Cobian list: the first one that holds lists.
pub fn lists_dir() -> Option<String> {
    find_lists()
        .first()
        .and_then(|l| Path::new(&l.path).parent().map(|p| p.to_string_lossy().to_string()))
        .or_else(|| {
            let p = program_dirs().into_iter().next()?;
            Some(if p.join("Lists").is_dir() { p.join("Lists") } else { p }.to_string_lossy().to_string())
        })
}

/// Every Cobian task list (`*.lst`) on this computer, the main lists first, then newest first.
pub fn find_lists() -> Vec<CobianList> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for dir in program_dirs() {
        let product = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        for sub in LIST_SUBDIRS {
            let d = if sub.is_empty() { dir.clone() } else { dir.join(sub) };
            let Ok(rd) = fs::read_dir(&d) else { continue };
            for e in rd.flatten() {
                let p = e.path();
                let is_lst = p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lst"));
                if !is_lst || !p.is_file() || !seen.insert(p.to_string_lossy().to_lowercase()) {
                    continue;
                }
                let (task_count, error) = match fs::read(&p).map_err(|e| e.to_string()).and_then(|b| crate::cobian::parse_file(&b)) {
                    Ok(tasks) => (Some(tasks.len()), None),
                    Err(e) => (None, Some(e)),
                };
                out.push(CobianList {
                    name: p.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                    path: p.to_string_lossy().to_string(),
                    product: product.clone(),
                    modified: e.metadata().ok().and_then(|m| m.modified().ok()).map(DateTime::<Local>::from),
                    task_count,
                    error,
                });
            }
        }
    }
    out.sort_by(|a, b| {
        let main = |l: &CobianList| !l.name.eq_ignore_ascii_case("MainList");
        main(a).cmp(&main(b)).then(b.modified.cmp(&a.modified))
    });
    out
}

/// One Cobian folder and the name it gets here.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdoptFolder {
    pub from: String,
    pub to: String,
    pub kind: BackupMode,
    pub created_at: DateTime<Local>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AdoptSource {
    pub source: String,
    pub folder_name: String,
    /// Oldest first.
    pub folders: Vec<AdoptFolder>,
    /// Something is wrong or can't be taken over.
    pub warnings: Vec<String>,
    /// Old incrementals Cobian left without their full (normal: its "full copies to keep" deletes
    /// old fulls only). Not taken over, left as they are. The UI sums them into one note per task.
    pub leftovers: Option<Leftovers>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Leftovers {
    pub count: usize,
    pub first: DateTime<Local>,
    pub last: DateTime<Local>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AdoptPlan {
    pub task_id: String,
    pub destination: String,
    pub sources: Vec<AdoptSource>,
}

impl AdoptPlan {
    #[cfg(test)]
    pub fn folder_count(&self) -> usize {
        self.sources.iter().map(|s| s.folders.len()).sum()
    }
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AdoptResult {
    pub renamed: usize,
    pub errors: Vec<String>,
    /// What was renamed (Cobian's name, the new name), for the rename journal.
    #[serde(skip)]
    pub done: Vec<AdoptFolder>,
}

/// The name Cobian gives a source's backups: its last folder name ("" for a drive root).
fn cobian_short_name(path: &str) -> String {
    let p = path.trim().trim_end_matches(['\\', '/']);
    let last = p.rsplit(['\\', '/']).next().unwrap_or(p);
    if last.ends_with(':') {
        String::new()
    } else {
        last.to_string()
    }
}

/// Cobian's own name variants for a source (it can be set to drop spaces from names).
fn prefixes(short: &str) -> Vec<String> {
    let mut v = vec![short.to_string()];
    let no_spaces: String = short.chars().filter(|c| !c.is_whitespace()).collect();
    if no_spaces != short {
        v.push(no_spaces);
    }
    v
}

/// The type word Cobian may add to a name, in the languages it ships in (and its Hebrew UI).
fn type_word(w: &str) -> Option<BackupMode> {
    let w = w.trim().trim_matches(['(', ')', '[', ']']).trim().to_lowercase();
    const FULL: [&str; 9] = ["full", "completo", "completa", "complet", "complète", "vollständig", "voll", "מלא", "volledig"];
    const INC: [&str; 9] = [
        "incremental",
        "incrementale",
        "incrémental",
        "inkrementell",
        "incrementeel",
        "אינקרמנטלי",
        "מצטבר",
        "differential",
        "diferencial",
    ];
    const DIFF: [&str; 4] = ["différentiel", "differenziale", "differentieel", "דיפרנציאלי"];
    if FULL.contains(&w.as_str()) {
        Some(BackupMode::Full)
    } else if INC.contains(&w.as_str()) || DIFF.contains(&w.as_str()) {
        // A differential holds what changed since the full; read on top of it, it works like an incremental.
        Some(BackupMode::Incremental)
    } else {
        None
    }
}

/// A Cobian timestamp in any order/separators: "2026-10-01 23;35;18", "2021-01-16 2241",
/// "16-01-2021 22.41", "2026-10-0123;35;18" (spaces removed). Year first or day first.
fn parse_timestamp(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    if s.is_empty() || !s.chars().next()?.is_ascii_digit() {
        return None;
    }
    if s.chars().any(|c| c.is_alphabetic()) {
        return None;
    }
    let groups: Vec<&str> = s.split(|c: char| !c.is_ascii_digit()).filter(|g| !g.is_empty()).collect();
    let year_first = groups.first()?.len() >= 4;
    let build = |y: i32, mo: u32, d: u32, h: u32, mi: u32, sec: u32| {
        // Day first, but the "month" can't be one: month/day/year.
        let (mo, d) = if !year_first && mo > 12 && d <= 12 { (d, mo) } else { (mo, d) };
        if !(1990..=2200).contains(&y) {
            return None;
        }
        NaiveDate::from_ymd_opt(y, mo, d)?.and_hms_opt(h, mi, sec)
    };
    // Every group of its own length (unpadded formats too): Y M D [h m [s [ms]]].
    let small = |g: &str| g.len() <= 2;
    let separate = groups.len() >= 3
        && if year_first {
            groups[0].len() == 4 && groups[1..].iter().all(|g| small(g) || g.len() == 3)
        } else {
            small(groups[0]) && small(groups[1]) && groups[2].len() == 4 && groups[3..].iter().all(|g| small(g) || g.len() == 3)
        }
        && groups.len() != 4;
    if separate {
        let n: Vec<u32> = groups.iter().map(|g| g.parse().ok()).collect::<Option<_>>()?;
        let at = |i: usize| n.get(i).copied().unwrap_or(0);
        let (y, mo, d) = if year_first { (at(0), at(1), at(2)) } else { (at(2), at(1), at(0)) };
        return build(y as i32, mo, d, at(3), at(4), at(5));
    }
    // Padded fields run together ("2021-01-16 2241", spaces removed): read by position.
    let first = groups[0].len();
    let digits: String = groups.concat();
    if !(first == 2 || first >= 4) || ![8, 12, 14, 15, 16, 17].contains(&digits.len()) {
        return None;
    }
    let num = |a: usize, b: usize| digits.get(a..b).and_then(|x| x.parse::<u32>().ok());
    let (y, mo, d) = if year_first {
        (num(0, 4)?, num(4, 6)?, num(6, 8)?)
    } else {
        (num(4, 8)?, num(2, 4)?, num(0, 2)?)
    };
    let h = num(8, 10).unwrap_or(0);
    let mi = num(10, 12).unwrap_or(0);
    let sec = num(12, 14).unwrap_or(0);
    build(y as i32, mo, d, h, mi, sec)
}

/// Reads a Cobian folder name for the given prefix: its time and, if the name says, its type.
fn parse_cobian(name: &str, prefix: &str) -> Option<(NaiveDateTime, Option<BackupMode>)> {
    if prefix.is_empty() || name.len() <= prefix.len() || !name.is_char_boundary(prefix.len()) {
        return None;
    }
    let (head, rest) = name.split_at(prefix.len());
    if head.to_lowercase() != prefix.to_lowercase() {
        return None;
    }
    // Cobian puts a space (or "_") between the name and the time; with spaces removed, nothing.
    let rest = rest.strip_prefix([' ', '_']).unwrap_or(rest);
    let rest = rest.trim();
    if let Some(at) = parse_timestamp(rest) {
        return Some((at, None));
    }
    // A type at the end: "... (Full)", "... Full", "...(Full)".
    let (stamp, word) = if let Some(open) = rest.strip_suffix(')').and_then(|r| r.rfind('(')) {
        (&rest[..open], &rest[open..])
    } else {
        let i = rest.rfind([' ', '_'])?;
        (&rest[..i], &rest[i + 1..])
    };
    let at = parse_timestamp(stamp.trim_end_matches([' ', '_']))?;
    if word.chars().any(|c| c.is_ascii_digit()) || word.chars().count() > 30 {
        return None;
    }
    Some((at, type_word(word)))
}

fn to_local(at: NaiveDateTime) -> DateTime<Local> {
    Local
        .from_local_datetime(&at)
        .earliest()
        .unwrap_or_else(|| Local.from_utc_datetime(&at))
}

/// Archive files Cobian writes when it compresses (can't be continued from).
const ARCHIVES: [&str; 5] = ["zip", "7z", "sqx", "exe", "enc"];

/// What the takeover would rename for `task`. `others` = the rest of the task list (existing and
/// being imported): a Cobian name two sources share in the same destination is ambiguous and skipped.
pub fn plan(task: &Task, others: &[Task]) -> AdoptPlan {
    let dest = task.destination.trim();
    let entries: Vec<(String, bool)> = fs::read_dir(dest)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let t = e.file_type().ok()?;
                    Some((e.file_name().to_string_lossy().to_string(), t.is_dir()))
                })
                .collect()
        })
        .unwrap_or_default();
    // Names that exist (or will, from this plan), lowercase.
    let mut taken: HashSet<String> = entries.iter().map(|(n, _)| n.to_lowercase()).collect();
    let same_dest: Vec<&Task> = others
        .iter()
        .filter(|o| o.id != task.id && normalize(&o.destination) == normalize(dest))
        .collect();

    let mut sources = Vec::new();
    for src in &task.sources {
        let folder_name = backup::sanitize_folder_name(&src.folder_name);
        let mut out = AdoptSource {
            source: src.path.clone(),
            folder_name: folder_name.clone(),
            folders: Vec::new(),
            warnings: Vec::new(),
            leftovers: None,
        };
        let short = cobian_short_name(&src.path);
        if short.is_empty() {
            out.warnings
                .push("גיבוי של כונן שלם - Cobian נותן לתיקיות שלו שם שלא ניתן לזהות בוודאות, ולכן הן לא מומרות".into());
            sources.push(out);
            continue;
        }
        let shared = task
            .sources
            .iter()
            .filter(|s| s.path != src.path)
            .chain(same_dest.iter().flat_map(|o| o.sources.iter()))
            .any(|s| cobian_short_name(&s.path).to_lowercase() == short.to_lowercase());
        if shared {
            out.warnings.push(format!(
                "לעוד תיקיית מקור באותו יעד יש אותו שם (\"{short}\"), כך שאי אפשר לדעת של מי כל תיקייה של Cobian - הן לא מומרות"
            ));
            sources.push(out);
            continue;
        }

        let mut found: Vec<(String, NaiveDateTime, Option<BackupMode>)> = Vec::new();
        let mut archives = 0;
        let mut plain: Option<String> = None;
        for (name, is_dir) in &entries {
            // Already in this app's format (ours, or Cobian set to the same format).
            if parse_folder(&folder_name, name).is_some() || parse_folder(&short, name).is_some() {
                continue;
            }
            if !is_dir {
                let p = Path::new(name);
                let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
                let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                if ARCHIVES.contains(&ext.as_str()) && prefixes(&short).iter().any(|pr| parse_cobian(&stem, pr).is_some()) {
                    archives += 1;
                }
                continue;
            }
            if name.eq_ignore_ascii_case(&short) {
                plain = Some(name.clone());
                continue;
            }
            if let Some((at, kind)) = prefixes(&short).iter().find_map(|pr| parse_cobian(name, pr)) {
                found.push((name.clone(), at, kind));
            }
        }
        if archives > 0 {
            out.warnings.push(format!(
                "נמצאו {archives} גיבויים דחוסים (zip/7z) של Cobian - אי אפשר להמשיך מהם, והם נשארים כמו שהם"
            ));
        }
        // Without "separated backups" Cobian kept one folder named like the source, always up to date.
        if found.is_empty() {
            if let Some(name) = plain {
                let modified = fs::metadata(Path::new(dest).join(&name))
                    .and_then(|m| m.modified())
                    .map(|t| DateTime::<Local>::from(t).naive_local())
                    .unwrap_or_else(|_| Local::now().naive_local());
                found.push((name, modified, Some(BackupMode::Full)));
                out.warnings.push(
                    "Cobian שמר את הגיבוי בתיקייה אחת בלי תאריך - היא תיחשב לגיבוי מלא מהתאריך שבו השתנתה לאחרונה".into(),
                );
            }
        }
        found.sort_by_key(|f| f.1);
        let only_full = task.mode == BackupMode::Full;
        let mut have_full = false;
        // Incrementals older than every full left: Cobian's retention deleted their full and kept them.
        let mut orphans: Vec<NaiveDateTime> = Vec::new();
        for (name, at, kind) in found {
            let kind = match kind {
                Some(k) if !only_full => k,
                Some(_) => BackupMode::Full,
                None if only_full || !have_full => BackupMode::Full,
                None => BackupMode::Incremental,
            };
            // An incremental with no full before it has nothing to sit on.
            if kind == BackupMode::Incremental && !have_full {
                orphans.push(at);
                continue;
            }
            have_full |= kind == BackupMode::Full;
            let to = [false, true]
                .into_iter()
                .map(|secs| dated_folder_name(&folder_name, at, kind, secs))
                .find(|n| !taken.contains(&n.to_lowercase()) && !taken.contains(&format!("{n}.partial").to_lowercase()));
            let Some(to) = to else {
                out.warnings.push(format!("כבר קיימת תיקייה בשם שהגיבוי \"{name}\" היה מקבל - הוא לא מומר"));
                continue;
            };
            taken.insert(to.to_lowercase());
            out.folders.push(AdoptFolder {
                from: name,
                to,
                kind,
                created_at: to_local(at),
            });
        }
        if let (Some(first), Some(last)) = (orphans.first(), orphans.last()) {
            out.leftovers = Some(Leftovers {
                count: orphans.len(),
                first: to_local(*first),
                last: to_local(*last),
            });
        }
        sources.push(out);
    }
    AdoptPlan {
        task_id: task.id.clone(),
        destination: dest.to_string(),
        sources,
    }
}

/// Renames the planned folders (worked out again here, from what is on disk now).
pub fn apply(task: &Task, others: &[Task]) -> AdoptResult {
    let p = plan(task, others);
    let dest = Path::new(&p.destination);
    let mut result = AdoptResult::default();
    for f in p.sources.iter().flat_map(|s| s.folders.iter()) {
        match rename_retry(&dest.join(&f.from), &dest.join(&f.to)) {
            Ok(()) => {
                result.renamed += 1;
                result.done.push(f.clone());
            }
            Err(e) => result.errors.push(format!("{}: {e}", f.from)),
        }
    }
    result
}

// ---- Putting Cobian's names back ----

/// How Cobian named its backups. Only for folders taken over before the renames were recorded:
/// their original names are gone, so they are rebuilt in this format (the user picks it).
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CobianNaming {
    /// "ymd-hms" (Cobian's default, `yyyy-mm-dd hh;nn;ss`) | "ymd-hm" | "yy-hms" | "yy-hm"
    pub date: String,
    /// "" = no type | "word" = " Full" | "paren" = " (Full)"
    pub kind: String,
}

impl CobianNaming {
    fn name(&self, prefix: &str, at: NaiveDateTime, kind: BackupMode) -> String {
        let fmt = match self.date.as_str() {
            "ymd-hm" => "%Y-%m-%d %H;%M",
            "yy-hms" => "%y-%m-%d %H;%M;%S",
            "yy-hm" => "%y-%m-%d %H;%M",
            _ => "%Y-%m-%d %H;%M;%S",
        };
        let word = if kind == BackupMode::Full { "Full" } else { "Incremental" };
        let tag = match self.kind.as_str() {
            "word" => format!(" {word}"),
            "paren" => format!(" ({word})"),
            _ => String::new(),
        };
        format!("{prefix} {}{tag}", at.format(fmt))
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RestoreFolder {
    /// Its name now (this app's).
    pub from: String,
    /// The name it gets back (Cobian's).
    pub to: String,
    pub kind: BackupMode,
    /// Why it can't be renamed back; None = it can.
    pub problem: Option<String>,
    /// The folder that holds it.
    #[serde(skip)]
    dir: PathBuf,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RestorePlan {
    pub task_id: String,
    pub destination: String,
    pub folders: Vec<RestoreFolder>,
    /// No record of the renames (an older version took them over): names rebuilt from `CobianNaming`.
    pub legacy: bool,
}

const NO_DEST: &str = "תיקיית היעד לא נמצאה - האם הכונן מחובר?";
const NAME_TAKEN: &str = "כבר קיימת ביעד תיקייה בשם הזה, ולכן התיקייה לא מקבלת אותו";

/// Which folders get Cobian's names back. `journal` = all recorded renames; `never_backed_up` = the
/// task hasn't written a backup here, so all its dated folders came from a takeover (only then are
/// unrecorded ones rebuilt with `naming`).
pub fn restore_plan(task: &Task, journal: &[CobianRename], never_backed_up: bool, naming: &CobianNaming) -> RestorePlan {
    let dest = task.destination.trim();
    let mut folders = Vec::new();
    let recorded: Vec<&CobianRename> = journal.iter().filter(|r| r.task_id == task.id).collect();
    let legacy = recorded.is_empty() && never_backed_up;
    // Names given in this plan (two folders can't both get one).
    let mut taken: HashSet<(PathBuf, String)> = HashSet::new();
    let mut check = |dir: &Path, from: &str, to: &str| -> Option<String> {
        if !dir.is_dir() {
            Some(NO_DEST.into())
        } else if !dir.join(from).is_dir() {
            Some("התיקייה כבר לא קיימת ביעד: היא נמחקה לפי הגדרת השמירה, או שגיבוי מלא השתמש בה מחדש".into())
        } else if dir.join(to).exists() || !taken.insert((dir.to_path_buf(), to.to_lowercase())) {
            Some(NAME_TAKEN.into())
        } else {
            None
        }
    };
    for r in &recorded {
        let dir = PathBuf::from(r.destination.trim());
        let problem = check(&dir, &r.to, &r.from);
        folders.push(RestoreFolder {
            from: r.to.clone(),
            to: r.from.clone(),
            kind: r.kind,
            problem,
            dir,
        });
    }
    if legacy {
        let dir = PathBuf::from(dest);
        for src in &task.sources {
            let short = cobian_short_name(&src.path);
            if short.is_empty() {
                continue;
            }
            for b in backup::list_backups(dest, &src.folder_name)
                .into_iter()
                .filter(|b| !b.partial)
            {
                let to = naming.name(&short, b.created_at.naive_local(), b.kind);
                let problem = check(&dir, &b.name, &to);
                folders.push(RestoreFolder {
                    from: b.name,
                    to,
                    kind: b.kind,
                    problem,
                    dir: dir.clone(),
                });
            }
        }
        folders.reverse(); // oldest first, like the recorded ones
    }
    RestorePlan {
        task_id: task.id.clone(),
        destination: dest.to_string(),
        folders,
        legacy,
    }
}

/// Renames the plan's folders back (those without a problem) and removes this app's index file
/// from them. `done` holds them as (Cobian's name = `from`, this app's name = `to`).
pub fn restore(plan: &RestorePlan) -> AdoptResult {
    let mut result = AdoptResult::default();
    for f in plan.folders.iter().filter(|f| f.problem.is_none()) {
        let to = f.dir.join(&f.to);
        match rename_retry(&f.dir.join(&f.from), &to) {
            Ok(()) => {
                crate::manifest::remove(&to);
                result.renamed += 1;
                result.done.push(AdoptFolder {
                    from: f.to.clone(),
                    to: f.from.clone(),
                    kind: f.kind,
                    created_at: Local::now(),
                });
            }
            Err(e) => result.errors.push(format!("{}: {e}", f.from)),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    #[test]
    fn reads_cobian_timestamps() {
        assert_eq!(parse_timestamp("2026-10-01 23;35;18"), Some(dt("2026-10-01 23:35:18")));
        assert_eq!(parse_timestamp("2021-01-16 2241"), Some(dt("2021-01-16 22:41:00")));
        assert_eq!(parse_timestamp("2026-10-0123;35;18"), Some(dt("2026-10-01 23:35:18")));
        assert_eq!(parse_timestamp("16-01-2021 22.41"), Some(dt("2021-01-16 22:41:00")));
        assert_eq!(parse_timestamp("2026-1-5 3;4;5"), Some(dt("2026-01-05 03:04:05")));
        assert_eq!(parse_timestamp("2026-10-01"), Some(dt("2026-10-01 00:00:00")));
        assert_eq!(parse_timestamp("2026-10-01 23;35;18;671"), Some(dt("2026-10-01 23:35:18")));
        assert_eq!(parse_timestamp("old"), None);
        assert_eq!(parse_timestamp("2026-13-45 10;00"), None);
    }

    #[test]
    fn reads_cobian_folder_names() {
        let at = dt("2026-10-01 23:35:18");
        assert_eq!(parse_cobian("Documents 2026-10-01 23;35;18", "Documents"), Some((at, None)));
        assert_eq!(
            parse_cobian("documents 2026-10-01 23;35;18 (Full)", "Documents"),
            Some((at, Some(BackupMode::Full)))
        );
        assert_eq!(
            parse_cobian("Documents 2026-10-01 23;35;18 Incremental", "Documents"),
            Some((at, Some(BackupMode::Incremental)))
        );
        assert_eq!(
            parse_cobian("Documents_2026-10-01 23;35;18(Differential)", "Documents"),
            Some((at, Some(BackupMode::Incremental)))
        );
        assert_eq!(parse_cobian("MyDocs2026-10-0123;35;18", "MyDocs"), Some((at, None)));
        assert_eq!(parse_cobian("Documents old", "Documents"), None);
        assert_eq!(parse_cobian("Documents2 2026-10-01 23;35;18", "Documents"), None);
        assert_eq!(parse_cobian("Documents 12 2026-10-01 23;35;18", "Documents"), None);
        assert_eq!(parse_cobian("Documents 2026-10-01 23;35;18 copy 2", "Documents"), None);
    }

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn task(dest: &Path, sources: &[&str], mode: BackupMode) -> Task {
        Task {
            id: "t".into(),
            name: "t".into(),
            destination: dest.to_string_lossy().to_string(),
            sources: sources
                .iter()
                .map(|p| Source {
                    path: p.to_string(),
                    folder_name: backup::default_folder_name(p),
                })
                .collect(),
            mode,
            ..Default::default()
        }
    }

    #[test]
    fn plans_and_renames_cobian_folders() {
        let root = Tmp(std::env::temp_dir().join(format!("backuper-cobian-{}", uuid::Uuid::new_v4())));
        let dest = root.0.join("dst");
        for d in [
            "Docs 2026-09-28 03;00;00",
            "Docs 2026-09-29 03;00;00",
            "Docs 2026-09-30 03;00;00 (Full)",
            "Docs 2026-10-01 03;00;12 (Incremental)",
            "Docs 2026-10-01 03-00 אינקרמנטלי", // already ours
            "Pics 2026-10-01 03;00;00",
            "Docs old", // not Cobian's
        ] {
            fs::create_dir_all(dest.join(d)).unwrap();
        }
        fs::write(dest.join("Music 2026-10-01 03;00;00.zip"), "z").unwrap();
        let t = task(&dest, &["C:\\Users\\me\\Docs", "D:\\Music"], BackupMode::Incremental);
        let p = plan(&t, &[]);
        let docs = &p.sources[0];
        let names: Vec<(&str, &str)> = docs.folders.iter().map(|f| (f.from.as_str(), f.to.as_str())).collect();
        assert_eq!(
            names,
            [
                ("Docs 2026-09-28 03;00;00", "Docs 2026-09-28 03-00 מלא"),
                ("Docs 2026-09-29 03;00;00", "Docs 2026-09-29 03-00 אינקרמנטלי"),
                ("Docs 2026-09-30 03;00;00 (Full)", "Docs 2026-09-30 03-00 מלא"),
                // The minute is taken by our own folder: with seconds.
                ("Docs 2026-10-01 03;00;12 (Incremental)", "Docs 2026-10-01 03-00-12 אינקרמנטלי"),
            ]
        );
        assert!(p.sources[1].folders.is_empty());
        assert_eq!(p.sources[1].warnings.len(), 1, "zip archive: {:?}", p.sources[1].warnings);

        // Another task in the same destination also backs up a "Docs": ambiguous.
        let other = task(&dest, &["E:\\Docs"], BackupMode::Full);
        let other = Task { id: "o".into(), ..other };
        assert!(plan(&t, &[other]).sources[0].folders.is_empty());

        let r = apply(&t, &[]);
        assert_eq!((r.renamed, r.errors.len()), (4, 0));
        let kinds: Vec<BackupMode> = backup::list_backups(&t.destination, "Docs").iter().map(|b| b.kind).collect();
        use BackupMode::*;
        assert_eq!(kinds, [Incremental, Incremental, Full, Incremental, Full], "newest first");
        assert!(dest.join("Pics 2026-10-01 03;00;00").is_dir(), "other sources' folders untouched");
        assert!(dest.join("Docs old").is_dir());
        assert_eq!(plan(&t, &[]).folder_count(), 0, "nothing left to take over");
    }

    #[test]
    fn orphan_incrementals_get_one_note() {
        let root = Tmp(std::env::temp_dir().join(format!("backuper-cobian-{}", uuid::Uuid::new_v4())));
        let dest = root.0.join("dst");
        // Cobian kept one full: the older fulls are gone, their incrementals are still there.
        for d in [
            "Pics 2026-10-03 01;58;50 (Incremental)",
            "Pics 2026-10-04 01;59;02 (Incremental)",
            "Pics 2026-11-05 01;58;00 (Incremental)",
            "Pics 2026-11-06 01;58;00 (Full)",
            "Pics 2026-11-07 01;58;00 (Incremental)",
        ] {
            fs::create_dir_all(dest.join(d)).unwrap();
        }
        let t = task(&dest, &["E:\\Pics"], BackupMode::Incremental);
        let p = plan(&t, &[]);
        assert_eq!(p.folder_count(), 2);
        assert!(p.sources[0].warnings.is_empty(), "{:?}", p.sources[0].warnings);
        let l = p.sources[0].leftovers.as_ref().expect("leftovers");
        assert_eq!(l.count, 3);
        assert_eq!(l.first.naive_local(), dt("2026-10-03 01:58:50"));
        assert_eq!(l.last.naive_local(), dt("2026-11-05 01:58:00"));
    }

    #[test]
    fn full_task_and_single_folder() {
        let root = Tmp(std::env::temp_dir().join(format!("backuper-cobian-{}", uuid::Uuid::new_v4())));
        let dest = root.0.join("dst");
        fs::create_dir_all(dest.join("LR 2026-09-28 03;00;00 Incremental")).unwrap();
        fs::create_dir_all(dest.join("Mail")).unwrap();
        let t = task(&dest, &["C:\\LR", "C:\\Mail"], BackupMode::Full);
        let p = plan(&t, &[]);
        assert_eq!(p.sources[0].folders[0].kind, BackupMode::Full, "a full-only task has only fulls");
        assert_eq!(p.sources[1].folders.len(), 1);
        assert!(p.sources[1].folders[0].to.starts_with("Mail "));
        assert!(p.sources[1].folders[0].to.ends_with(" מלא"));
    }

    #[test]
    fn restores_cobian_names() {
        let root = Tmp(std::env::temp_dir().join(format!("backuper-cobian-{}", uuid::Uuid::new_v4())));
        let dest = root.0.join("dst");
        for d in ["Docs 2026-09-28 03;00;07 (Full)", "Docs 2026-09-29 03;00;00 (Incremental)"] {
            fs::create_dir_all(dest.join(d)).unwrap();
        }
        let t = task(&dest, &["C:\\Docs"], BackupMode::Incremental);
        let r = apply(&t, &[]);
        assert_eq!(r.done.len(), 2);
        let journal: Vec<CobianRename> = r
            .done
            .iter()
            .map(|f| CobianRename {
                task_id: t.id.clone(),
                destination: t.destination.clone(),
                from: f.from.clone(),
                to: f.to.clone(),
                kind: f.kind,
                at: Local::now(),
            })
            .collect();
        // A backup this app made later, and the index file a run left in a taken-over folder.
        fs::create_dir_all(dest.join("Docs 2026-10-01 03-00 אינקרמנטלי")).unwrap();
        crate::manifest::write(&dest.join(&journal[0].to), &[]);
        assert!(crate::manifest::read(&dest.join(&journal[0].to)).is_some());

        let p = restore_plan(&t, &journal, false, &CobianNaming::default());
        assert!(!p.legacy);
        assert_eq!(p.folders.len(), 2);
        assert!(p.folders.iter().all(|f| f.problem.is_none()), "{:?}", p.folders);
        let r = restore(&p);
        assert_eq!((r.renamed, r.errors.len()), (2, 0));
        assert!(dest.join("Docs 2026-09-28 03;00;07 (Full)").is_dir(), "the exact original name");
        assert!(dest.join("Docs 2026-09-29 03;00;00 (Incremental)").is_dir());
        assert!(crate::manifest::read(&dest.join("Docs 2026-09-28 03;00;07 (Full)")).is_none(), "index removed");
        let left: Vec<String> = backup::list_backups(&t.destination, "Docs").into_iter().map(|b| b.name).collect();
        assert_eq!(left, ["Docs 2026-10-01 03-00 אינקרמנטלי"], "this app's own backup stays");

        // Recorded renames whose folders are gone (or already back) can't be restored again.
        let p = restore_plan(&t, &journal, false, &CobianNaming::default());
        assert!(p.folders.iter().all(|f| f.problem.is_some()));

        // Taken over by an older version (no record), task never backed up: names rebuilt.
        fs::remove_dir_all(dest.join("Docs 2026-10-01 03-00 אינקרמנטלי")).unwrap();
        apply(&t, &[]);
        assert!(restore_plan(&t, &[], false, &CobianNaming::default()).folders.is_empty(), "the task has backups of its own");
        let naming = CobianNaming {
            date: "ymd-hm".into(),
            kind: "paren".into(),
        };
        let p = restore_plan(&t, &[], true, &naming);
        assert!(p.legacy);
        let names: Vec<(&str, &str)> = p.folders.iter().map(|f| (f.from.as_str(), f.to.as_str())).collect();
        assert_eq!(
            names,
            [
                ("Docs 2026-09-28 03-00 מלא", "Docs 2026-09-28 03;00 (Full)"),
                ("Docs 2026-09-29 03-00 אינקרמנטלי", "Docs 2026-09-29 03;00 (Incremental)"),
            ]
        );
        assert_eq!(
            CobianNaming::default().name("Docs", dt("2026-09-28 03:00:07"), BackupMode::Full),
            "Docs 2026-09-28 03;00;07"
        );
        assert_eq!(restore(&p).renamed, 2);
        assert!(dest.join("Docs 2026-09-28 03;00 (Full)").is_dir());
    }
}
