//! The incremental path, without robocopy: `list_files` lists a folder tree (applying the same
//! filters robocopy does) and `copy_files` copies an explicit list of files (the changes).
//! std::fs::copy uses CopyFileExW on Windows, so attributes and the modified time are preserved.

use super::{CopyResult, CopyStats, EngineEvent, Filters, ListedFile, Outcome, ALWAYS_EXCLUDED_DIRS};
use crate::model::FileError;
use regex::{Regex, RegexBuilder};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const MAX_ERRORS: usize = 50;

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
    include_regex: Vec<Regex>,
    exclude_regex: Vec<Regex>,
    exclude_dir_names: Vec<Pattern>,
    /// /XD entries with a backslash: full paths (robocopy never matches them as relative paths).
    exclude_dir_paths: Vec<Pattern>,
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

impl Rules {
    fn new(source: &Path, f: &Filters) -> Self {
        let patterns = |v: &[String]| -> Vec<Pattern> {
            v.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).map(Pattern::new).collect()
        };
        let (dir_paths, dir_names): (Vec<String>, Vec<String>) = f
            .exclude_dirs
            .iter()
            .map(|d| d.trim().trim_end_matches('\\').to_string())
            .chain(ALWAYS_EXCLUDED_DIRS.iter().map(|d| d.to_string()))
            .partition(|d| d.contains('\\'));
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() * TICKS_PER_SEC + UNIX_EPOCH_TICKS;
        Rules {
            include: patterns(&f.include_files),
            exclude_files: patterns(&f.exclude_files),
            // Validated when saved; a rule that still fails to compile skips nothing.
            include_regex: f.include_regex.iter().filter_map(|p| compile_regex(p).ok()).collect(),
            exclude_regex: f.exclude_regex.iter().filter_map(|p| compile_regex(p).ok()).collect(),
            exclude_dir_names: patterns(&dir_names),
            exclude_dir_paths: patterns(&dir_paths),
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

    fn regex_ok(&self, e: &Entry) -> bool {
        if self.include_regex.is_empty() && self.exclude_regex.is_empty() {
            return true;
        }
        let name = String::from_utf16_lossy(&e.name);
        self.include_regex.iter().all(|r| r.is_match(&name)) && !self.exclude_regex.iter().any(|r| r.is_match(&name))
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

/// Longest list of names handed to robocopy on its command line (the limit is 32,767 characters).
const MAX_NAMES_LEN: usize = 28_000;

const TOO_MANY: &str = "יותר מדי קבצים שונים תואמים לביטוי הרגולרי, ואי אפשר להעביר אותם לרובוקופי. צמצמו את הביטוי, או הוסיפו כלל שמצמצם את הקבצים (למשל סיומות).";

/// robocopy has no regular expressions, so list the source once and turn the regex rules into
/// plain file names it understands: either "skip these names" or "copy only these names",
/// whichever list is shorter. Ok(None) = cancelled.
pub fn resolve_regex(root: &Path, filters: &Filters, cancel: &AtomicBool) -> Result<Option<Filters>, String> {
    if filters.include_regex.is_empty() && filters.exclude_regex.is_empty() {
        return Ok(Some(filters.clone()));
    }
    let mut out = filters.clone();
    out.include_regex.clear();
    out.exclude_regex.clear();
    let Some(listing) = list_files(root, &out, cancel)? else { return Ok(None) };
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
        if len(&dropped) > MAX_NAMES_LEN {
            return Err(TOO_MANY.into());
        }
        out.exclude_files.extend(dropped);
    } else {
        if len(&kept) > MAX_NAMES_LEN {
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
                if !rules.skip_dir(&e, &child) {
                    pending.push(child);
                }
            } else if rules.keep_file(&e) {
                out.files.push(ListedFile {
                    rel: child,
                    size: e.size,
                    modified: e.modified,
                });
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
        // One retry: files are often locked only for a moment. A file that's gone stays gone.
        let result = copy().or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
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
                    });
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
                filters: f.clone(),
                log_file: tree.0.join(format!("{i}.log")),
            };
            Robocopy.mirror(&job, &cancel, &mut |_| {});
            let copied = names(list_files(&target, &Filters::default(), &cancel).unwrap().unwrap().files);
            let listed = names(list_files(&src, &f, &cancel).unwrap().unwrap().files);
            assert_eq!(listed, copied, "case {i}: {f:?}");
        }
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
}
