//! Import of a plain JSON backup config: `{"copy_sources": {"src_0": [paths], ...}, "copy_destinations": [paths]}`.
//!
//! The file has no schedule, filters or retention, so the tasks are manual with the app's defaults.
//! `Split` decides how many: one task with every folder (the groups merged: a folder listed in
//! several groups is one source), one task per group, or one task per folder. All of them go to
//! the file's first destination, so backup folder names are made unique across the tasks.

use serde::Deserialize;

use crate::backup::default_folder_name;
use crate::cobian::Imported;
use crate::model::{Schedule, Source, Task};

/// Id of the single merged task. Fixed, so importing the same file again updates the task instead
/// of duplicating it; the split tasks get ids derived from their group key / folder path for the same reason.
pub const TASK_ID: &str = "simple-config";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Split {
    /// One task with every folder.
    #[default]
    One,
    /// One task per `src_N` group.
    Group,
    /// One task per folder.
    Source,
}

pub struct Parsed {
    pub tasks: Vec<Imported>,
    /// Non-empty groups and distinct folders in the file (what the split options can offer).
    pub groups: usize,
    pub sources: usize,
}

/// Every task of a plain config import has an id starting with this (to find an earlier import of
/// the file in another split).
pub fn is_simple_id(id: &str) -> bool {
    id == TASK_ID || id.starts_with("simple-config-")
}

fn ltr(s: &str) -> String {
    format!("\u{2066}{s}\u{2069}")
}

fn norm(path: &str) -> String {
    path.trim_end_matches('\\').to_lowercase()
}

/// FNV-1a: a stable id for a folder path (std's hasher isn't stable across versions).
fn path_id(path: &str) -> String {
    let h = norm(path)
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    format!("{TASK_ID}-{h:016x}")
}

/// "A, B, C ועוד 2".
fn names_of(sources: &[Source]) -> String {
    let names: Vec<String> = sources.iter().take(3).map(|s| default_folder_name(&s.path)).collect();
    let rest = sources.len().saturating_sub(3);
    let mut s = names.join(", ");
    if rest > 0 {
        s += &format!(" ועוד {rest}");
    }
    s
}

