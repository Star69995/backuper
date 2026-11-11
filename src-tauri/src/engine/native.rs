//! The incremental path, without robocopy: `list_files` lists a folder tree (applying the same
//! filters robocopy does) and `copy_files` copies an explicit list of files (the changes).
//! std::fs::copy uses CopyFileExW on Windows, so attributes and the modified time are preserved.

use super::{is_disk_full, CopyResult, CopyStats, EngineEvent, Filters, ListedFile, Outcome, ALWAYS_EXCLUDED_DIRS, DISK_FULL_MESSAGE};
use crate::model::FileError;
use regex::{Regex, RegexBuilder};
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_ERRORS: usize = 50;

const ATTR_READONLY: u32 = 0x1;
const ATTR_HIDDEN: u32 = 0x2;
const ATTR_SYSTEM: u32 = 0x4;
const ATTR_DIRECTORY: u32 = 0x10;
const ATTR_REPARSE_POINT: u32 = 0x400;
/// Reparse tags of links to another place (junctions, symlinks) - robocopy's /XJ skips them.
/// Cloud placeholders (OneDrive) are reparse points too, but not links, so they're listed.
const TAG_NAME_SURROGATE: u32 = 0x2000_0000;
/// FILETIME of 1970-01-01.
const UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;
const TICKS_PER_SEC: u64 = 10_000_000;

#[repr(C)]
struct FindData {
    attributes: u32,
    created: [u32; 2],
    accessed: [u32; 2],
    written: [u32; 2],
    size_high: u32,
    size_low: u32,
    /// The reparse tag, when `attributes` has ATTR_REPARSE_POINT.
    reserved0: u32,
    reserved1: u32,
    name: [u16; 260],
    /// The 8.3 name (empty when there is none).
    short_name: [u16; 14],
}

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

#[link(name = "kernel32")]
extern "system" {
    fn FindFirstFileExW(name: *const u16, level: i32, data: *mut FindData, op: i32, filter: *const u8, flags: u32) -> isize;
    fn FindNextFileW(handle: isize, data: *mut FindData) -> i32;
    fn FindClose(handle: isize) -> i32;
}

#[link(name = "ntdll")]
extern "system" {
    fn RtlIsNameInExpression(expr: *const UnicodeString, name: *const UnicodeString, ignore_case: u8, upcase: *const u16) -> u8;
}

/// A wildcard pattern, matched the way robocopy matches its file specs and /XF /XD: DOS wildcards
/// (`*.*` also matches names without a dot, `?` can match nothing before a dot or at the end),
/// case-insensitive.
struct Pattern(Vec<u16>);

impl Pattern {
    fn new(p: &str) -> Self {
        let chars: Vec<char> = p.chars().collect();
        let mut expr = String::new();
        for (i, &c) in chars.iter().enumerate() {
            let next = chars.get(i + 1).copied();
            match c {
                '?' => expr.push('>'),
                '*' if next == Some('.') => expr.push('<'),
                '.' if matches!(next, Some('?' | '*')) => expr.push('"'),
                // ignore_case wants an upper-case expression.
                c => {
                    let up: Vec<char> = c.to_uppercase().collect();
                    expr.push(if up.len() == 1 { up[0] } else { c });
                }
            }
        }
        Pattern(expr.encode_utf16().collect())
    }

    fn matches(&self, name: &[u16]) -> bool {
        let as_ustr = |s: &[u16]| UnicodeString {
            length: (s.len() * 2) as u16,
            maximum_length: (s.len() * 2) as u16,
            buffer: s.as_ptr(),
        };
        if name.is_empty() || name.len() > 32_000 || self.0.len() > 32_000 {
            return false;
        }
        let (expr, name) = (as_ustr(&self.0), as_ustr(name));
        unsafe { RtlIsNameInExpression(&expr, &name, 1, std::ptr::null()) != 0 }
    }
}

struct Entry {
    name: Vec<u16>,
    short_name: Vec<u16>,
    attributes: u32,
    reparse_tag: u32,
    size: u64,
    modified: u64,
}

impl Entry {
    fn is_link(&self) -> bool {
        self.attributes & ATTR_REPARSE_POINT != 0 && self.reparse_tag & TAG_NAME_SURROGATE != 0
    }

    /// robocopy matches wildcards against the long and the 8.3 name (`*.htm` matches "page.html").
    fn matches_any(&self, patterns: &[Pattern]) -> bool {
        patterns
            .iter()
            .any(|p| p.matches(&self.name) || (!self.short_name.is_empty() && p.matches(&self.short_name)))
    }
}

fn until_nul(s: &[u16]) -> Vec<u16> {
    s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())].to_vec()
}

/// One folder's entries ("." and ".." left out). `short_names` = also fetch the 8.3 names.
fn read_dir(dir: &[u16], short_names: bool) -> io::Result<Vec<Entry>> {
    let mut pattern = dir.to_vec();
    pattern.extend("\\*".encode_utf16());
    pattern.push(0);
    let mut d: FindData = unsafe { std::mem::zeroed() };
    // FindExInfoStandard (0) includes the 8.3 name, FindExInfoBasic (1) skips it and is faster.
    let level = if short_names { 0 } else { 1 };
    let h = unsafe { FindFirstFileExW(pattern.as_ptr(), level, &mut d, 0, std::ptr::null(), 0) };
    if h == -1 {
        let e = io::Error::last_os_error();
        // An empty drive root has no "." entry, so nothing at all is found.
        return if e.raw_os_error() == Some(2) { Ok(Vec::new()) } else { Err(e) };
    }
    let mut out = Vec::new();
    let result = loop {
        let name = until_nul(&d.name);
        if name != [b'.' as u16] && name != [b'.' as u16, b'.' as u16] {
            out.push(Entry {
                name,
                short_name: until_nul(&d.short_name),
                attributes: d.attributes,
                reparse_tag: d.reserved0,
                size: ((d.size_high as u64) << 32) | d.size_low as u64,
                modified: ((d.written[1] as u64) << 32) | d.written[0] as u64,
            });
        }
        if unsafe { FindNextFileW(h, &mut d) } == 0 {
            let e = io::Error::last_os_error();
            // ERROR_NO_MORE_FILES
            break if e.raw_os_error() == Some(18) { Ok(out) } else { Err(e) };
        }
    };
    unsafe { FindClose(h) };
    result
}

