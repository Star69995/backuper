//! Import of Cobian Backup / Cobian Reflector task lists (`.lst`).
//!
//! The file (UTF-16 LE) is a flat list of sections `<§- id -§> ... <§§- id -§§>` holding
//! `Key=Value` lines. A value `{ *§* id *§* }` points at another section of the same file
//! (sources, destination, schedule, filters...). The first section lists the tasks.

use crate::model::{BackupMode, FilterRule, Schedule, Source, Task};
use std::collections::HashMap;

/// Days are 0 = Sunday .. 6 = Saturday, as in the app.
const DAY_NAMES: [&str; 7] = ["ראשון", "שני", "שלישי", "רביעי", "חמישי", "שישי", "שבת"];

/// Keeps a path/mask left-to-right inside Hebrew text.
fn ltr(s: &str) -> String {
    format!("\u{2066}{s}\u{2069}")
}

/// Cobian keeps unlimited copies with 0; the app needs a number.
const UNLIMITED_KEEP: u32 = 30;

pub struct Imported {
    pub task: Task,
    /// Settings that couldn't be carried over exactly.
    pub warnings: Vec<String>,
}

struct Doc {
    sections: HashMap<String, Vec<(String, String)>>,
    root: Option<String>,
}

#[derive(Clone, Copy)]
struct Sec<'a> {
    doc: &'a Doc,
    kv: &'a [(String, String)],
}

fn reference(v: &str) -> Option<&str> {
    let inner = v.trim().strip_prefix('{')?.strip_suffix('}')?.trim();
    Some(inner.strip_prefix("*§*")?.strip_suffix("*§*")?.trim())
}