/// Ok(None) = not this format (try others).
pub fn parse(bytes: &[u8], split: Split) -> Result<Option<Parsed>, String> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Ok(None);
    };
    let Some(groups) = value.get("copy_sources").and_then(|v| v.as_object()) else {
        return Ok(None);
    };
    let paths = |v: &serde_json::Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p.as_str())
            .map(|p| p.trim().replace('/', "\\"))
            .filter(|p| !p.is_empty())
            .collect()
    };

    // Groups in file order (src_0, src_1, ...), not the map's alphabetical order.
    let mut keys: Vec<&String> = groups.keys().collect();
    keys.sort_by_key(|k| (k.trim_start_matches("src_").parse::<u32>().unwrap_or(u32::MAX), (*k).clone()));

    // Each folder once, in the first group that lists it.
    let mut seen: Vec<String> = Vec::new();
    let mut grouped: Vec<(&String, Vec<Source>)> = Vec::new();
    let mut listed = 0;
    for key in keys {
        let mut sources = Vec::new();
        for path in paths(&groups[key]) {
            listed += 1;
            if !seen.contains(&norm(&path)) {
                seen.push(norm(&path));
                sources.push(Source { path, folder_name: String::new() });
            }
        }
        if !sources.is_empty() {
            grouped.push((key, sources));
        }
    }
    let total = seen.len();
    if total == 0 {
        return Err("לא נמצאו תיקיות מקור בקובץ".into());
    }

    let dests = value.get("copy_destinations").map(paths).unwrap_or_default();
    let destination = dests.first().cloned().unwrap_or_default();
    let group_count = grouped.len();

    // (id, name, sources)
    let mut parts: Vec<(String, String, Vec<Source>)> = match split {
        Split::One => vec![(
            TASK_ID.into(),
            "גיבוי מהקובץ".into(),
            grouped.into_iter().flat_map(|(_, s)| s).collect(),
        )],
        Split::Group => grouped
            .into_iter()
            .map(|(key, s)| (format!("{TASK_ID}-{key}"), format!("גיבוי מהקובץ - {}", names_of(&s)), s))
            .collect(),
        Split::Source => grouped
            .into_iter()
            .flat_map(|(_, s)| s)
            .map(|s| (path_id(&s.path), default_folder_name(&s.path), vec![s]))
            .collect(),
    };

    // One destination for all: two folders with the same last name would write the same dated
    // folders, so the second one gets " (2)" (like the task editor does).
    let mut taken: Vec<String> = Vec::new();
    for (_, _, sources) in &mut parts {
        for s in sources {
            let base = default_folder_name(&s.path);
            let mut name = base.clone();
            let mut n = 2;
            while taken.contains(&name.to_lowercase()) {
                name = format!("{base} ({n})");
                n += 1;
            }
            taken.push(name.to_lowercase());
            s.folder_name = name;
        }
    }
    // Two folders named alike in split mode would also give two tasks the same name.
    if split == Split::Source {
        for (_, name, sources) in &mut parts {
            *name = sources[0].folder_name.clone();
        }
    }

    // Notes about the whole file go on the first task only.
    let mut file_warnings = Vec::new();
    if listed > total {
        file_warnings.push(format!(
            "{} תיקיות הופיעו ביותר מקבוצה אחת - כל תיקייה יובאה פעם אחת{}",
            listed - total,
            if split == Split::One { ", וכל הקבוצות אוחדו למשימה אחת" } else { "" }
        ));
    }
    if dests.len() > 1 {
        file_warnings.push(format!("בקובץ כמה יעדים - יובא רק הראשון ({})", ltr(&dests[0])));
    }
    if dests.is_empty() {
        file_warnings.push("לא נמצא יעד בקובץ - יש לבחור יעד לפני השמירה".into());
    }
    file_warnings.push(if parts.len() == 1 {
        "הקובץ לא כולל תזמון - המשימה תיובא כהפעלה ידנית".into()
    } else {
        "הקובץ לא כולל תזמון - המשימות ייובאו כהפעלה ידנית".into()
    });

    let tasks = parts
        .into_iter()
        .enumerate()
        .map(|(i, (id, name, sources))| Imported {
            task: Task {
                id,
                name,
                sources,
                destination: destination.clone(),
                schedule: Schedule::Manual,
                ..Task::default()
            },
            warnings: if i == 0 { std::mem::take(&mut file_warnings) } else { Vec::new() },
        })
        .collect();
    Ok(Some(Parsed {
        tasks,
        groups: group_count,
        sources: total,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{"copy_sources":{"src_10":["D:/b",""],"src_2":["D:/a","d:\\B\\","E:/x/a"],"src_0":["C:/x"]},
                           "copy_destinations":["H:/","I:/"]}"#;

    #[test]
    fn merges_groups_and_skips_blanks() {
        let p = parse(JSON.as_bytes(), Split::One).unwrap().unwrap();
        assert_eq!((p.groups, p.sources, p.tasks.len()), (2, 4, 1));
        let t = &p.tasks[0].task;
        assert_eq!(t.id, TASK_ID);
        let paths: Vec<_> = t.sources.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, ["C:\\x", "D:\\a", "d:\\B\\", "E:\\x\\a"]);
        let names: Vec<_> = t.sources.iter().map(|s| s.folder_name.as_str()).collect();
        assert_eq!(names, ["x", "a", "B", "a (2)"]);
        assert_eq!(t.destination, "H:\\");
        assert!(p.tasks[0].warnings.len() >= 3);
    }

    #[test]
    fn splits_per_group_and_per_folder() {
        let p = parse(JSON.as_bytes(), Split::Group).unwrap().unwrap();
        let ids: Vec<_> = p.tasks.iter().map(|i| i.task.id.as_str()).collect();
        assert_eq!(ids, ["simple-config-src_0", "simple-config-src_2"]);
        assert_eq!(p.tasks[1].task.sources.len(), 3);
        assert!(p.tasks[1].warnings.is_empty());

        let p = parse(JSON.as_bytes(), Split::Source).unwrap().unwrap();
        assert_eq!(p.tasks.len(), 4);
        let names: Vec<_> = p.tasks.iter().map(|i| i.task.name.as_str()).collect();
        assert_eq!(names, ["x", "a", "B", "a (2)"]);
        assert!(p.tasks.iter().all(|i| i.task.destination == "H:\\" && i.task.sources.len() == 1));
        assert!(p.tasks.iter().all(|i| is_simple_id(&i.task.id)));
        // Same folder -> same id on the next import (case and trailing slash don't matter).
        assert_eq!(path_id("D:\\B"), path_id("d:\\b\\"));
    }

    #[test]
    fn other_json_is_not_this_format() {
        assert!(parse(br#"{"app":"backuper","tasks":[]}"#, Split::One).unwrap().is_none());
        assert!(parse(b"not json", Split::One).unwrap().is_none());
    }
}