/// `\\?\` form, so paths longer than 260 characters work.
fn verbatim(p: &Path) -> io::Result<Vec<u16>> {
    let s = std::path::absolute(p)?.to_string_lossy().trim_end_matches('\\').to_string();
    let v = if s.starts_with(r"\\?\") {
        s
    } else if let Some(unc) = s.strip_prefix(r"\\") {
        format!(r"\\?\UNC\{unc}")
    } else {
        format!(r"\\?\{s}")
    };
    Ok(v.encode_utf16().collect())
}

/// `Filters` compiled for matching, with robocopy's meaning of each one.
struct Rules {
    include: Vec<Pattern>,
    exclude_files: Vec<Pattern>,
    /// /XF entries with a backslash: full paths of single files.
    exclude_file_paths: Vec<Pattern>,
    include_regex: Vec<Regex>,
    exclude_regex: Vec<Regex>,
    exclude_dir_regex: Vec<Regex>,
    exclude_dir_names: Vec<Pattern>,
    /// /XD entries with a backslash: full paths (robocopy never matches them as relative paths).
    exclude_dir_paths: Vec<Pattern>,
    /// The entries of `exclude_dir_paths` / `exclude_file_paths` with a wildcard: robocopy rejects those on its
    /// command line, so `list_files` notes what they match (`Listing::wild_dirs` / `wild_files`).
    exclude_dir_wild: Vec<Pattern>,
    exclude_file_wild: Vec<Pattern>,
    /// The source as robocopy prints it, for matching full paths.
    base: String,
    /// /MAX: larger files are skipped.
    max_size: Option<u64>,
    /// /MAXAGE: files last modified before this FILETIME are skipped.
    min_modified: Option<u64>,
    /// /XA: files (not folders) with any of these attributes are skipped.
    skip_attributes: u32,
    short_names: bool,
}

/// Case-insensitive like the rest of Windows. Err = the message to show the user.
pub fn compile_regex(p: &str) -> Result<Regex, String> {
    RegexBuilder::new(p)
        .case_insensitive(true)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| e.to_string())
}

/// A folder / file entry with a wildcard and a backslash (`*\src-tauri\target`): robocopy's /XD and /XF reject
/// it ("Invalid Parameter"), so `resolve_regex` swaps it for the full paths it matches.
fn is_wild_path(s: &str) -> bool {
    let s = s.trim();
    s.contains('\\') && s.contains(['*', '?'])
}

impl Rules {
    fn new(source: &Path, f: &Filters) -> Self {
        let f = &f.for_source(source);
        let patterns = |v: &[String]| -> Vec<Pattern> {
            v.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).map(Pattern::new).collect()
        };
        let (dir_paths, dir_names): (Vec<String>, Vec<String>) = f
            .exclude_dirs
            .iter()
            .map(|d| d.trim().trim_end_matches('\\').to_string())
            .chain(ALWAYS_EXCLUDED_DIRS.iter().map(|d| d.to_string()))
            .partition(|d| d.contains('\\'));
        let (file_paths, file_names): (Vec<String>, Vec<String>) =
            f.exclude_files.iter().cloned().partition(|n| n.contains('\\'));
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() * TICKS_PER_SEC + UNIX_EPOCH_TICKS;
        Rules {
            include: patterns(&f.include_files),
            exclude_files: patterns(&file_names),
            exclude_file_paths: patterns(&file_paths),
            // Validated when saved; a rule that still fails to compile skips nothing.
            include_regex: f.include_regex.iter().filter_map(|p| compile_regex(p).ok()).collect(),
            exclude_regex: f.exclude_regex.iter().filter_map(|p| compile_regex(p).ok()).collect(),
            exclude_dir_regex: f.exclude_dir_regex.iter().filter_map(|p| compile_regex(p).ok()).collect(),
            exclude_dir_names: patterns(&dir_names),
            exclude_dir_paths: patterns(&dir_paths),
            exclude_dir_wild: patterns(&dir_paths.iter().filter(|p| is_wild_path(p)).cloned().collect::<Vec<_>>()),
            exclude_file_wild: patterns(&file_paths.iter().filter(|p| is_wild_path(p)).cloned().collect::<Vec<_>>()),
            base: source.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_string(),
            max_size: f.max_size,
            min_modified: f.max_age_days.map(|d| now.saturating_sub(d as u64 * 86_400 * TICKS_PER_SEC)),
            skip_attributes: if f.exclude_hidden { ATTR_HIDDEN } else { 0 } | if f.exclude_system { ATTR_SYSTEM } else { 0 },
            short_names: !(f.include_files.is_empty() && f.exclude_files.is_empty() && f.exclude_dirs.is_empty()),
        }
    }

    fn skip_dir(&self, e: &Entry, rel: &str) -> bool {
        e.matches_any(&self.exclude_dir_names)
            || (!self.exclude_dir_paths.is_empty() && {
                let full: Vec<u16> = format!("{}\\{rel}", self.base).encode_utf16().collect();
                self.exclude_dir_paths.iter().any(|p| p.matches(&full))
            })
    }

    /// The folder's name when a folder regex matches it (the folder is skipped).
    fn regex_skips_dir(&self, e: &Entry) -> Option<String> {
        if self.exclude_dir_regex.is_empty() {
            return None;
        }
        let name = String::from_utf16_lossy(&e.name);
        self.exclude_dir_regex.iter().any(|r| r.is_match(&name)).then_some(name)
    }

    fn regex_ok(&self, e: &Entry) -> bool {
        if self.include_regex.is_empty() && self.exclude_regex.is_empty() {
            return true;
        }
        let name = String::from_utf16_lossy(&e.name);
        self.include_regex.iter().all(|r| r.is_match(&name)) && !self.exclude_regex.iter().any(|r| r.is_match(&name))
    }

    fn skip_file_path(&self, rel: &str) -> bool {
        !self.exclude_file_paths.is_empty() && {
            let full: Vec<u16> = format!("{}\\{rel}", self.base).encode_utf16().collect();
            self.exclude_file_paths.iter().any(|p| p.matches(&full))
        }
    }

    /// The full path of a folder / file when a wildcard path rule matches it (see `exclude_dir_wild`).
    fn wild_match(&self, patterns: &[Pattern], rel: &str) -> Option<String> {
        if patterns.is_empty() {
            return None;
        }
        let full = format!("{}\\{rel}", self.base);
        let w: Vec<u16> = full.encode_utf16().collect();
        patterns.iter().any(|p| p.matches(&w)).then_some(full)
    }

    fn keep_file(&self, e: &Entry) -> bool {
        (self.include.is_empty() || e.matches_any(&self.include))
            && !e.matches_any(&self.exclude_files)
            && self.regex_ok(e)
            && self.max_size.map_or(true, |m| e.size <= m)
            && self.min_modified.map_or(true, |m| e.modified >= m)
            && e.attributes & self.skip_attributes == 0
    }
}

