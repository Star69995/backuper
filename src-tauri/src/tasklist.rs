//! The task list as a file: export/import, automatic snapshots on every change (in the app
//! data dir, and optionally in a folder of the user's choice, e.g. the backup drive).
//! Snapshot files and folders are hidden + system (files also read-only), so they don't show
//! in Explorer and aren't deleted by accident.

use crate::model::Task;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const APP: &str = "backuper";
/// Snapshots kept per folder (newest first).
const MAX_SNAPSHOTS: usize = 50;
/// Folder for the snapshots inside the user's copy folder.
pub const COPY_DIR_NAME: &str = "Backuper - גיבויי רשימת משימות";
/// The single, overwritten copy that older versions kept in the copy folder.
pub const OLD_COPY_FILE_NAME: &str = "Backuper - רשימת משימות.json";

const READONLY: u32 = 0x1;
const HIDDEN: u32 = 0x2;
const SYSTEM: u32 = 0x4;
const NORMAL: u32 = 0x80;

fn set_attributes(path: &Path, attrs: u32) -> bool {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetFileAttributesW(name: *const u16, attrs: u32) -> i32;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe { SetFileAttributesW(wide.as_ptr(), attrs) != 0 }
}

/// Removes a (protected) snapshot file.
fn remove_protected(path: &Path) {
    set_attributes(path, NORMAL);
    let _ = fs::remove_file(path);
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskListFile {
    app: String,
    version: u32,
    saved_at: DateTime<Local>,
    tasks: Vec<Task>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub path: String,
    pub saved_at: DateTime<Local>,
    pub task_count: usize,
}

pub fn to_json(tasks: &[Task]) -> String {
    let file = TaskListFile {
        app: APP.into(),
        version: 1,
        saved_at: Local::now(),
        tasks: tasks.to_vec(),
    };
    serde_json::to_string_pretty(&file).unwrap_or_default()
}

/// Writes via a temp file + rename, so a crash never leaves half a file.
pub fn write_file(path: &Path, tasks: &[Task]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, to_json(tasks)).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Ok(None) = not a task list of this app (try other formats).
/// Accepts an export/snapshot file or a bare `tasks.json` array.
pub fn parse(bytes: &[u8]) -> Result<Option<Vec<Task>>, String> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return Ok(None);
    };
    let tasks = match value {
        serde_json::Value::Array(_) => value,
        serde_json::Value::Object(ref o) if o.get("app").and_then(|a| a.as_str()) == Some(APP) => {
            o.get("tasks").cloned().unwrap_or_default()
        }
        _ => return Ok(None),
    };
    let mut tasks: Vec<Task> = serde_json::from_value(tasks).map_err(|e| format!("קובץ המשימות פגום: {e}"))?;
    tasks.iter_mut().for_each(Task::migrate);
    Ok(Some(tasks))
}

fn read(path: &Path) -> Option<(DateTime<Local>, Vec<Task>)> {
    let file: TaskListFile = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    Some((file.saved_at, file.tasks))
}

/// Snapshot files, newest first ("tasks 2026-10-02 14-30-05.json" sorts by time).
fn snapshot_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("tasks ") && n.ends_with(".json"))
        })
        .collect();
    files.sort();
    files.reverse();
    files
}

/// The newest snapshot in `dir` that can still be read.
pub fn latest(dir: &Path) -> Option<(DateTime<Local>, Vec<Task>)> {
    snapshot_files(dir).iter().find_map(|f| read(f))
}

/// Saves a snapshot unless the newest one already holds the same tasks; keeps the newest MAX_SNAPSHOTS.
pub fn snapshot(dir: &Path, tasks: &[Task]) -> Result<(), String> {
    let files = snapshot_files(dir);
    if files.first().and_then(|f| read(f)).is_some_and(|(_, t)| t == tasks) {
        return Ok(());
    }
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    set_attributes(dir, HIDDEN | SYSTEM);
    let stamp = Local::now().format("%Y-%m-%d %H-%M-%S").to_string();
    // Several changes within the same second get "_002", "_003"... (sorts after ".json", in order).
    // Always above the highest existing number, so a name freed by rotation is never reused.
    let base = format!("tasks {stamp}");
    let last = files
        .iter()
        .filter_map(|f| f.file_stem()?.to_str()?.strip_prefix(&base).map(String::from))
        .map(|rest| rest.strip_prefix('_').and_then(|n| n.parse::<u32>().ok()).unwrap_or(1))
        .max();
    let path = match last {
        None => dir.join(format!("{base}.json")),
        Some(n) => dir.join(format!("{base}_{:03}.json", n + 1)),
    };
    write_file(&path, tasks)?;
    set_attributes(&path, READONLY | HIDDEN | SYSTEM);
    for old in snapshot_files(dir).into_iter().skip(MAX_SNAPSHOTS) {
        remove_protected(&old);
    }
    Ok(())
}

/// Where the snapshots are in a folder the user picked: its snapshot subfolder when it has
/// one (the copy folder from the settings), otherwise the folder itself.
pub fn snapshots_in(dir: &Path) -> PathBuf {
    let sub = dir.join(COPY_DIR_NAME);
    if sub.is_dir() {
        sub
    } else {
        dir.to_path_buf()
    }
}

pub fn list_snapshots(dir: &Path) -> Vec<Snapshot> {
    snapshot_files(dir)
        .into_iter()
        .filter_map(|p| {
            let (saved_at, tasks) = read(&p)?;
            Some(Snapshot {
                path: p.to_string_lossy().into_owned(),
                saved_at,
                task_count: tasks.len(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(name: &str) -> Task {
        Task {
            id: name.into(),
            name: name.into(),
            ..Task::default()
        }
    }

    #[test]
    fn round_trips_and_reads_bare_arrays() {
        let tasks = vec![task("a"), task("b")];
        assert_eq!(parse(to_json(&tasks).as_bytes()).unwrap().unwrap(), tasks);
        let bare = serde_json::to_string(&tasks).unwrap();
        assert_eq!(parse(bare.as_bytes()).unwrap().unwrap(), tasks);
        assert!(parse(b"<\xa7- x -\xa7>").unwrap().is_none());
        assert!(parse(br#"{"other": 1}"#).unwrap().is_none());
    }

    #[test]
    fn snapshots_skip_duplicates_and_rotate() {
        let dir = std::env::temp_dir().join(format!("backuper-snap-{}", uuid::Uuid::new_v4()));
        snapshot(&dir, &[task("a")]).unwrap();
        snapshot(&dir, &[task("a")]).unwrap();
        assert_eq!(list_snapshots(&dir).len(), 1, "same content isn't saved twice");
        for i in 0..MAX_SNAPSHOTS + 5 {
            snapshot(&dir, &[task(&i.to_string())]).unwrap();
        }
        let list = list_snapshots(&dir);
        assert_eq!(list.len(), MAX_SNAPSHOTS);
        assert_eq!(
            read(Path::new(&list[0].path)).unwrap().1,
            [task(&(MAX_SNAPSHOTS + 4).to_string())]
        );
        assert!(
            fs::metadata(&list[0].path).unwrap().permissions().readonly(),
            "snapshots are protected"
        );
        assert_eq!(latest(&dir).unwrap().1, [task(&(MAX_SNAPSHOTS + 4).to_string())]);
        for s in &list {
            remove_protected(Path::new(&s.path));
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
