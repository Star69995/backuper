//! Import of a plain JSON backup config: `{"copy_sources": {"src_0": [paths], ...}, "copy_destinations": [paths]}`.
//!
//! The file has no schedule, filters or retention, so everything becomes one manual task with the
//! app's defaults. The groups are merged: a folder listed in several groups is one source.

use crate::cobian::Imported;
use crate::model::{Schedule, Source, Task};

/// Fixed id, so importing the same file again updates the task instead of duplicating it.
const TASK_ID: &str = "simple-config";

fn ltr(s: &str) -> String {
    format!("\u{2066}{s}\u{2069}")
}

/// Ok(None) = not this format (try others).
pub fn parse(bytes: &[u8]) -> Result<Option<Imported>, String> {
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

    let mut warnings = Vec::new();
    let mut sources: Vec<Source> = Vec::new();
    let mut listed = 0;
    for key in keys {
        for path in paths(&groups[key]) {
            listed += 1;
            let norm = path.trim_end_matches('\\').to_lowercase();
            if !sources.iter().any(|s| s.path.trim_end_matches('\\').to_lowercase() == norm) {
                sources.push(Source { path, folder_name: String::new() });
            }
        }
    }
    if sources.is_empty() {
        return Err("לא נמצאו תיקיות מקור בקובץ".into());
    }
    if listed > sources.len() {
        warnings.push(format!(
            "{} תיקיות הופיעו ביותר מקבוצה אחת - כל תיקייה יובאה פעם אחת, וכל הקבוצות אוחדו למשימה אחת",
            listed - sources.len()
        ));
    }

    let dests = value.get("copy_destinations").map(paths).unwrap_or_default();
    if dests.len() > 1 {
        warnings.push(format!(
            "בקובץ כמה יעדים - יובא רק הראשון ({})",
            ltr(&dests[0])
        ));
    }
    if dests.is_empty() {
        warnings.push("לא נמצא יעד בקובץ - יש לבחור יעד לפני השמירה".into());
    }
    warnings.push("הקובץ לא כולל תזמון - המשימה תיובא כהפעלה ידנית".into());

    let task = Task {
        id: TASK_ID.into(),
        name: "גיבוי מהקובץ".into(),
        sources,
        destination: dests.into_iter().next().unwrap_or_default(),
        schedule: Schedule::Manual,
        ..Task::default()
    };
    Ok(Some(Imported { task, warnings }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_groups_and_skips_blanks() {
        let json = r#"{"copy_sources":{"src_10":["D:/b",""],"src_2":["D:/a","d:\\B\\"],"src_0":["C:/x"]},
                       "copy_destinations":["H:/","I:/"]}"#;
        let i = parse(json.as_bytes()).unwrap().unwrap();
        let paths: Vec<_> = i.task.sources.iter().map(|s| s.path.as_str()).collect();
        assert_eq!(paths, ["C:\\x", "D:\\a", "d:\\B\\"]);
        assert_eq!(i.task.destination, "H:\\");
        assert!(i.warnings.len() >= 3);
    }

    #[test]
    fn other_json_is_not_this_format() {
        assert!(parse(br#"{"app":"backuper","tasks":[]}"#).unwrap().is_none());
        assert!(parse(b"not json").unwrap().is_none());
    }
}