/// One folder or file on a path, for `verdict` (from `find`, or made up for a path that isn't on disk).
#[derive(Debug, Clone)]
pub struct Item {
    pub name: String,
    /// The 8.3 name (empty when there is none or it isn't known).
    pub short_name: String,
    pub attributes: u32,
    pub reparse_tag: u32,
    pub size: u64,
    /// FILETIME.
    pub modified: u64,
}

impl Item {
    fn entry(&self) -> Entry {
        Entry {
            name: self.name.encode_utf16().collect(),
            short_name: self.short_name.encode_utf16().collect(),
            attributes: self.attributes,
            reparse_tag: self.reparse_tag,
            size: self.size,
            modified: self.modified,
        }
    }

    pub fn is_dir(&self) -> bool {
        self.attributes & ATTR_DIRECTORY != 0
    }
}

/// The folder or file at `path` as a directory listing reports it (with its 8.3 name), or None.
pub fn find(path: &Path) -> Option<Item> {
    let p = path.to_string_lossy();
    if p.contains(['*', '?']) {
        return None;
    }
    let mut w = verbatim(path).ok()?;
    w.push(0);
    let mut d: FindData = unsafe { std::mem::zeroed() };
    let h = unsafe { FindFirstFileExW(w.as_ptr(), 0, &mut d, 0, std::ptr::null(), 0) };
    if h == -1 {
        return None;
    }
    unsafe { FindClose(h) };
    Some(Item {
        name: String::from_utf16_lossy(&until_nul(&d.name)),
        short_name: String::from_utf16_lossy(&until_nul(&d.short_name)),
        attributes: d.attributes,
        reparse_tag: d.reserved0,
        size: ((d.size_high as u64) << 32) | d.size_low as u64,
        modified: ((d.written[1] as u64) << 32) | d.written[0] as u64,
    })
}

/// What a backup of `base` does with one path under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Kept,
    /// Inside (or is) a link to another place, given relative to the base: never followed (/XJ).
    Link(String),
    /// Inside (or is) a skipped folder, given relative to the base.
    Folder(String),
    /// The file itself is skipped.
    File,
}

/// `parts` = each folder under `base` on the way, then the item itself. The same decisions
/// `list_files` makes while walking the tree, so the filter check can't drift from a real backup.
pub fn verdict(base: &str, parts: &[Item], filters: &Filters) -> Verdict {
    let rules = Rules::new(Path::new(base), filters);
    let mut rel = String::new();
    for (i, p) in parts.iter().enumerate() {
        let e = p.entry();
        rel = if rel.is_empty() { p.name.clone() } else { format!("{rel}\\{}", p.name) };
        if e.is_link() {
            return Verdict::Link(rel);
        }
        if i + 1 < parts.len() || p.is_dir() {
            if rules.skip_dir(&e, &rel) || rules.regex_skips_dir(&e).is_some() {
                return Verdict::Folder(rel);
            }
        } else if !rules.keep_file(&e) || rules.skip_file_path(&rel) {
            return Verdict::File;
        }
    }
    Verdict::Kept
}

/// Longest list of names handed to robocopy on its command line (the limit is 32,767 characters).
const MAX_NAMES_LEN: usize = 28_000;

const TOO_MANY: &str = "יותר מדי שמות שונים תואמים לביטוי הרגולרי, ואי אפשר להעביר אותם לרובוקופי. צמצמו את הביטוי, או הוסיפו כלל שמצמצם את הקבצים (למשל סיומות).";

/// robocopy has no regular expressions, so list the source once and turn the regex rules into
/// plain names it understands. Folder regexes become folder names to skip (a regex matches the
/// name only, so every folder with a matching name is skipped anyway). File regexes become either
/// "skip these names" or "copy only these names", whichever list is shorter. Ok(None) = cancelled.
/// Folder / file rules with a wildcard and a backslash (`*\src-tauri\target`), which robocopy rejects, go the
/// same way: replaced by the full paths they match.
pub fn resolve_regex(root: &Path, filters: &Filters, cancel: &AtomicBool) -> Result<Option<Filters>, String> {
    // robocopy gets the source's relative folder / file paths in full.
    let filters = &filters.for_source(root);
    let wild = filters.exclude_dirs.iter().chain(&filters.exclude_files).any(|s| is_wild_path(s));
    if !wild && filters.include_regex.is_empty() && filters.exclude_regex.is_empty() && filters.exclude_dir_regex.is_empty() {
        return Ok(Some(filters.clone()));
    }
    let mut out = filters.clone();
    out.include_regex.clear();
    out.exclude_regex.clear();
    // Skips the folders matching a folder regex and notes their names.
    let Some(listing) = list_files(root, &out, cancel)? else { return Ok(None) };
    out.exclude_dir_regex.clear();
    out.exclude_dirs.retain(|s| !is_wild_path(s));
    out.exclude_files.retain(|s| !is_wild_path(s));
    let dirs_len = listing.regex_dirs.iter().chain(&listing.wild_dirs).map(|n| n.len() + 3).sum::<usize>()
        + listing.wild_files.iter().map(|n| n.len() + 3).sum::<usize>();
    if dirs_len > MAX_NAMES_LEN {
        return Err(TOO_MANY.into());
    }
    out.exclude_dirs.extend(listing.regex_dirs);
    out.exclude_dirs.extend(listing.wild_dirs);
    out.exclude_files.extend(listing.wild_files);
    if filters.include_regex.is_empty() && filters.exclude_regex.is_empty() {
        return Ok(Some(out));
    }
    let budget = MAX_NAMES_LEN - dirs_len;
    let rules = Rules::new(root, filters);
    let mut seen = HashSet::new();
    let (mut kept, mut dropped) = (Vec::new(), Vec::new());
    for f in listing.files {
        let name = f.rel.rsplit('\\').next().unwrap_or(&f.rel).to_string();
        if !seen.insert(name.clone()) {
            continue;
        }
        let e = Entry {
            name: name.encode_utf16().collect(),
            short_name: Vec::new(),
            attributes: 0,
            reparse_tag: 0,
            size: 0,
            modified: 0,
        };
        if rules.regex_ok(&e) {
            kept.push(name)
        } else {
            dropped.push(name)
        }
    }
    let len = |v: &[String]| v.iter().map(|n| n.len() + 3).sum::<usize>();
    if kept.is_empty() {
        // Nothing matches.
        out.exclude_files.push("*".into());
    } else if len(&dropped) <= len(&kept) {
        if len(&dropped) > budget {
            return Err(TOO_MANY.into());
        }
        out.exclude_files.extend(dropped);
    } else {
        if len(&kept) > budget {
            return Err(TOO_MANY.into());
        }
        out.include_files = kept;
    }
    Ok(Some(out))
}

