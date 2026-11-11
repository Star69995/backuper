//! Tauri commands invoked by the UI.

use crate::backup;
use crate::core::{Core, Job};
use crate::model::{BackupMode, FilterRule, Notice, Progress, RunRecord, RunStatus, Schedule, Settings, SourceBackups, Task, TaskState, Trigger};
use crate::schedule;
use serde::Serialize;
use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::sync::Arc;
use tauri::State;
use tauri_plugin_autostart::ManagerExt;

type CoreState<'a> = State<'a, Arc<Core>>;
type UpdatesState<'a> = State<'a, Arc<crate::updater::Updates>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    tasks: Vec<Task>,
    states: HashMap<String, TaskState>,
    current: Option<Progress>,
    queue: Vec<Job>,
    settings: Settings,
    autostart: bool,
    /// Tasks whose drive was connected, waiting for the user to say whether to back up.
    drive_prompts: Vec<String>,
    notice: Option<Notice>,
    update: crate::model::UpdateStatus,
    /// Per task: how many of its folders a Cobian takeover renamed (recorded, can be put back).
    cobian_renamed: HashMap<String, usize>,
}

#[tauri::command]
pub fn get_snapshot(core: CoreState, updates: UpdatesState, app: tauri::AppHandle) -> Snapshot {
    let s = core.store.lock().unwrap();
    Snapshot {
        tasks: s.tasks.clone(),
        states: s.states.clone(),
        current: core.current.lock().unwrap().clone(),
        queue: core.queued(),
        settings: s.settings.clone(),
        drive_prompts: core.drive_prompts(),
        notice: s.notice.clone(),
        update: updates.status(),
        cobian_renamed: s.cobian_renames.iter().fold(HashMap::new(), |mut m, r| {
            *m.entry(r.task_id.clone()).or_insert(0) += 1;
            m
        }),
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
    t.group = t.group.trim().to_string();
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
    backup::validate_task(&t)
        .and_then(|_| crate::drives::validate(&t))
        .map_err(|e| format!("{}: {e}", if t.name.is_empty() { "משימה" } else { &t.name }))?;
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
        core.cancel(id, false);
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

/// Queues tasks to run now. A paused backup is continued, unless `fresh`. Returns how many were actually queued (others were already queued/running).
#[tauri::command]
pub fn run_tasks(core: CoreState, ids: Vec<String>, mode: Option<BackupMode>, fresh: bool) -> usize {
    ids.into_iter()
        .filter(|id| {
            core.enqueue(Job {
                task_id: id.clone(),
                mode,
                trigger: Trigger::Manual,
                fresh,
            })
        })
        .count()
}

/// Continues a task whose last run was stopped or failed (e.g. destination full): same mode as that
/// run, and `backup_source` picks up in the folder it left. Returns false if the task is already
/// queued or running.
#[tauri::command]
pub fn resume_task(core: CoreState, id: String) -> bool {
    let mode = core
        .store
        .lock()
        .unwrap()
        .history
        .iter()
        .find(|r| r.task_id == id)
        .filter(|r| matches!(r.status, RunStatus::Cancelled | RunStatus::Failed))
        .map(|r| r.mode);
    core.enqueue(Job {
        task_id: id,
        mode,
        trigger: Trigger::Manual,
        fresh: false,
    })
}

/// Answers the "drive connected" question: queues `run`, drops the rest of `ids`.
#[tauri::command]
pub fn answer_drive_prompts(core: CoreState, ids: Vec<String>, run: Vec<String>) {
    core.answer_drive_prompts(&ids, &run);
}

#[tauri::command]
pub fn cancel_task(core: CoreState, id: String, discard: bool) {
    core.cancel(&id, discard);
}

#[tauri::command]
pub fn skip_totals_scan(core: CoreState) {
    core.skip_totals_scan();
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

/// Free and total bytes of the drive a path is on.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSpace {
    free: u64,
    total: u64,
}

/// The drive of `path`, asked through the nearest folder that exists (a destination may not
/// have been created yet). None = no such drive (disconnected) or it can't be read.
fn disk_space_of(path: &str) -> Option<DiskSpace> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(dir: *const u16, avail: *mut u64, total: *mut u64, total_free: *mut u64) -> i32;
    }
    std::path::Path::new(path).ancestors().find_map(|dir| {
        if dir.as_os_str().is_empty() {
            return None;
        }
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
        let (mut free, mut total, mut total_free) = (0u64, 0u64, 0u64);
        // `free` = what this user can use (quotas), which is what a backup can fill.
        (unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) } != 0)
            .then_some(DiskSpace { free, total })
    })
}