impl<'a> Sec<'a> {
    fn all(self, key: &'a str) -> impl Iterator<Item = &'a str> {
        self.kv
            .iter()
            .filter(move |(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
    fn get(self, key: &'a str) -> Option<&'a str> {
        self.all(key).next()
    }
    fn flag(self, key: &'a str) -> Option<bool> {
        self.get(key).map(|v| v.trim().eq_ignore_ascii_case("true"))
    }
    fn num(self, key: &'a str) -> Option<i64> {
        self.get(key)?.trim().parse().ok()
    }
    fn children(self, key: &'a str) -> Vec<Sec<'a>> {
        self.all(key).filter_map(|v| self.doc.section(reference(v)?)).collect()
    }
    fn child(self, key: &'a str) -> Option<Sec<'a>> {
        self.children(key).into_iter().next()
    }
}

impl Doc {
    fn parse(text: &str) -> Doc {
        let mut sections = HashMap::new();
        let mut root = None;
        let mut current: Option<(String, Vec<(String, String)>)> = None;
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if let Some(id) = line.strip_prefix("<§§-").and_then(|s| s.strip_suffix("-§§>")) {
                if let Some((cid, kv)) = current.take() {
                    if cid == id.trim() {
                        sections.insert(cid, kv);
                    }
                }
            } else if let Some(id) = line.strip_prefix("<§-").and_then(|s| s.strip_suffix("-§>")) {
                let id = id.trim().to_string();
                root.get_or_insert_with(|| id.clone());
                current = Some((id, Vec::new()));
            } else if let Some((_, kv)) = &mut current {
                // "§Key:Value" lines are format metadata.
                if !line.starts_with('§') {
                    if let Some((k, v)) = line.split_once('=') {
                        kv.push((k.trim().to_string(), v.to_string()));
                    }
                }
            }
        }
        Doc { sections, root }
    }

    fn section(&self, id: &str) -> Option<Sec<'_>> {
        self.sections.get(id).map(|kv| Sec { doc: self, kv })
    }
}

/// The file is UTF-16 LE with a BOM; UTF-8 is accepted too.
fn decode(bytes: &[u8]) -> Result<String, String> {
    let utf16 = |b: &[u8]| {
        let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Ok(utf16(rest));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).map_err(|e| e.to_string());
    }
    match String::from_utf8(bytes.to_vec()) {
        Ok(s) => Ok(s),
        Err(_) => Ok(utf16(bytes)),
    }
}

pub fn parse_file(bytes: &[u8]) -> Result<Vec<Imported>, String> {
    let text = decode(bytes)?;
    let doc = Doc::parse(&text);
    let root = doc
        .root
        .as_deref()
        .and_then(|id| doc.section(id))
        .ok_or("הקובץ אינו רשימת משימות של Cobian")?;
    let tasks = root.children("BackupTask");
    if tasks.is_empty() {
        return Err("לא נמצאו משימות בקובץ".into());
    }
    Ok(tasks.into_iter().map(convert).collect())
}

/// "2026-10-01 23:35:18:671" -> ("2026-10-01", "23:35")
fn date_time(v: &str) -> Option<(String, String)> {
    let (date, time) = v.trim().split_once(' ')?;
    let mut parts = time.split(':');
    let h: u32 = parts.next()?.trim().parse().ok()?;
    let m: u32 = parts.next()?.trim().parse().ok()?;
    (h < 24 && m < 60).then(|| (date.to_string(), format!("{h:02}:{m:02}")))
}

fn convert_schedule(s: Option<Sec>, w: &mut Vec<String>) -> Schedule {
    let Some(s) = s else {
        w.push("לא נמצא תזמון - המשימה תיובא כהפעלה ידנית".into());
        return Schedule::Manual;
    };
    let (date, time) = s
        .get("SchDateAndTime")
        .and_then(date_time)
        .unwrap_or_else(|| (String::new(), "03:00".into()));
    // Cobian: 0 once, 1 daily, 2 weekly, 3 monthly, 4 yearly, 5 timer, 6 manual.
    match s.num("SchSchedule") {
        Some(0) if !date.is_empty() => Schedule::Once {
            at: format!("{date}T{time}"),
        },
        Some(1) => Schedule::Daily { time },
        Some(2) => {
            let mut days: Vec<u8> = s
                .all("SchDaysOfWeek")
                .filter_map(|d| d.trim().parse().ok())
                .filter(|d| *d < 7)
                .collect();
            days.sort();
            days.dedup();
            if days.is_empty() {
                w.push("לא נבחרו ימים בתזמון השבועי - נקבע יום ראשון".into());
                days.push(0);
            }
            Schedule::Weekly { days, time }
        }
        Some(3) => {
            let day = s.num("SchDayOfMonth").filter(|d| (1..=31).contains(d)).unwrap_or_else(|| {
                w.push("לא נמצא יום בחודש בתזמון החודשי - נקבע היום הראשון בחודש".into());
                1
            });
            Schedule::Monthly { day: day as u8, time }
        }
        Some(5) => match s.num("SchTimer").filter(|m| *m > 0) {
            Some(minutes) => Schedule::Interval { minutes: minutes as u32 },
            None => {
                w.push("לא נמצא מרווח זמן בתזמון - המשימה תיובא כהפעלה ידנית".into());
                Schedule::Manual
            }
        },
        Some(6) => Schedule::Manual,
        Some(4) => {
            w.push("תזמון שנתי לא נתמך - המשימה תיובא כהפעלה ידנית".into());
            Schedule::Manual
        }
        _ => {
            w.push("סוג התזמון לא נתמך - המשימה תיובא כהפעלה ידנית".into());
            Schedule::Manual
        }
    }
}

fn schedule_time(s: &Schedule) -> &str {
    match s {
        Schedule::Daily { time } | Schedule::Weekly { time, .. } | Schedule::Monthly { time, .. } => time,
        _ => "03:00",
    }
}

/// Cobian filter entries: a wildcard mask, a file/folder path or a size.
fn convert_filter(f: Sec, include: bool, w: &mut Vec<String>) -> Option<FilterRule> {
    let mask = f.get("FOMask").unwrap_or("").trim();
    if include {
        if !mask.is_empty() {
            w.push(format!(
                "ב-Cobian גובו רק קבצים שתואמים ל-\"{}\" - כאן אין סינון \"רק\", ולכן יגובו כל הקבצים. אפשר להוסיף כללי החרגה במקום",
                ltr(mask)
            ));
        }
        return None;
    }
    if mask.is_empty() {
        let size = f.num("FOSize").unwrap_or(0);
        if size > 0 {
            let mb = (size as u64).div_ceil(1024 * 1024).max(1);
            return Some(FilterRule::LargerThan { mb });
        }
        w.push("כלל החרגה של Cobian שלא ניתן להמיר - לא יובא".into());
        return None;
    }
    if !mask.contains('\\') {
        return Some(FilterRule::Pattern { value: mask.to_string() });
    }
    let last = mask.trim_end_matches('\\').rsplit('\\').next().unwrap_or(mask);
    if std::path::Path::new(mask).is_file() || (last.contains('.') && !std::path::Path::new(mask).is_dir()) {
        w.push(format!("החרגה של קובץ לפי נתיב מלא ({}) הומרה להחרגה לפי שם הקובץ \"{}\" בכל התיקיות", ltr(mask), ltr(last)));
        return Some(FilterRule::Pattern { value: last.to_string() });
    }
    Some(FilterRule::Folder { value: mask.to_string() })
}

fn convert(t: Sec) -> Imported {
    let mut w = Vec::new();
    let mut task = Task {
        id: t.get("TaskId").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_default(),
        name: t.get("TaskName").unwrap_or("").trim().to_string(),
        enabled: t.flag("TaskEnabled").unwrap_or(true),
        // Cobian copies empty folders unless told to ignore them.
        copy_empty_dirs: !t.flag("TaskIgnoreEmptyDirectories").unwrap_or(false),
        ..Task::default()
    };
    if task.name.is_empty() {
        task.name = "משימה מ-Cobian".into();
    }

    // SDKind 1 = local/network folder; 0 = single file; others are FTP/SFTP.
    for s in t.children("TaskSource") {
        let path = s.get("SDPath").unwrap_or("").trim();
        if path.is_empty() {
            continue;
        }
        match s.num("SDKind") {
            Some(1) => task.sources.push(Source {
                path: path.to_string(),
                folder_name: String::new(),
            }),
            Some(0) => w.push(format!("מקור שהוא קובץ בודד לא נתמך ולא יובא: {}", ltr(path))),
            _ => w.push(format!("מקור שאינו תיקייה מקומית (FTP וכו') לא נתמך ולא יובא: {}", ltr(path))),
        }
    }
    let dests = t.children("TaskDestination");
    if dests.len() > 1 {
        w.push("למשימה כמה יעדים - יובא רק הראשון".into());
    }
    if let Some(d) = dests.first() {
        let path = d.get("SDPath").unwrap_or("").trim();
        if d.num("SDKind") == Some(1) {
            task.destination = path.to_string();
        } else if !path.is_empty() {
            w.push(format!("יעד שאינו תיקייה מקומית לא נתמך: {}", ltr(path)));
        }
    }

    task.schedule = convert_schedule(t.child("TaskSchedule"), &mut w);

    // Cobian: 0 full, 1 incremental, 2 differential, 3 dummy.
    task.mode = match t.num("TaskBackupType") {
        Some(0) => BackupMode::Full,
        Some(2) => {
            w.push("גיבוי דיפרנציאלי לא נתמך - יובא כאינקרמנטלי".into());
            BackupMode::Incremental
        }
        Some(3) => {
            w.push("משימת דמה (ללא העתקה) - תיובא כגיבוי מלא".into());
            BackupMode::Full
        }
        _ => BackupMode::Incremental,
    };
    if task.mode == BackupMode::Incremental {
        if t.flag("TaskFixedFullBackup").unwrap_or(false) {
            let day = t.num("TaskFixedFullBackupDay").filter(|d| (0..7).contains(d)).unwrap_or(0) as u8;
            task.full_schedule = Some(Schedule::Weekly {
                days: vec![day],
                time: schedule_time(&task.schedule).to_string(),
            });
            if let Schedule::Weekly { days, .. } = &task.schedule {
                if !days.contains(&day) {
                    w.push(format!(
                        "הגיבוי המלא קבוע ליום {} שאינו מימי הגיבוי - הוא ירוץ ביום הזה כריצה נפרדת",
                        DAY_NAMES[day as usize]
                    ));
                }
            }
        } else if let Some(n) = t.num("TaskForceFullBackupCount").filter(|n| *n > 0) {
            w.push(format!(
                "\"גיבוי מלא כל {n} גיבויים\" לא נתמך - הגיבוי המלא ירוץ רק כשאין עדיין גיבוי מלא. אפשר לקבוע לו תזמון במצב משולב"
            ));
        }
    }

    let keep = t.num("TaskFullCopiesToKeep").unwrap_or(1);
    task.keep_count = if keep <= 0 {
        w.push(format!("ב-Cobian נשמרו גיבויים ללא הגבלה - נקבעו {UNLIMITED_KEEP}"));
        UNLIMITED_KEEP
    } else {
        keep as u32
    };

    if !t.flag("TaskIncludeSubdirectories").unwrap_or(true) {
        w.push("ב-Cobian לא גובו תתי-תיקיות - כאן יגובו גם תתי-התיקיות".into());
    }
    for f in t.children("TaskIncludeFilters") {
        convert_filter(f, true, &mut w);
    }
    task.filters = t
        .children("TaskExcludeFilters")
        .into_iter()
        .filter_map(|f| convert_filter(f, false, &mut w))
        .collect();

    Imported { task, warnings: w }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(id: &str, lines: &[&str]) -> String {
        format!(
            "<§- {id} -§>\r\n§DuplicatedKeys:True\r\n§PairSeparator:=\r\n{}\r\n<§§- {id} -§§>\r\n\r\n",
            lines.join("\r\n")
        )
    }
    fn r(id: &str) -> String {
        format!("{{ *§* {id} *§* }}")
    }

    fn sample() -> Vec<u8> {
        let mut s = String::new();
        s += &section("root", &[&format!("BackupTask={}", r("t1")), &format!("BackupTask={}", r("t2"))]);
        s += &section(
            "t1",
            &[
                "TaskId=7d7c8a4d-1704-4eed-bbc7-368049912640",
                "TaskName=Docs to K Daily א-ב",
                "TaskEnabled=True",
                "TaskBackupType=1",
                &format!("TaskSource={}", r("s1")),
                &format!("TaskSource={}", r("s2")),
                &format!("TaskDestination={}", r("d1")),
                &format!("TaskSchedule={}", r("sch1")),
                "TaskFullCopiesToKeep=1",
                "TaskForceFullBackupCount=4",
                "TaskFixedFullBackup=True",
                "TaskFixedFullBackupDay=0",
                "TaskIgnoreEmptyDirectories=False",
                &format!("TaskExcludeFilters={}", r("f1")),
            ],
        );
        s += &section("s1", &["SDKind=1", "SDPath=C:\\Users\\Inbai\\Documents"]);
        s += &section("s2", &["SDKind=1", "SDPath=D:\\מסמכים שלי"]);
        s += &section("d1", &["SDKind=1", "SDPath=K:\\cobian\\Docs"]);
        s += &section(
            "sch1",
            &["SchSchedule=1", "SchDateAndTime=2026-10-01 23:35:18:671", "SchDaysOfWeek=0", "SchDaysOfWeek=3"],
        );
        s += &section("f1", &["FOFilterKind=3", "FOMask=*.tmp"]);
        s += &section(
            "t2",
            &[
                "TaskId=f7e7a81f",
                "TaskName=LR catalogs",
                "TaskEnabled=False",
                "TaskBackupType=0",
                &format!("TaskSource={}", r("s3")),
                &format!("TaskDestination={}", r("d2")),
                &format!("TaskSchedule={}", r("sch2")),
                "TaskFullCopiesToKeep=5",
                &format!("TaskIncludeFilters={}", r("f2")),
            ],
        );
        s += &section("s3", &["SDKind=1", "SDPath=C:\\LR catalogs"]);
        s += &section("d2", &["SDKind=1", "SDPath=K:\\Cobian LR Bkp"]);
        s += &section(
            "sch2",
            &["SchSchedule=2", "SchDateAndTime=2026-10-01 00:10:31:445", "SchDaysOfWeek=5", "SchDaysOfWeek=1"],
        );
        s += &section("f2", &["FOFilterKind=3", "FOMask=*.lrcat"]);
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(s.encode_utf16().flat_map(|u| u.to_le_bytes()));
        bytes
    }

    #[test]
    fn imports_tasks() {
        let list = parse_file(&sample()).unwrap();
        assert_eq!(list.len(), 2);

        let a = &list[0].task;
        assert_eq!(a.id, "7d7c8a4d-1704-4eed-bbc7-368049912640");
        assert_eq!(a.name, "Docs to K Daily א-ב");
        assert!(a.enabled && a.copy_empty_dirs);
        assert_eq!(a.sources.iter().map(|s| s.path.as_str()).collect::<Vec<_>>(), ["C:\\Users\\Inbai\\Documents", "D:\\מסמכים שלי"]);
        assert_eq!(a.destination, "K:\\cobian\\Docs");
        assert_eq!(a.mode, BackupMode::Incremental);
        assert_eq!(a.schedule, Schedule::Daily { time: "23:35".into() });
        assert_eq!(
            a.full_schedule,
            Some(Schedule::Weekly {
                days: vec![0],
                time: "23:35".into()
            })
        );
        assert_eq!(a.keep_count, 1);
        assert_eq!(a.filters, [FilterRule::Pattern { value: "*.tmp".into() }]);
        assert!(list[0].warnings.is_empty(), "{:?}", list[0].warnings);

        let b = &list[1].task;
        assert!(!b.enabled);
        assert_eq!(b.mode, BackupMode::Full);
        assert_eq!(b.full_schedule, None);
        assert_eq!(
            b.schedule,
            Schedule::Weekly {
                days: vec![1, 5],
                time: "00:10".into()
            }
        );
        assert_eq!(b.keep_count, 5);
        assert_eq!(list[1].warnings.len(), 1);
        assert!(list[1].warnings[0].contains("*.lrcat"));
    }

    #[test]
    fn rejects_other_files() {
        assert!(parse_file(b"hello").is_err());
    }
}