/// A folder tree's files, plus the folders that couldn't be read.
#[derive(Default)]
pub struct Listing {
    pub files: Vec<ListedFile>,
    /// The first MAX_ERRORS of the unreadable folders.
    pub errors: Vec<FileError>,
    pub failed_dirs: u64,
    /// Names of the folders skipped by a folder regex (for `resolve_regex`).
    pub regex_dirs: BTreeSet<String>,
    /// Full paths of the folders / files skipped by a rule with a wildcard and a backslash (for `resolve_regex`).
    pub wild_dirs: BTreeSet<String>,
    pub wild_files: BTreeSet<String>,
}

/// Every file under `root` that passes `filters`, as robocopy's own listing would have it
/// (same filter meanings: DOS wildcards on the long and the 8.3 name, /XJ, /XA, /MAX, /MAXAGE).
/// One FindFirstFileExW pass per folder gives size, modified time and attributes without opening
/// any file, which keeps this fast on big trees (robocopy /L plus a stat per file was ~40x slower).
/// Err = the root itself can't be read. Ok(None) = cancelled.
pub fn list_files(root: &Path, filters: &Filters, cancel: &AtomicBool) -> Result<Option<Listing>, String> {
    let rules = Rules::new(root, filters);
    let root_w = verbatim(root).map_err(|e| e.to_string())?;
    let mut out = Listing::default();
    // Folders still to read, relative to the root.
    let mut pending = vec![String::new()];
    while let Some(rel) = pending.pop() {
        if cancel.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut dir = root_w.clone();
        if !rel.is_empty() {
            dir.push(b'\\' as u16);
            dir.extend(rel.encode_utf16());
        }
        let entries = match read_dir(&dir, rules.short_names) {
            Ok(v) => v,
            Err(e) if rel.is_empty() => return Err(e.to_string()),
            // Gone since its parent was read.
            Err(e) if matches!(e.raw_os_error(), Some(2 | 3)) => continue,
            Err(e) => {
                out.failed_dirs += 1;
                if out.errors.len() < MAX_ERRORS {
                    out.errors.push(FileError {
                        path: format!("{}\\{rel}", rules.base),
                        code: e.raw_os_error().map(|c| c as u32),
                        message: e.to_string(),
                        debug: String::new(),
                    });
                }
                continue;
            }
        };
        for e in entries {
            if e.is_link() {
                continue;
            }
            let name = String::from_utf16_lossy(&e.name);
            let child = if rel.is_empty() { name } else { format!("{rel}\\{name}") };
            if e.attributes & ATTR_DIRECTORY != 0 {
                if rules.skip_dir(&e, &child) {
                    out.wild_dirs.extend(rules.wild_match(&rules.exclude_dir_wild, &child));
                    continue;
                }
                match rules.regex_skips_dir(&e) {
                    Some(name) => {
                        out.regex_dirs.insert(name);
                    }
                    None => pending.push(child),
                }
            } else if rules.keep_file(&e) && !rules.skip_file_path(&child) {
                out.files.push(ListedFile {
                    rel: child,
                    size: e.size,
                    modified: e.modified,
                });
            } else {
                out.wild_files.extend(rules.wild_match(&rules.exclude_file_wild, &child));
            }
        }
    }
    Ok(Some(out))
}

/// What `prune_filtered` removed.
#[derive(Default, Debug)]
pub struct Pruned {
    pub files: u64,
    pub bytes: u64,
    /// Items that should have gone but couldn't be removed (in use, no permission).
    pub failed: u64,
}

/// Files and bytes under `dir`, to report before it is removed.
fn tree_size(dir: &Path) -> (u64, u64) {
    let (mut files, mut bytes) = (0, 0);
    let mut pending = vec![dir.to_path_buf()];
    while let Some(d) = pending.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                pending.push(e.path());
            } else {
                files += 1;
                bytes += m.len();
            }
        }
    }
    (files, bytes)
}