/// Free space on the drives of `paths` (task destinations), in the same order.
#[tauri::command]
pub async fn disk_space(paths: Vec<String>) -> Result<Vec<Option<DiskSpace>>, String> {
    tauri::async_runtime::spawn_blocking(move || paths.iter().map(|p| disk_space_of(p)).collect())
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

/// Live check of a regex filter: error, plain-language reading and an optional try-out on a sample name.
#[tauri::command]
pub fn check_regex(pattern: String, sample: Option<String>) -> crate::model::RegexCheck {
    crate::regexcheck::check(&pattern, sample.as_deref())
}

/// The filter check: what all the rules together (task + global) do with one path, and each rule's part.
#[tauri::command]
pub fn check_filters(
    rules: Vec<FilterRule>,
    global_rules: Vec<FilterRule>,
    sources: Vec<String>,
    probe: crate::model::FilterProbe,
) -> crate::model::FilterCheck {
    crate::filtercheck::check(&rules, &global_rules, &sources, &probe)
}

#[tauri::command]
pub fn save_settings(core: CoreState, settings: Settings) -> Result<(), String> {
    crate::filters::validate(&settings.global_filters)?;
    let mut s = core.store.lock().unwrap();
    let new_copy_dir = settings.task_list_copy_dir.trim() != s.settings.task_list_copy_dir.trim();
    let previous = std::mem::replace(&mut s.settings, settings);
    // A newly chosen folder gets the copy right away, so a wrong folder is reported now.
    if new_copy_dir {
        if let Err(e) = s.write_task_list_copy() {
            s.settings = previous;
            return Err(e);
        }
    }
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
    /// It exists and is identical, so importing it changes nothing.
    unchanged: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    /// "backuper" (export / snapshot / tasks.json) | "simple" (copy_sources JSON) | "cobian"
    source: &'static str,
    tasks: Vec<ImportedTask>,
    /// Plain config only: how it can be split (non-empty groups, distinct folders).
    simple: Option<SimpleShape>,
    /// Tasks of an earlier import of a plain config in another split, replaced by this one
    /// (deleted when importing, so their folders don't count as conflicts).
    replaces: Vec<ReplacedTask>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimpleShape {
    groups: usize,
    sources: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacedTask {
    id: String,
    name: String,
}

/// Reads a task list file (this app's export or snapshot, a plain JSON config, or a Cobian .lst)
/// without saving anything. `split` = how a plain config becomes tasks (ignored for the others).
#[tauri::command]
pub fn import_tasks(
    core: CoreState,
    path: String,
    split: Option<crate::simpleconfig::Split>,
) -> Result<ImportPreview, String> {
    let bytes = std::fs::read(&path).map_err(|e| format!("לא ניתן לקרוא את הקובץ: {e}"))?;
    let mut simple = None;
    let (source, imported) = match crate::tasklist::parse(&bytes)? {
        Some(tasks) => (
            "backuper",
            tasks
                .into_iter()
                .map(|task| crate::cobian::Imported {
                    task,
                    warnings: Vec::new(),
                })
                .collect(),
        ),
        None => match crate::simpleconfig::parse(&bytes, split.unwrap_or_default())? {
            Some(p) => {
                simple = Some(SimpleShape {
                    groups: p.groups,
                    sources: p.sources,
                });
                ("simple", p.tasks)
            }
            None => ("cobian", crate::cobian::parse_file(&bytes)?),
        },
    };
    let mut existing = core.store.lock().unwrap().tasks.clone();
    // Importing a plain config split differently than last time replaces the earlier tasks.
    let mut replaces = Vec::new();
    if simple.is_some() {
        existing.retain(|t| {
            let old = crate::simpleconfig::is_simple_id(&t.id) && !imported.iter().any(|i| i.task.id == t.id);
            if old {
                replaces.push(ReplacedTask {
                    id: t.id.clone(),
                    name: t.name.clone(),
                });
            }
            !old
        });
    }
    let tasks = imported
        .into_iter()
        .map(|i| {
            let current = existing.iter().find(|t| t.id == i.task.id);
            let exists = current.is_some();
            match prepare(i.task.clone()) {
                Ok(task) => {
                    let error = check_folder_conflicts(&existing, std::slice::from_ref(&task)).err();
                    let unchanged = current == Some(&task);
                    ImportedTask {
                        task,
                        warnings: i.warnings,
                        error,
                        exists,
                        unchanged,
                    }
                }
                Err(error) => ImportedTask {
                    task: i.task,
                    warnings: i.warnings,
                    error: Some(error),
                    exists,
                    unchanged: false,
                },
            }
        })
        .collect();
    Ok(ImportPreview {
        source,
        tasks,
        simple,
        replaces,
    })
}

/// Saves the given tasks (all of them when `ids` is None) to a file. Returns how many.
#[tauri::command]
pub fn export_tasks(core: CoreState, path: String, ids: Option<Vec<String>>) -> Result<usize, String> {
    let tasks: Vec<Task> = {
        let s = core.store.lock().unwrap();
        s.tasks
            .iter()
            .filter(|t| ids.as_ref().is_none_or(|ids| ids.contains(&t.id)))
            .cloned()
            .collect()
    };
    crate::tasklist::write_file(std::path::Path::new(&path), &tasks).map_err(|e| format!("השמירה נכשלה: {e}"))?;
    Ok(tasks.len())
}

/// Snapshots of the task list, newest first: the automatic ones, or those in a folder the
/// user picked (e.g. the copy folder on the backup drive, after reinstalling).
#[tauri::command]
pub fn list_task_snapshots(core: CoreState, dir: Option<String>) -> Vec<crate::tasklist::Snapshot> {
    let dir = match dir {
        Some(d) => crate::tasklist::snapshots_in(std::path::Path::new(&d)),
        None => core.store.lock().unwrap().snapshots_dir(),
    };
    crate::tasklist::list_snapshots(&dir)
}

/// Shows a sample notification with `sound`, so the user can hear it before choosing.
#[tauri::command]
pub fn test_sound(app: tauri::AppHandle, sound: String) {
    let body = if sound.is_empty() { "ללא צליל" } else { "כך תישמע ההתראה" };
    crate::core::toast(&app, "בדיקת צליל - Backuper", body, &sound);
}

#[tauri::command]
pub fn dismiss_notice(core: CoreState) {
    core.store.lock().unwrap().notice = None;
    core.changed();
}

/// Checks GitHub for a newer version now. Returns whether one is available.
#[tauri::command]
pub async fn check_updates(updates: UpdatesState<'_>) -> Result<bool, String> {
    updates.check().await
}

/// Downloads and installs the newer version (the app restarts); waits for running backups first.
#[tauri::command]
pub async fn install_update(updates: UpdatesState<'_>) -> Result<(), String> {
    updates.install_now().await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CobianLists {
    lists: Vec<crate::cobianmigrate::CobianList>,
    /// Where a file dialog for picking a list should open.
    dir: Option<String>,
}

/// Cobian Backup / Reflector task lists found on this computer.
#[tauri::command]
pub async fn find_cobian_lists() -> Result<CobianLists, String> {
    tauri::async_runtime::spawn_blocking(|| CobianLists {
        lists: crate::cobianmigrate::find_lists(),
        dir: crate::cobianmigrate::lists_dir(),
    })
    .await
    .map_err(|e| e.to_string())
}

/// The rest of the task list, with `incoming` in place of tasks that have the same id.
fn others_with(existing: &[Task], incoming: &[Task]) -> Vec<Task> {
    let mut all: Vec<Task> = existing.iter().filter(|t| !incoming.iter().any(|i| i.id == t.id)).cloned().collect();
    all.extend(incoming.iter().cloned());
    all
}

/// Which existing Cobian backup folders each task would take over (nothing is changed).
/// `tasks` may be saved tasks or ones still in the import preview.
#[tauri::command]
pub async fn plan_cobian_takeover(core: CoreState<'_>, tasks: Vec<Task>) -> Result<Vec<crate::cobianmigrate::AdoptPlan>, String> {
    let existing = core.store.lock().unwrap().tasks.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let all = others_with(&existing, &tasks);
        tasks.iter().map(|t| crate::cobianmigrate::plan(t, &all)).collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// Sets the tasks' Cobian decision: `take_over` = continue from Cobian's backups (renamed at the
/// task's next run, see `Core::take_over_cobian`). Either way the task no longer waits for an answer.
fn set_cobian_decision(core: &Core, ids: &[String], take_over: bool) {
    {
        let mut s = core.store.lock().unwrap();
        let mut changed = false;
        for t in s.tasks.iter_mut().filter(|t| ids.contains(&t.id)) {
            if t.cobian_pending || t.cobian_takeover != take_over {
                t.cobian_pending = false;
                t.cobian_takeover = take_over;
                changed = true;
            }
        }
        if changed {
            s.save_tasks();
        }
    }
    core.changed();
}

/// "Continue from Cobian's backups": nothing is renamed now. The folders get this app's names right
/// before each task's next run, so Cobian keeps seeing them until this app takes over.
#[tauri::command]
pub fn take_over_cobian(core: CoreState, ids: Vec<String>) {
    set_cobian_decision(&core, &ids, true);
}

/// "Don't continue from Cobian's backups" (also cancels a takeover that hasn't run yet): the
/// tasks' first run will be a full backup, so they may run.
#[tauri::command]
pub fn decline_cobian_takeover(core: CoreState, ids: Vec<String>) {
    set_cobian_decision(&core, &ids, false);
}

fn cobian_restore_plan(
    core: &Core,
    id: &str,
    naming: &crate::cobianmigrate::CobianNaming,
) -> Result<crate::cobianmigrate::RestorePlan, String> {
    let (task, journal, never_backed_up) = {
        let s = core.store.lock().unwrap();
        let task = s.task(id).cloned().ok_or("המשימה לא נמצאה")?;
        let never = s.states.get(id).is_none_or(|st| st.last_backup.is_none());
        (task, s.cobian_renames.clone(), never)
    };
    Ok(crate::cobianmigrate::restore_plan(&task, &journal, never_backed_up, naming))
}

/// Which of the tasks' folders would get Cobian's names back (nothing is changed).
#[tauri::command]
pub async fn plan_cobian_restore(
    core: CoreState<'_>,
    ids: Vec<String>,
    naming: crate::cobianmigrate::CobianNaming,
) -> Result<Vec<crate::cobianmigrate::RestorePlan>, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || ids.iter().map(|id| cobian_restore_plan(&core, id, &naming)).collect())
        .await
        .map_err(|e| e.to_string())?
}

/// Gives the tasks' taken-over folders Cobian's names back, so Cobian sees them again. Only while no
/// backup runs. A task that got names back is switched off (it would otherwise run next to Cobian),
/// and none of them takes Cobian's folders over again by itself.
#[tauri::command]
pub async fn restore_cobian_names(
    core: CoreState<'_>,
    ids: Vec<String>,
    naming: crate::cobianmigrate::CobianNaming,
) -> Result<crate::cobianmigrate::AdoptResult, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let results = core
            .when_idle(|| {
                ids.iter()
                    .map(|id| cobian_restore_plan(&core, id, &naming).map(|p| (id.clone(), crate::cobianmigrate::restore(&p))))
                    .collect::<Result<Vec<_>, String>>()
            })
            .ok_or_else(|| "גיבוי רץ עכשיו או ממתין בתור - נסו שוב כשהגיבויים יסתיימו".to_string())??;
        let mut total = crate::cobianmigrate::AdoptResult::default();
        {
            let mut s = core.store.lock().unwrap();
            let before = s.cobian_renames.len();
            let mut tasks_changed = false;
            for (id, r) in &results {
                s.cobian_renames
                    .retain(|e| !(e.task_id == *id && r.done.iter().any(|d| d.from == e.from && d.to == e.to)));
                if let Some(t) = s.tasks.iter_mut().find(|t| t.id == *id) {
                    if t.cobian_takeover || (r.renamed > 0 && t.enabled) {
                        t.cobian_takeover = false;
                        if r.renamed > 0 {
                            t.enabled = false;
                        }
                        tasks_changed = true;
                    }
                }
            }
            if s.cobian_renames.len() != before {
                s.save_cobian_renames();
            }
            if tasks_changed {
                s.save_tasks();
            }
        }
        for (_, r) in results {
            total.renamed += r.renamed;
            total.errors.extend(r.errors);
        }
        core.changed();
        Ok(total)
    })
    .await
    .map_err(|e| e.to_string())?
}
