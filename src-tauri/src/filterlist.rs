//! A list of filter rules as a file (export/import of the rule editor), so a long list can move
//! between tasks, the global rules and machines.

use crate::model::FilterRule;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const APP: &str = "backuper";
const KIND: &str = "filters";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FilterListFile<'a> {
    app: &'static str,
    kind: &'static str,
    version: u32,
    saved_at: DateTime<Local>,
    rules: &'a [FilterRule],
}

/// What an import found: the rules that are complete and valid, and how many were left out.
#[derive(Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FilterImport {
    pub rules: Vec<FilterRule>,
    pub skipped: usize,
}

/// Writes via a temp file + rename, so a crash never leaves half a file.
pub fn write_file(path: &Path, rules: &[FilterRule]) -> Result<(), String> {
    let file = FilterListFile {
        app: APP,
        kind: KIND,
        version: 1,
        saved_at: Local::now(),
        rules,
    };
    let json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Accepts an export of this app or a bare array of rules. A rule that can't be read or fails
/// validation is counted in `skipped`, the rest still come through.
pub fn parse(bytes: &[u8]) -> Result<FilterImport, String> {
    const NOT_A_LIST: &str = "זה לא קובץ של כללי סינון של Backuper.";
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| NOT_A_LIST.to_string())?;
    let items = match value {
        serde_json::Value::Array(a) => a,
        serde_json::Value::Object(o) if o.get("app").and_then(|a| a.as_str()) == Some(APP) => {
            match (o.get("kind").and_then(|k| k.as_str()), o.get("rules")) {
                (Some(KIND), Some(serde_json::Value::Array(a))) => a.clone(),
                (Some("tasks") | None, _) | (_, None) => {
                    return Err("זה קובץ של רשימת משימות ולא של כללי סינון. לייבוא משימות יש כפתור בדף המשימות.".into())
                }
                _ => return Err(NOT_A_LIST.into()),
            }
        }
        _ => return Err(NOT_A_LIST.into()),
    };
    let mut out = FilterImport::default();
    for item in items {
        match serde_json::from_value::<FilterRule>(item) {
            Ok(r) if crate::filters::validate(std::slice::from_ref(&r)).is_ok() => out.rules.push(r),
            _ => out.skipped += 1,
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_skips() {
        let rules = vec![
            FilterRule::Extension { value: "tmp, log".into() },
            FilterRule::Folder { value: "node_modules".into() },
            FilterRule::System,
        ];
        let dir = std::env::temp_dir().join(format!("backuper-filterlist-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.json");
        write_file(&file, &rules).unwrap();
        let back = parse(&fs::read(&file).unwrap()).unwrap();
        assert_eq!((back.rules, back.skipped), (rules, 0));
        fs::remove_dir_all(&dir).unwrap();

        // Bare array: an empty rule and an unknown kind are left out, the rest stays.
        let bare = br#"[{"kind":"folder","value":"cache"},{"kind":"folder","value":" "},{"kind":"nope"}]"#;
        let r = parse(bare).unwrap();
        assert_eq!((r.rules.len(), r.skipped), (1, 2));
        assert!(parse(b"{\"app\":\"backuper\",\"tasks\":[]}").is_err());
        assert!(parse(b"hello").is_err());
    }
}