/// Makes a backup folder that already holds files (the previous full, updated in place, or a
/// resumed copy) match `filters` as they are now: the folders and files in `target` that a backup of
/// `source` would skip are removed. robocopy's mirror never purges what its filters exclude, so
/// without this a rule added since the folder was written would leave its items there. The rules see
/// each item as at the same relative path under `source` (so path rules work), with the size, time and
/// attributes the copy kept. Ok(None) = cancelled.
pub fn prune_filtered(source: &Path, target: &Path, filters: &Filters, cancel: &AtomicBool) -> Result<Option<Pruned>, String> {
    let rules = Rules::new(source, filters);
    let root_w = verbatim(target).map_err(|e| e.to_string())?;
    let mut out = Pruned::default();
    let mut pending = vec![String::new()];
    while let Some(rel) = pending.pop() {
        if cancel.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let mut dir = root_w.clone();
        if !rel.is_empty() {
            dir.push(b'\\' as u16);
            dir.extend(rel.encode_utf16());
        }
        let entries = match read_dir(&dir, rules.short_names) {
            Ok(v) => v,
            Err(e) if rel.is_empty() && e.raw_os_error() != Some(2) => return Err(e.to_string()),
            // Gone, or not readable: nothing to prune there (the copy reports real problems).
            Err(_) => continue,
        };
        for e in entries {
            if e.is_link() {
                continue;
            }
            let name = String::from_utf16_lossy(&e.name);
            let child = if rel.is_empty() { name.clone() } else { format!("{rel}\\{name}") };
            let path = target.join(&child);
            if e.attributes & ATTR_DIRECTORY != 0 {
                if rules.skip_dir(&e, &child) || rules.regex_skips_dir(&e).is_some() {
                    let (files, bytes) = tree_size(&path);
                    match remove_tree(&path) {
                        Ok(()) => {
                            out.files += files;
                            out.bytes += bytes;
                        }
                        Err(_) => out.failed += 1,
                    }
                } else {
                    pending.push(child);
                }
            } else if !(rel.is_empty() && name.eq_ignore_ascii_case(crate::manifest::FILE_NAME))
                && (!rules.keep_file(&e) || rules.skip_file_path(&child))
            {
                let mut gone = fs::remove_file(&path);
                if gone.is_err() {
                    // A read-only file can't be deleted until the attribute is cleared.
                    if let Ok(m) = fs::metadata(&path) {
                        let mut p = m.permissions();
                        if p.readonly() {
                            p.set_readonly(false);
                            let _ = fs::set_permissions(&path, p);
                            gone = fs::remove_file(&path);
                        }
                    }
                }
                match gone {
                    Ok(()) => {
                        out.files += 1;
                        out.bytes += e.size;
                    }
                    Err(_) => out.failed += 1,
                }
            }
        }
    }
    Ok(Some(out))
}

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

/// Win32 errors that robocopy / CopyFileEx give on a file that a plain read-and-write copy can
/// still save: 87 (invalid parameter - seen on OneDrive files, where the attributes or the cloud
/// state don't fit the destination) and the cloud-file errors (the read downloads the file first).
pub fn stream_retry(code: Option<u32>) -> bool {
    matches!(code, Some(87 | 362..=366 | 380..=396 | 1920))
}

/// Copies `from` by reading it and writing a new file (no CopyFileEx, so no attributes or cloud
/// state are handed over; reading a cloud placeholder downloads it). Keeps the modified time (the
/// incremental diff depends on it) and the plain attributes. A half-written copy is removed.
pub fn copy_stream(from: &Path, to: &Path) -> io::Result<u64> {
    let mut src = fs::File::open(from)?;
    let meta = src.metadata()?;
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    // A read-only old copy (a resumed folder) can't be overwritten.
    if let Ok(m) = fs::metadata(to) {
        let mut p = m.permissions();
        if p.readonly() {
            p.set_readonly(false);
            let _ = fs::set_permissions(to, p);
        }
    }
    let mut dst = fs::File::create(to)?;
    let copied = io::copy(&mut src, &mut dst).and_then(|n| {
        if let Ok(m) = meta.modified() {
            dst.set_modified(m)?;
        }
        Ok(n)
    });
    drop(dst);
    match copied {
        Ok(n) => {
            use std::os::windows::fs::MetadataExt;
            // Hidden, system, archive: the ones that matter for a copy. Not read-only: a backup copy
            // must stay deletable (retention, the user).
            let attrs = meta.file_attributes() & 0x26;
            crate::tasklist::set_attributes(to, if attrs == 0 { crate::tasklist::NORMAL } else { attrs });
            Ok(n)
        }
        Err(e) => {
            let _ = fs::remove_file(to);
            Err(e)
        }
    }
}

/// `fs::copy`, and when Windows refuses with an error `stream_retry` knows, the copy by streaming.
/// The copy never keeps the read-only attribute (CopyFileEx hands it over), like robocopy's /A-:R.
fn copy_file(from: &Path, to: &Path, debug: &mut Vec<String>) -> io::Result<u64> {
    let n = copy_file_as_is(from, to, debug)?;
    drop_readonly(to);
    Ok(n)
}

fn copy_file_as_is(from: &Path, to: &Path, debug: &mut Vec<String>) -> io::Result<u64> {
    match fs::copy(from, to) {
        Err(e) if stream_retry(e.raw_os_error().map(|c| c as u32)) => {
            let n = copy_stream(from, to)?;
            debug.push(format!("הועתק בקריאה וכתיבה אחרי שגיאה {}: {}", e.raw_os_error().unwrap_or(0), from.display()));
            Ok(n)
        }
        other => other,
    }
}

/// Clears the read-only attribute of one file or folder, keeping its other attributes.
fn drop_readonly(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    let Ok(m) = fs::symlink_metadata(path) else { return false };
    let attrs = m.file_attributes();
    if attrs & ATTR_READONLY == 0 {
        return false;
    }
    // Only the attributes SetFileAttributesW takes.
    let rest = attrs & 0x3126 & !ATTR_READONLY;
    crate::tasklist::set_attributes(path, if rest == 0 { crate::tasklist::NORMAL } else { rest })
}

/// Clears the read-only attribute on `dir` and everything under it (links are not followed).
/// Attributes come from the folder listing, so only read-only items are touched. Returns how many
/// items were changed.
pub fn clear_readonly(dir: &Path) -> u64 {
    let mut changed = drop_readonly(dir) as u64;
    let Ok(root) = verbatim(dir) else { return changed };
    let mut pending = vec![root];
    while let Some(d) = pending.pop() {
        let Ok(entries) = read_dir(&d, false) else { continue };
        for e in entries {
            let mut w = d.clone();
            w.push(b'\\' as u16);
            w.extend_from_slice(&e.name);
            if e.attributes & ATTR_READONLY != 0 {
                use std::os::windows::ffi::OsStringExt;
                let rest = e.attributes & 0x3126 & !ATTR_READONLY;
                let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&w));
                if crate::tasklist::set_attributes(&path, if rest == 0 { crate::tasklist::NORMAL } else { rest }) {
                    changed += 1;
                }
            }
            if e.attributes & ATTR_DIRECTORY != 0 && !e.is_link() {
                pending.push(w);
            }
        }
    }
    changed
}

