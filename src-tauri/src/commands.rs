//! Tauri commands invoked by the UI.

use crate::backup;
use crate::core::{Core, Job};
use crate::model::{BackupMode, Progress, RunRecord, Schedule, Settings, SourceBackups, Task, TaskState, Trigger};
use crate::schedule;
use serde::Serialize;
use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::sync::Arc;
use tauri::State;
use tauri_plugin_autostart::ManagerExt;

type CoreState<'a> = State<'a, Arc<Core>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    tasks: Vec<Task>,
    states: HashMap<String, TaskState>,
    current: Option<Progress>,
    queue: Vec<Job>,
    settings: Settings,
    autostart: bool,
}

#[tauri::command]
pub fn get_snapshot(core: CoreState, app: tauri::AppHandle) -> Snapshot {
    let s = core.store.lock().unwrap();
    Snapshot {
        tasks: s.tasks.clone(),
        states: s.states.clone(),
        current: core.current.lock().unwrap().clone(),
        queue: core.queued(),
        settings: s.settings.clone(),
        // Dev builds never register; show the saved choice there.
        autostart: if cfg!(debug_assertions) {
            s.settings.start_with_windows
        } else {
            app.autolaunch().is_enabled().unwrap_or(false)
        },
    }
}

fn prepare(mut t: Task) -> Result<Task, String> {
    t.migrate();
    // A full schedule only makes sense next to incremental runs (combined mode).
    if t.mode == BackupMode::Full {
        t.full_schedule = None;
    }
    t.name = t.name.trim().to_string();
    t.destination = t.destination.trim().to_string();
    for src in &mut t.sources {
        src.path = src.path.trim().to_string();
        if src.folder_name.trim().is_empty() {
            src.folder_name = backup::default_folder_name(&src.path);
        }
        src.folder_name = backup::sanitize_folder_name(&src.folder_name);
    }
    if t.id.is_empty() {
        t.id = uuid::Uuid::new_v4().to_string();
    }
    backup::validate_task(&t).map_err(|e| format!("{}: {e}", if t.name.is_empty() { "משימה" } else { &t.name }))?;
    Ok(t)
}

/// Two sources writing the same dated folders into the same destination would delete each other's backups.
/// `incoming` replaces the existing tasks with the same id.
fn check_folder_conflicts(existing: &[Task], incoming: &[Task]) -> Result<(), String> {
    let mut all: Vec<&Task> = existing.iter().filter(|t| !incoming.iter().any(|p| p.id == t.id)).collect();
    all.extend(incoming.iter());
    let key = |t: &Task, f: &str| format!("{}\\{}", t.destination.trim_end_matches('\\'), f).to_lowercase();
    let mut seen: HashMap<String, &str> = HashMap::new();
    for t in &all {
        for src in &t.sources {
            if let Some(other) = seen.insert(key(t, &src.folder_name), &t.name) {
                if other != t.name {
                    return Err(format!(
                        "למשימות \"{other}\" ו-\"{}\" יש תיקיית מקור עם אותו שם תיקיית גיבוי ({}) באותו יעד - יש לשנות אחד מהם",
                        t.name, src.folder_name
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Creates or updates tasks (used by the editor, bulk edit, undo and import). All-or-nothing.
#[tauri::command]
pub fn save_tasks(core: CoreState, tasks: Vec<Task>) -> Result<Vec<Task>, String> {
    let prepared = tasks.into_iter().map(prepare).collect::<Result<Vec<_>, _>>()?;
    {
        let mut s = core.store.lock().unwrap();
        check_folder_conflicts(&s.tasks, &prepared)?;
        for t in &prepared {
            match s.tasks.iter().position(|x| x.id == t.id) {
                Some(i) => {
                    let existing = std::mem::replace(&mut s.tasks[i], t.clone());
                    // A changed schedule gets its next run recomputed.
                    let st = s.states.entry(t.id.clone()).or_default();
                    if existing.schedule != t.schedule || existing.enabled != t.enabled {
                        st.next_run = None;
                    }
                    if existing.full_schedule != t.full_schedule || existing.enabled != t.enabled {
                        st.next_full_run = None;
                    }
                }
                None => s.tasks.push(t.clone()),
            }
        }
        s.save_tasks();
        s.save_states();
    }
    core.refresh_next_runs();
    core.changed();
    Ok(prepared)
}

#[tauri::command]
pub fn delete_tasks(core: CoreState, ids: Vec<String>) {
    for id in &ids {
        core.cancel(id);
    }
    let mut s = core.store.lock().unwrap();
    s.tasks.retain(|t| !ids.contains(&t.id));
    s.states.retain(|id, _| !ids.contains(id));
    s.save_tasks();
    s.save_states();
    drop(s);
    core.changed();
}

#[tauri::command]
pub fn reorder_tasks(core: CoreState, ids: Vec<String>) {
    let mut s = core.store.lock().unwrap();
    s.tasks
        .sort_by_key(|t| ids.iter().position(|i| *i == t.id).unwrap_or(usize::MAX));
    s.save_tasks();
    drop(s);
    core.changed();
}

/// Queues tasks to run now. Returns how many were actually queued (others were already queued/running).
#[tauri::command]
pub fn run_tasks(core: CoreState, ids: Vec<String>, mode: Option<BackupMode>) -> usize {
    ids.into_iter()
        .filter(|id| {
            core.enqueue(Job {
                task_id: id.clone(),
                mode,
                trigger: Trigger::Manual,
            })
        })
        .count()
}

#[tauri::command]
pub fn cancel_task(core: CoreState, id: String) {
    core.cancel(&id);
}

#[tauri::command]
pub fn get_history(core: CoreState) -> Vec<RunRecord> {
    core.store.lock().unwrap().history.clone()
}

#[tauri::command]
pub fn clear_history(core: CoreState) {
    core.store.lock().unwrap().clear_history();
    core.changed();
}

/// The robocopy log of a run (UTF-16 file), capped to the last ~4MB of text.
#[tauri::command]
pub async fn read_log(core: CoreState<'_>, run_id: String, source_index: usize) -> Result<String, String> {
    let path = {
        let s = core.store.lock().unwrap();
        s.history
            .iter()
            .find(|r| r.id == run_id)
            .and_then(|r| r.sources.get(source_index)?.log_file.clone())
    }
    .ok_or("אין יומן מפורט לריצה זו")?;
    let bytes = std::fs::read(&path).map_err(|_| "קובץ היומן לא נמצא".to_string())?;
    const MAX: usize = 8 * 1024 * 1024;
    let start = bytes.len().saturating_sub(MAX) & !1;
    let units: Vec<u16> = bytes[start..]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    Ok(String::from_utf16_lossy(&units).trim_start_matches('\u{feff}').to_string())
}

fn find_task(core: &Core, id: &str) -> Result<Task, String> {
    core.store
        .lock()
        .unwrap()
        .task(id)
        .cloned()
        .ok_or_else(|| "המשימה לא נמצאה".to_string())
}

#[tauri::command]
pub async fn list_backups(core: CoreState<'_>, task_id: String) -> Result<Vec<SourceBackups>, String> {
    let task = find_task(&core, &task_id)?;
    tauri::async_runtime::spawn_blocking(move || backup::list_task_backups(&task))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_backup(core: CoreState<'_>, task_id: String, folder_name: String, name: String) -> Result<(), String> {
    let task = find_task(&core, &task_id)?;
    if core.current.lock().unwrap().as_ref().is_some_and(|p| p.task_id == task_id) {
        return Err("לא ניתן למחוק גיבוי בזמן שהמשימה רצה".into());
    }
    tauri::async_runtime::spawn_blocking(move || backup::delete_backup(&task, &folder_name, &name))
        .await
        .map_err(|e| e.to_string())?
}

fn dir_size(path: &std::path::Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(path) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(ft) if ft.is_dir() => dir_size(&e.path()),
            Ok(ft) if ft.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

#[tauri::command]
pub async fn folder_size(path: String) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || dir_size(std::path::Path::new(&path)))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_path(path: String) -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .raw_arg(format!("\"{path}\""))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn preview_schedule(schedule: Schedule) -> Result<Vec<String>, String> {
    schedule::validate(&schedule)?;
    Ok(schedule::preview(&schedule, 3).iter().map(|d| d.to_rfc3339()).collect())
}

#[tauri::command]
pub fn save_settings(core: CoreState, settings: Settings) -> Result<(), String> {
    crate::filters::validate(&settings.global_filters)?;
    let mut s = core.store.lock().unwrap();
    s.settings = settings;
    s.save_settings();
    drop(s);
    core.changed();
    Ok(())
}

/// Makes the Windows startup entry match `enabled`. Debug builds never touch it (it would register the debug exe).
pub fn apply_autostart(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Ok(());
    }
    let al = app.autolaunch();
    if al.is_enabled().unwrap_or(false) == enabled {
        return Ok(());
    }
    if enabled { al.enable() } else { al.disable() }.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_autostart(core: CoreState, app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    apply_autostart(&app, enabled)?;
    let mut s = core.store.lock().unwrap();
    s.settings.start_with_windows = enabled;
    s.save_settings();
    drop(s);
    core.changed();
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedTask {
    task: Task,
    warnings: Vec<String>,
    /// Why the task can't be imported as is (validation / folder conflict).
    error: Option<String>,
    /// A task with this id already exists (importing again updates it).
    exists: bool,
}

/// Reads a Cobian task list (.lst) and converts its tasks, without saving anything.
#[tauri::command]
pub fn import_cobian(core: CoreState, path: String) -> Result<Vec<ImportedTask>, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("לא ניתן לקרוא את הקובץ: {e}"))?;
    let imported = crate::cobian::parse_file(&bytes)?;
    let existing = core.store.lock().unwrap().tasks.clone();
    Ok(imported
        .into_iter()
        .map(|i| {
            let exists = existing.iter().any(|t| t.id == i.task.id);
            match prepare(i.task.clone()) {
                Ok(task) => {
                    let error = check_folder_conflicts(&existing, std::slice::from_ref(&task)).err();
                    ImportedTask { task, warnings: i.warnings, error, exists }
                }
                Err(error) => ImportedTask { task: i.task, warnings: i.warnings, error: Some(error), exists },
            }
        })
        .collect())
}