/// Deletes a folder tree the app wrote (a backup folder or part of one). Files and folders copied
/// from a read-only source item used to keep the attribute, and a read-only item makes the delete
/// fail with "access denied" (always on FAT32 / exFAT drives), so on a failure the attribute is
/// cleared throughout and the delete tried again.
pub fn remove_tree(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(e),
        Err(first) => {
            if clear_readonly(path) == 0 {
                return Err(first);
            }
            fs::remove_dir_all(path)
        }
    }
}

const ATTRIBUTE_NAMES: [(u32, &str); 15] = [
    (0x1, "READONLY"),
    (0x2, "HIDDEN"),
    (0x4, "SYSTEM"),
    (0x20, "ARCHIVE"),
    (0x100, "TEMPORARY"),
    (0x200, "SPARSE"),
    (0x400, "REPARSE_POINT"),
    (0x800, "COMPRESSED"),
    (0x1000, "OFFLINE"),
    (0x4000, "ENCRYPTED"),
    (0x10000, "VIRTUAL"),
    (0x40000, "RECALL_ON_OPEN"),
    (0x80000, "PINNED"),
    (0x100000, "UNPINNED"),
    (0x400000, "RECALL_ON_DATA_ACCESS"),
];

fn filetime_text(t: u64) -> String {
    t.checked_sub(UNIX_EPOCH_TICKS)
        .and_then(|d| chrono::DateTime::<chrono::Utc>::from_timestamp((d / TICKS_PER_SEC) as i64, 0))
        .map_or_else(|| format!("filetime {t}"), |d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
}

/// What a path looks like on disk, on one line, for debugging a copy error.
pub fn describe(path: &Path) -> String {
    let Some(i) = find(path) else {
        return "not found".into();
    };
    let names: Vec<&str> = ATTRIBUTE_NAMES.iter().filter(|(bit, _)| i.attributes & bit != 0).map(|(_, n)| *n).collect();
    let mut s = format!("attributes 0x{:X} [{}]", i.attributes, names.join(" "));
    if i.attributes & ATTR_REPARSE_POINT != 0 {
        s.push_str(&format!(", reparse tag 0x{:08X}", i.reparse_tag));
    }
    s.push_str(&format!(", size {}, modified {}", i.size, filetime_text(i.modified)));
    s
}

/// A source file and the file it should have become, for `FileError.debug`.
fn describe_pair(from: &Path, to: &Path) -> String {
    format!(
        "source: {}; destination: {}; path length {}",
        describe(from),
        describe(to),
        from.to_string_lossy().chars().count()
    )
}

#[link(name = "kernel32")]
extern "system" {
    fn GetVolumeInformationW(
        root: *const u16,
        name: *mut u16,
        name_len: u32,
        serial: *mut u32,
        max_component: *mut u32,
        flags: *mut u32,
        fs_name: *mut u16,
        fs_len: u32,
    ) -> i32;
    fn GetDriveTypeW(root: *const u16) -> u32;
}

/// The drive of `path` as "D: NTFS, fixed disk" (for the debug text of a run).
pub fn volume_info(path: &Path) -> String {
    let s = path.to_string_lossy();
    let root = match s.as_bytes() {
        [d, b':', ..] if d.is_ascii_alphabetic() => format!("{}:\\", *d as char),
        _ => return "network path".into(),
    };
    let wide: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
    let mut fs_name = [0u16; 32];
    let ok = unsafe {
        GetVolumeInformationW(wide.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), fs_name.as_mut_ptr(), 32)
    };
    let kind = match unsafe { GetDriveTypeW(wide.as_ptr()) } {
        2 => "removable",
        3 => "fixed disk",
        4 => "network drive",
        5 => "optical",
        6 => "ram disk",
        _ => "unknown type",
    };
    let fs = if ok != 0 { String::from_utf16_lossy(&until_nul(&fs_name)) } else { "not readable".into() };
    format!("{} {fs}, {kind}", &root[..2])
}

/// Lowercased, without a trailing backslash: for comparing paths as text.
fn norm_path(p: &Path) -> String {
    p.to_string_lossy().trim_end_matches('\\').to_lowercase()
}

/// A mirror's file errors, once robocopy is done: robocopy logs an ERROR line for every failed
/// try, also for a file that then went through on its retry (/R), so the list has more entries than
/// the summary's FAILED count. Each entry is checked against the destination: a file that is there
/// with the source's size and time was copied after all and is dropped. The others get a second
/// chance by streaming (see `stream_retry`); whatever still fails keeps its error, with debug
/// info. Returns how many files were saved by streaming: robocopy counted those as failed, the
/// ones that went through on its own retry it never did.
pub fn settle_errors(
    source: &Path,
    target: &Path,
    stats: &mut CopyStats,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(EngineEvent),
) -> u64 {
    let prefix = format!("{}\\", norm_path(source));
    let mut streamed = 0;
    let mut bytes = 0;
    let errors = std::mem::take(&mut stats.errors);
    for mut e in errors {
        let from = Path::new(&e.path);
        let rel = e
            .path
            .to_lowercase()
            .starts_with(&prefix)
            .then(|| e.path.get(prefix.len()..))
            .flatten()
            .map(str::to_string);
        let (Some(rel), Some(src)) = (rel, find(from).filter(|i| !i.is_dir())) else {
            // A folder, a file that's gone, or not under the source: nothing to settle.
            e.debug = describe(from);
            stats.errors.push(e);
            continue;
        };
        let to = target.join(&rel);
        let there = find(&to).filter(|d| !d.is_dir() && d.size == src.size && d.modified.abs_diff(src.modified) <= 2 * TICKS_PER_SEC);
        if there.is_some() {
            stats.debug.push(format!("error {} on a try, but the file is in the destination: {}", e.code.unwrap_or(0), e.path));
            continue;
        }
        if stream_retry(e.code) && !cancel.load(Ordering::SeqCst) {
            on_event(EngineEvent::File { path: e.path.clone(), size: src.size });
            match copy_stream(from, &to) {
                Ok(n) => {
                    streamed += 1;
                    bytes += n;
                    stats.debug.push(format!("copied by reading and writing after error {}: {}", e.code.unwrap_or(0), e.path));
                    continue;
                }
                Err(err) => {
                    e.debug = format!("second try by reading and writing failed: {err}; {}", describe_pair(from, &to));
                }
            }
        }
        if e.debug.is_empty() {
            e.debug = describe_pair(from, &to);
        }
        stats.errors.push(e);
    }
    stats.files_copied += streamed;
    stats.bytes_copied += bytes;
    stats.files_failed = stats.files_failed.saturating_sub(streamed);
    streamed
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
        exact: true,
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
        let mut copy = || -> std::io::Result<u64> {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            copy_file(&from, &to, &mut stats.debug)
        };
        // One retry: files are often locked only for a moment. A file that's gone stays gone, and a
        // full disk doesn't free up by waiting.
        let result = copy().or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound || is_disk_full(e.raw_os_error().map(|c| c as u32)) {
                return Err(e);
            }
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
                log.push(format!("שגיאה\t{} - {e}", from.display()));
                if stats.errors.len() < MAX_ERRORS {
                    stats.errors.push(FileError {
                        path: from.to_string_lossy().to_string(),
                        code: e.raw_os_error().map(|c| c as u32),
                        message: e.to_string(),
                        debug: describe_pair(&from, &to),
                    });
                }
                // Every further file would fail too: stop here. The cut-off file is removed so the
                // resume doesn't take a truncated copy for a finished one.
                if is_disk_full(e.raw_os_error().map(|c| c as u32)) {
                    let _ = fs::remove_file(&to);
                    log.push(DISK_FULL_MESSAGE.into());
                    write_log(log_file, &log);
                    return CopyResult {
                        outcome: Outcome::Failed,
                        message: DISK_FULL_MESSAGE.into(),
                        stats,
                    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::robocopy::Robocopy;
    use crate::engine::{CopyEngine, CopyJob};
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    struct Tree(PathBuf);

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::process::Command::new("cmd").args(["/c", "rmdir"]).arg(self.0.join("src").join("junc")).output();
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn file(p: &Path, hours_old: u64) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "x").unwrap();
        let t = SystemTime::now() - Duration::from_secs(hours_old * 3600);
        fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
    }

    fn names(files: Vec<ListedFile>) -> BTreeSet<String> {
        files.into_iter().map(|f| f.rel.to_lowercase()).collect()
    }

    /// The incremental listing must see exactly what a robocopy full backup copies.
    #[test]
    fn listing_matches_robocopy() {
        let tree = Tree(std::env::temp_dir().join(format!("backuper-scan-{}", uuid::Uuid::new_v4())));
        let src = tree.0.join("src");
        for n in ["noext", "page.html", "page.htm", "a.b.c", "x.txt", "longfilename.jpeg", "שלום.TXT", ".dotfile", r"sub\deep\d.txt", r"sub\s.htm", r"$RECYCLE.BIN\r.txt"] {
            file(&src.join(n), 0);
        }
        file(&src.join("old47.txt"), 47);
        file(&src.join("old49.txt"), 49);
        file(&src.join(r"hid\inside.txt"), 0);
        file(&src.join("hidden.txt"), 0);
        file(&src.join("sys.txt"), 0);
        fs::write(src.join("big.bin"), vec![0u8; 5000]).unwrap();
        let attrib = |flag: &str, p: &str| assert!(std::process::Command::new("attrib").arg(flag).arg(src.join(p)).status().unwrap().success());
        attrib("+H", "hid");
        attrib("+H", "hidden.txt");
        attrib("+S", "sys.txt");
        file(&tree.0.join(r"outside\linked.txt"), 0);
        let mk = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(src.join("junc")).arg(tree.0.join("outside")).output().unwrap();
        assert!(mk.status.success());

        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let cases = vec![
            Filters::default(),
            Filters { include_files: v(&["*.htm"]), ..Default::default() },
            Filters { include_files: v(&["noext.*", "?????"]), ..Default::default() },
            Filters { include_files: v(&["*.*"]), ..Default::default() },
            Filters { include_files: v(&["a*.*", "*.c", "*."]), ..Default::default() },
            Filters { include_files: v(&["*.t?t", ".*"]), ..Default::default() },
            Filters { exclude_files: v(&["*.htm", "*.b", "*x*"]), ..Default::default() },
            Filters { exclude_dirs: v(&["de*"]), ..Default::default() },
            Filters { exclude_files: v(&[&src.join("sub").join("s.htm").display().to_string(), "x.txt"]), ..Default::default() },
            Filters { exclude_dirs: v(&[&format!("{}\\", src.join("sub").join("deep").display()), "hid"]), ..Default::default() },
            Filters { exclude_dirs: v(&[r"sub\deep"]), ..Default::default() },
            Filters { exclude_hidden: true, exclude_system: true, ..Default::default() },
            Filters { max_size: Some(4000), max_age_days: Some(2), ..Default::default() },
        ];
        let cancel = AtomicBool::new(false);
        for (i, f) in cases.into_iter().enumerate() {
            let target = tree.0.join(format!("full{i}"));
            let job = CopyJob {
                source: src.clone(),
                target: target.clone(),
                copy_empty_dirs: false,
                filters: f.for_source(&src),
                log_file: tree.0.join(format!("{i}.log")),
                totals: crate::engine::TotalsSource::Scan,
                late_totals: None,
            };
            Robocopy.mirror(&job, &cancel, &mut |_| {});
            let copied = names(list_files(&target, &Filters::default(), &cancel).unwrap().unwrap().files);
            let listed = names(list_files(&src, &f, &cancel).unwrap().unwrap().files);
            assert_eq!(listed, copied, "case {i}: {f:?}");
        }
    }

    /// robocopy rejects `*\a\b` in /XD and /XF ("Invalid Parameter", exit 16, nothing copied): `resolve_regex`
    /// swaps such rules for the full paths they match, and the mirror skips exactly those.
    #[test]
    fn wildcard_path_rules_reach_robocopy_as_full_paths() {
        let tree = Tree(std::env::temp_dir().join(format!("backuper-wild-{}", uuid::Uuid::new_v4())));
        let src = tree.0.join("src");
        for n in [r"p1\target\t.txt", r"p2\x\target\t.txt", r"p2\keep.txt", r"p2\x\drop.log", r"top.txt"] {
            file(&src.join(n), 0);
        }
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let f = Filters { exclude_dirs: v(&[r"*\target"]), exclude_files: v(&[r"*\x\drop.log"]), ..Default::default() };
        let cancel = AtomicBool::new(false);
        let resolved = resolve_regex(&src, &f, &cancel).unwrap().unwrap();
        assert!(!resolved.exclude_dirs.iter().chain(&resolved.exclude_files).any(|s| is_wild_path(s)));
        let target = tree.0.join("full");
        let job = CopyJob {
            source: src.clone(),
            target: target.clone(),
            copy_empty_dirs: false,
            filters: resolved,
            log_file: tree.0.join("wild.log"),
            totals: crate::engine::TotalsSource::Scan,
            late_totals: None,
        };
        let r = Robocopy.mirror(&job, &cancel, &mut |_| {});
        assert!(r.stats.exit_code.map_or(true, |c| c < 8), "{}", r.message);
        let copied = names(list_files(&target, &Filters::default(), &cancel).unwrap().unwrap().files);
        let want: BTreeSet<String> = [r"p2\keep.txt", "top.txt"].iter().map(|s| s.to_string()).collect();
        assert_eq!(copied, want);
        assert_eq!(copied, names(list_files(&src, &f, &cancel).unwrap().unwrap().files));
    }

    #[test]
    fn listing_gives_size_and_modified_time() {
        let tree = Tree(std::env::temp_dir().join(format!("backuper-scan-{}", uuid::Uuid::new_v4())));
        let p = tree.0.join("src").join("a.txt");
        file(&p, 5);
        let l = list_files(&tree.0.join("src"), &Filters::default(), &AtomicBool::new(false)).unwrap().unwrap();
        let m = fs::metadata(&p).unwrap().modified().unwrap().duration_since(UNIX_EPOCH).unwrap();
        assert_eq!(l.files.len(), 1);
        assert_eq!(l.files[0].size, 1);
        assert_eq!(l.files[0].modified, m.as_nanos() as u64 / 100 + UNIX_EPOCH_TICKS);
        assert!(list_files(&tree.0.join("missing"), &Filters::default(), &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn stream_copy_keeps_content_and_modified_time() {
        let tree = Tree(std::env::temp_dir().join(format!("backuper-stream-{}", uuid::Uuid::new_v4())));
        let (from, to) = (tree.0.join("src").join("a.txt"), tree.0.join("dst").join("sub").join("a.txt"));
        file(&from, 30);
        assert_eq!(copy_stream(&from, &to).unwrap(), 1);
        assert_eq!(fs::read(&to).unwrap(), b"x");
        assert_eq!(fs::metadata(&to).unwrap().modified().unwrap(), fs::metadata(&from).unwrap().modified().unwrap());
        // Over an old read-only copy.
        let mut perms = fs::metadata(&to).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&to, perms).unwrap();
        assert!(copy_stream(&from, &to).is_ok());
    }

    #[test]
    fn errors_of_files_that_made_it_are_dropped_and_87_is_saved_by_streaming() {
        let tree = Tree(std::env::temp_dir().join(format!("backuper-settle-{}", uuid::Uuid::new_v4())));
        let (src, dst) = (tree.0.join("src"), tree.0.join("dst"));
        for n in ["retried.txt", "invalid.txt", "locked.txt"] {
            file(&src.join(n), 5);
        }
        // robocopy's retry got this one through.
        copy_stream(&src.join("retried.txt"), &dst.join("retried.txt")).unwrap();
        let error = |name: &str, code: u32| FileError {
            path: src.join(name).to_string_lossy().to_string(),
            code: Some(code),
            ..Default::default()
        };
        let mut stats = CopyStats {
            files_failed: 2,
            errors: vec![error("retried.txt", 87), error("invalid.txt", 87), error("locked.txt", 32)],
            ..Default::default()
        };
        let streamed = settle_errors(&src, &dst, &mut stats, &AtomicBool::new(false), &mut |_| {});
        assert_eq!(streamed, 1);
        assert_eq!((stats.files_copied, stats.files_failed), (1, 1));
        assert!(dst.join("invalid.txt").exists());
        assert_eq!(stats.errors.len(), 1);
        assert!(stats.errors[0].path.ends_with("locked.txt"));
        assert!(stats.errors[0].debug.contains("attributes"), "{}", stats.errors[0].debug);
    }

    #[test]
    fn read_only_items_are_not_copied_as_such_and_still_get_deleted() {
        let tree = Tree(std::env::temp_dir().join(format!("backuper-readonly-{}", uuid::Uuid::new_v4())));
        let readonly = |p: &Path| {
            let mut perms = fs::metadata(p).unwrap().permissions();
            perms.set_readonly(true);
            fs::set_permissions(p, perms).unwrap();
        };
        let src = tree.0.join("src");
        file(&src.join("a.txt"), 5);
        readonly(&src.join("a.txt"));
        let dst = tree.0.join("dst");
        fs::create_dir_all(&dst).unwrap();
        copy_file(&src.join("a.txt"), &dst.join("a.txt"), &mut Vec::new()).unwrap();
        assert!(!fs::metadata(dst.join("a.txt")).unwrap().permissions().readonly());
        copy_stream(&src.join("a.txt"), &dst.join("b.txt")).unwrap();
        assert!(!fs::metadata(dst.join("b.txt")).unwrap().permissions().readonly());

        // A folder written by an older version: read-only files and a read-only subfolder.
        let old = tree.0.join("old");
        file(&old.join("sub").join("c.txt"), 5);
        file(&old.join("d.txt"), 5);
        readonly(&old.join("sub").join("c.txt"));
        readonly(&old.join("d.txt"));
        readonly(&old.join("sub"));
        remove_tree(&old).unwrap();
        assert!(!old.exists());
        // Clear src's attribute so the temp tree can go.
        clear_readonly(&tree.0);
    }
}
