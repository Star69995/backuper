//! The running core: a single worker that executes queued backups one at a time,
//! and the built-in scheduler that enqueues due tasks. UI-independent apart from events.

use crate::backup;
use crate::drives::DriveWatch;
use crate::engine::{robocopy::Robocopy, CopyEngine, EngineEvent};
use crate::model::{BackupMode, DeleteProblem, DriveAction, Navigate, Progress, RunRecord, RunStatus, Schedule, Settings, SourceRun, Task, Trigger};
use crate::schedule;
use crate::store::Store;
use chrono::{Duration as ChronoDuration, Local};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

/// `backup::rename_previous` for each source of the task (same order).
fn rename_task_backups(task: &Task) -> Vec<backup::Renamed> {
    task.sources.iter().map(|s| backup::rename_previous(&task.destination, s)).collect()
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Slot {
    /// Not due yet.
    Idle,
    /// Was due but missed while off, and catch-up is disabled: skipped.
    Skipped,
    Run(Trigger),
}

/// Checks one schedule slot; if it is due, moves it to its next occurrence.
fn take_due(next: &mut Option<chrono::DateTime<Local>>, sched: &Schedule, now: chrono::DateTime<Local>, catch_up: bool) -> Slot {
    let Some(at) = *next else { return Slot::Idle };
    if now < at {
        return Slot::Idle;
    }
    *next = schedule::next_after(sched, now);
    let missed = now - at > ChronoDuration::minutes(MISSED_GRACE_MINUTES);
    match (missed, catch_up) {
        (false, _) => Slot::Run(Trigger::Scheduled),
        (true, true) => Slot::Run(Trigger::CatchUp),
        (true, false) => Slot::Skipped,
    }
}

/// Fills in a missing next-run time, or clears it when the schedule is off/manual. Returns true if changed.
fn refresh_slot(next: &mut Option<chrono::DateTime<Local>>, sched: Option<&Schedule>, now: chrono::DateTime<Local>) -> bool {
    match sched.filter(|s| **s != Schedule::Manual) {
        None if next.is_some() => {
            *next = None;
            true
        }
        Some(s) if next.is_none() => {
            *next = schedule::next_after(s, now);
            next.is_some()
        }
        _ => false,
    }
}

/// The mode a job runs in: what it asked for, else a full one when the user asked for the next
/// automatic run to be full, else the task's own mode.
fn requested_mode(job: &Job, task: &Task, full_next: bool) -> BackupMode {
    match job.mode {
        Some(m) => m,
        None if full_next && job.trigger != Trigger::Manual => BackupMode::Full,
        None => task.mode,
    }
}

/// One record for the whole run; status is the worst of its sources.
fn summarize(
    id: String,
    task: &Task,
    trigger: Trigger,
    mode: BackupMode,
    started_at: chrono::DateTime<Local>,
    sources: Vec<SourceRun>,
) -> RunRecord {
    let status = sources.iter().filter_map(|s| s.status).max().unwrap_or(RunStatus::Failed);
    let message = match sources.as_slice() {
        [] => "אין תיקיות מקור במשימה".to_string(),
        [one] => one.message.clone(),
        many => {
            let count = |st: RunStatus| many.iter().filter(|s| s.status == Some(st)).count();
            let mut parts = vec![format!("{} תיקיות", many.len())];
            for (st, label) in [
                (RunStatus::Success, "הצליחו"),
                (RunStatus::Warning, "עם אזהרות"),
                (RunStatus::Failed, "נכשלו"),
                (RunStatus::Cancelled, "בוטלו"),
            ] {
                let n = count(st);
                if n > 0 {
                    parts.push(format!("{n} {label}"));
                }
            }
            let copied: u64 = many.iter().map(|s| s.files_copied).sum();
            let mut msg = format!("{}. הועתקו {copied} קבצים", parts.join(", "));
            let size: u64 = many
                .iter()
                .filter(|s| s.mode == Some(BackupMode::Full) || s.files_copied > 0)
                .map(|s| s.backup_bytes)
                .sum();
            if size > 0 {
                msg.push_str(&format!(". גודל הגיבוי: {}", backup::fmt_bytes(size)));
            }
            let freed: u64 = many.iter().map(|s| s.freed_bytes).sum();
            if freed > 0 {
                msg.push_str(&format!(". פונו {} בכונן היעד", backup::fmt_bytes(freed)));
            }
            msg
        }
    };
    RunRecord {
        id,
        task_id: task.id.clone(),
        task_name: task.name.clone(),
        trigger,
        mode,
        started_at,
        finished_at: Local::now(),
        status,
        message,
        files_copied: sources.iter().map(|s| s.files_copied).sum(),
        bytes_copied: sources.iter().map(|s| s.bytes_copied).sum(),
        backup_bytes: sources.iter().map(|s| s.backup_bytes).sum(),
        freed_bytes: sources.iter().map(|s| s.freed_bytes).sum(),
        files_deleted: sources.iter().map(|s| s.files_deleted).sum(),
        files_failed: sources.iter().map(|s| s.files_failed).sum(),
        sources,
    }
}

/// A scheduled time this late is treated as missed (computer was off or asleep).
const MISSED_GRACE_MINUTES: i64 = 2;
const SCHEDULER_TICK: Duration = Duration::from_secs(5);
const DRIVE_TICK: Duration = Duration::from_secs(3);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub task_id: String,
    /// None = the task's own mode.
    pub mode: Option<BackupMode>,
    pub trigger: Trigger,
    /// Don't continue a paused or failed backup folder, start a new one (the user chose to).
    /// Scheduled runs never continue one either: they start the new cycle.
    pub fresh: bool,
}

pub struct Core {
    pub store: Mutex<Store>,
    queue: Mutex<VecDeque<Job>>,
    queue_cv: Condvar,
    pub current: Mutex<Option<Progress>>,
    cancel: AtomicBool,
    /// Set with `cancel` for a full stop: the half-done backup folder is deleted (a pause keeps it).
    discard: AtomicBool,
    /// No scan beside a full copy to find its totals: the setting is off, or the user gave up on
    /// it (for the rest of the run).
    skip_scan: AtomicBool,
    /// The worker took a job and hasn't finished it yet (set and cleared under the queue lock).
    working: AtomicBool,
    /// Tasks waiting for the user to answer "the drive was connected - back up now?".
    drive_prompts: Mutex<Vec<String>>,
    /// Backup folders that should have been deleted but weren't, until the user deals with them.
    delete_problems: Mutex<Vec<DeleteProblem>>,
    /// Where a clicked notification sends the window, until the window went there (`Snapshot.navigate`:
    /// state, not an event, so it survives a window that is still coming up).
    navigate: Mutex<Option<Navigate>>,
    engine: Box<dyn CopyEngine>,
    app: AppHandle,
}

impl Core {
    pub fn new(app: AppHandle, store: Store) -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(store),
            queue: Mutex::new(VecDeque::new()),
            queue_cv: Condvar::new(),
            current: Mutex::new(None),
            cancel: AtomicBool::new(false),
            discard: AtomicBool::new(false),
            skip_scan: AtomicBool::new(false),
            working: AtomicBool::new(false),
            drive_prompts: Mutex::new(Vec::new()),
            delete_problems: Mutex::new(Vec::new()),
            navigate: Mutex::new(None),
            engine: Box::new(Robocopy),
            app,
        })
    }

    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || me.worker_loop());
        let me = self.clone();
        std::thread::spawn(move || me.scheduler_loop());
        // Its own thread: checking a disconnected network drive letter can block for a while.
        let me = self.clone();
        std::thread::spawn(move || me.drive_loop());
        // Sources renamed while their drive wasn't connected, if it is now.
        let me = self.clone();
        std::thread::spawn(move || me.rename_backups_when_idle(None));
        // Whether admin work needs Windows's prompt (`elevation`): asks Windows, takes a moment.
        let me = self.clone();
        std::thread::spawn(move || {
            crate::elevation::refresh();
            me.changed();
        });
    }

    /// Tells the UI to refetch its snapshot.
    pub fn changed(&self) {
        let _ = self.app.emit("snapshot-changed", ());
        crate::tray::refresh(&self.app);
    }

    pub fn navigate(&self) -> Option<Navigate> {
        self.navigate.lock().unwrap().clone()
    }

    pub fn dismiss_navigate(&self) {
        *self.navigate.lock().unwrap() = None;
    }

    /// A notification was clicked: points the window at what it was about.
    fn follow(&self, target: &ToastTarget) {
        let navigate = match target {
            ToastTarget::Window | ToastTarget::Update => return,
            ToastTarget::Tasks => Navigate { view: "tasks".into(), run_id: None },
            ToastTarget::Settings => Navigate { view: "settings".into(), run_id: None },
            ToastTarget::Run(id) => Navigate { view: "history".into(), run_id: Some(id.clone()) },
        };
        *self.navigate.lock().unwrap() = Some(navigate);
        self.changed();
    }

    pub fn queued(&self) -> Vec<Job> {
        self.queue.lock().unwrap().iter().cloned().collect()
    }

    /// Returns false if the task is already queued or running.
    pub fn enqueue(&self, job: Job) -> bool {
        // A task imported from Cobian waits for the user's answer about Cobian's existing backups:
        // a first run now would write a full backup next to them instead of continuing from them.
        if self
            .store
            .lock()
            .unwrap()
            .tasks
            .iter()
            .any(|t| t.id == job.task_id && t.cobian_pending)
        {
            return false;
        }
        let running = self.current.lock().unwrap().as_ref().map(|p| p.task_id.clone());
        let mut q = self.queue.lock().unwrap();
        if running.as_deref() == Some(job.task_id.as_str()) {
            return false;
        }
        if let Some(queued) = q.iter_mut().find(|j| j.task_id == job.task_id) {
            // A full backup that comes due replaces a queued incremental of the same task.
            if job.mode == Some(BackupMode::Full) && queued.mode != Some(BackupMode::Full) {
                queued.mode = Some(BackupMode::Full);
                drop(q);
                self.changed();
                return true;
            }
            return false;
        }
        q.push_back(job);
        drop(q);
        self.queue_cv.notify_one();
        self.changed();
        true
    }

    pub fn drive_prompts(&self) -> Vec<String> {
        self.drive_prompts.lock().unwrap().clone()
    }

    /// The user's answer to the drive prompt: `run` gets queued, all of `ids` stop waiting.
    pub fn answer_drive_prompts(&self, ids: &[String], run: &[String]) {
        self.drive_prompts.lock().unwrap().retain(|id| !ids.contains(id));
        for id in run {
            self.enqueue(Job {
                task_id: id.clone(),
                mode: None,
                trigger: Trigger::DriveConnected,
                fresh: false,
            });
        }
        self.changed();
    }

    pub fn delete_problems(&self) -> Vec<DeleteProblem> {
        self.delete_problems.lock().unwrap().clone()
    }

    /// A backup folder of `task` couldn't be deleted. Told right away - a notification and the main
    /// window with the question - since the folder keeps taking room the next backups need.
    pub fn report_delete_problem(&self, task: &Task, path: &std::path::Path, e: &std::io::Error) {
        self.report_problem(task, path, e, false)
    }

    /// `blocking`: the running backup waits for the user's answer (`await_delete_problems`).
    fn report_problem(&self, task: &Task, path: &std::path::Path, e: &std::io::Error, blocking: bool) {
        // With permanent admin permission, try as admin first, unasked (nobody may be at the computer).
        if crate::elevation::cached() == crate::model::AdminPermission::On {
            let p = path.to_string_lossy().to_string();
            if crate::acl::delete_elevated(std::slice::from_ref(&p)).is_ok() && !path.exists() {
                return;
            }
        }
        let problem = DeleteProblem {
            blocking,
            task_id: task.id.clone(),
            task_name: task.name.clone(),
            path: path.to_string_lossy().to_string(),
            reason: DeleteProblem::reason_of(e).into(),
            message: e.to_string(),
            at: Local::now(),
        };
        {
            let mut list = self.delete_problems.lock().unwrap();
            list.retain(|p| p.path != problem.path);
            list.push(problem);
        }
        let sound = self.store.lock().unwrap().settings.sound_failure.clone();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        toast(&self.app, format!("גיבוי ישן לא נמחק: {}", task.name), format!("{name} - הוא ממשיך לתפוס מקום בכונן הגיבוי. פתחו את Backuper כדי לטפל בזה"), &sound, ToastTarget::Window);
        self.changed();
        crate::tray::show_main(&self.app);
    }

    /// Holds the running backup of `task_id` while a blocking report of it is open: the user deletes
    /// it, or skips the delete (`dismiss_delete_problems`), or cancels the run.
    fn await_delete_problems(&self, task_id: &str) {
        loop {
            let waiting = self.delete_problems.lock().unwrap().iter().any(|p| p.blocking && p.task_id == task_id);
            if !waiting || self.cancel.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        // Nothing waits on them any more (a cancelled run, or the user's answer).
        let mut list = self.delete_problems.lock().unwrap();
        let mut changed = false;
        for p in list.iter_mut().filter(|p| p.blocking && p.task_id == task_id) {
            p.blocking = false;
            changed = true;
        }
        drop(list);
        if changed {
            self.changed();
        }
    }

    /// The user closed the question without deleting (a waiting backup goes on without the delete).
    pub fn dismiss_delete_problems(&self, paths: &[String]) {
        self.delete_problems.lock().unwrap().retain(|p| !paths.contains(&p.path));
        self.changed();
    }

    /// Tries again to delete reported folders (only those: nothing else can be handed in), `elevated`
    /// = as admin, after Windows's permission prompt. Returns the paths that are still there.
    pub fn retry_delete(&self, paths: &[String], elevated: bool) -> Result<Vec<String>, String> {
        let known: Vec<String> = self
            .delete_problems
            .lock()
            .unwrap()
            .iter()
            .filter(|p| paths.contains(&p.path))
            .map(|p| p.path.clone())
            .collect();
        let mut failures: Vec<(String, std::io::Error)> = Vec::new();
        let result = if elevated {
            crate::acl::delete_elevated(&known)
        } else {
            for p in &known {
                if let Err(e) = crate::engine::native::remove_tree(std::path::Path::new(p)) {
                    failures.push((p.clone(), e));
                }
            }
            Ok(())
        };
        let left: Vec<String> = known.iter().filter(|p| std::path::Path::new(p).exists()).cloned().collect();
        {
            let mut list = self.delete_problems.lock().unwrap();
            list.retain(|p| !known.contains(&p.path) || left.contains(&p.path));
            for p in list.iter_mut() {
                if let Some((_, e)) = failures.iter().find(|(f, _)| *f == p.path) {
                    p.reason = DeleteProblem::reason_of(e).into();
                    p.message = e.to_string();
                } else if elevated && left.contains(&p.path) {
                    // Even admin couldn't: most likely a program still has a file open.
                    p.reason = "inUse".into();
                }
            }
        }
        self.changed();
        result.map(|_| left)
    }

    /// "Back up the open files now" on a finished run's source: the files that failed because another
    /// program held them open are copied from a shadow copy into that run's backup folder (Windows
    /// asks for admin permission). Fixes the run's record, the folder's index and the task's last
    /// status. Returns the updated record.
    pub fn backup_open_files(&self, run_id: &str, source_index: usize) -> Result<RunRecord, String> {
        let busy = "גיבוי רץ או ממתין בתור כרגע - נסו שוב כשיסתיים";
        let record = self.store.lock().unwrap().history.iter().find(|r| r.id == run_id).cloned().ok_or("הריצה לא נמצאה ביומן")?;
        let src = record.sources.get(source_index).ok_or("המקור לא נמצא בריצה")?.clone();
        let folder = PathBuf::from(src.target_folder.clone().ok_or("לריצה הזאת אין תיקיית גיבוי")?);
        if !folder.is_dir() {
            return Err("תיקיית הגיבוי של הריצה הזאת כבר לא קיימת".into());
        }
        let mut errors = src.errors.clone();
        let never = AtomicBool::new(false);
        let saved = self
            .when_idle(|| crate::vss::save_open_files(Path::new(&src.source), &folder, &mut errors, None, &never))
            .ok_or(busy)?;
        if saved.files == 0 {
            return Err(saved.problem.unwrap_or_else(|| "לא נמצאו קבצים פתוחים לגבות".into()));
        }
        crate::manifest::add(&folder, &saved.listed);
        let mut s = self.store.lock().unwrap();
        let Some(rec) = s.history.iter_mut().find(|r| r.id == run_id) else {
            return Err("הריצה לא נמצאה ביומן".into());
        };
        {
            let run = &mut rec.sources[source_index];
            run.errors = errors;
            run.files_copied += saved.files;
            run.bytes_copied += saved.bytes;
            run.backup_bytes += saved.bytes;
            run.files_failed = run.files_failed.saturating_sub(saved.files);
            if run.status == Some(RunStatus::Warning) && run.files_failed == 0 && run.errors.is_empty() {
                run.status = Some(RunStatus::Success);
                run.message = format!("הועתקו {} קבצים, מהם {} שהיו פתוחים בתוכנה אחרת וגובו אחר כך דרך VSS", run.files_copied, saved.files);
            } else {
                run.message = format!("{}. {} קבצים שהיו פתוחים בתוכנה אחרת גובו אחר כך דרך VSS", run.message, saved.files);
            }
        }
        rec.status = rec.sources.iter().filter_map(|s| s.status).max().unwrap_or(rec.status);
        rec.files_copied = rec.sources.iter().map(|s| s.files_copied).sum();
        rec.bytes_copied = rec.sources.iter().map(|s| s.bytes_copied).sum();
        rec.backup_bytes = rec.sources.iter().map(|s| s.backup_bytes).sum();
        rec.files_failed = rec.sources.iter().map(|s| s.files_failed).sum();
        rec.message = match rec.sources.as_slice() {
            [one] => one.message.clone(),
            _ => format!("{}. {} קבצים שהיו פתוחים בתוכנה אחרת גובו אחר כך דרך VSS", rec.message, saved.files),
        };
        let rec = rec.clone();
        // The task's last run is this one: its status (and the tray's warning dot) follow.
        if let Some(st) = s.states.get_mut(&rec.task_id) {
            if st.last_run_at == Some(rec.finished_at) {
                st.last_status = Some(rec.status);
                st.last_message = Some(rec.message.clone());
            }
        }
        s.save_history();
        s.save_states();
        drop(s);
        self.changed();
        Ok(rec)
    }

    /// Cancels a queued or running task. A running one is paused (its folder is kept, the next run
    /// continues in it), or with `discard` stopped for good.
    pub fn cancel(&self, task_id: &str, discard: bool) {
        self.queue.lock().unwrap().retain(|j| j.task_id != task_id);
        if self.current.lock().unwrap().as_ref().is_some_and(|p| p.task_id == task_id) {
            self.discard.store(discard, Ordering::SeqCst);
            self.cancel.store(true, Ordering::SeqCst);
        }
        self.changed();
    }

    /// Gives up on a paused backup so it leaves the "paused" list: with `delete` its partial folders
    /// are removed too (the ones that are unsafe to delete stay, and the task stays paused). Does
    /// nothing while the task runs or waits in the queue.
    pub fn dismiss_paused(&self, task_id: &str, delete: bool) -> Result<(), String> {
        let busy = self.queue.lock().unwrap().iter().any(|j| j.task_id == task_id)
            || self.current.lock().unwrap().as_ref().is_some_and(|p| p.task_id == task_id);
        if busy {
            return Err("המשימה רצה או ממתינה בתור".into());
        }
        let task = self.store.lock().unwrap().tasks.iter().find(|t| t.id == task_id).cloned();
        let Some(task) = task else { return Ok(()) };
        if delete && backup::discard_partials(&task, &|p, e| self.report_delete_problem(&task, p, e)) > 0 {
            return Err("חלק מההעתקה החלקית לא נמחק: זה הגיבוי המלא היחיד שיש, או שהתיקייה בשימוש. אפשר להסיר את ההשהיה בלי למחוק".into());
        }
        let mut s = self.store.lock().unwrap();
        if let Some(st) = s.states.get_mut(task_id) {
            st.resumable = false;
        }
        s.save_states();
        drop(s);
        self.changed();
        Ok(())
    }

    /// Makes the task's next automatic run (scheduled, catch-up, drive connected) a full backup,
    /// once; `on = false` takes the request back.
    pub fn set_full_next(&self, task_id: &str, on: bool) {
        let mut s = self.store.lock().unwrap();
        if !s.tasks.iter().any(|t| t.id == task_id) {
            return;
        }
        s.states.entry(task_id.to_string()).or_default().full_next = on;
        s.save_states();
        drop(s);
        self.changed();
    }

    /// The user opened the details of the task's last run: its warnings are known, so the tray
    /// icon drops its dot for it (until the next run).
    pub fn mark_status_seen(&self, task_id: &str) {
        let mut s = self.store.lock().unwrap();
        match s.states.get_mut(task_id) {
            Some(st) if !st.status_seen => st.status_seen = true,
            _ => return,
        }
        s.save_states();
        drop(s);
        self.changed();
    }

    /// Stops the scan beside a running full copy that finds how much there is to copy: the copy
    /// carries on with counters only (and the estimate from the last backup, if it had one).
    pub fn skip_totals_scan(&self) {
        self.skip_scan.store(true, Ordering::SeqCst);
    }

    /// Renames the backup folders of sources whose folder name changed (`Source::previous_names`)
    /// in the tasks `only` (None = all), if no backup is running or waiting; otherwise the worker
    /// does it before the task's next run. Called on save, at startup and when a drive connects.
    pub fn rename_backups_when_idle(&self, only: Option<&[String]>) {
        let tasks: Vec<Task> = self
            .store
            .lock()
            .unwrap()
            .tasks
            .iter()
            .filter(|t| only.is_none_or(|ids| ids.contains(&t.id)))
            .filter(|t| t.sources.iter().any(|s| !s.previous_names.is_empty()))
            .cloned()
            .collect();
        if tasks.is_empty() {
            return;
        }
        // Folders only: the store is locked after the queue is released (get_snapshot locks them the other way round).
        if let Some(done) = self.when_idle(|| tasks.iter().map(rename_task_backups).collect::<Vec<_>>()) {
            for (task, results) in tasks.iter().zip(done) {
                self.apply_renamed(task, &results);
            }
        }
    }

    /// Drops the previous names that have no folders left, keeping any added since `task` was read.
    fn apply_renamed(&self, task: &Task, results: &[backup::Renamed]) {
        let mut changed = false;
        {
            let mut s = self.store.lock().unwrap();
            let Some(cur) = s.tasks.iter_mut().find(|t| t.id == task.id) else { return };
            for (src, r) in task.sources.iter().zip(results) {
                let Some(c) = cur.sources.iter_mut().find(|c| c.path == src.path && c.folder_name == src.folder_name) else {
                    continue;
                };
                let before = c.previous_names.len();
                c.previous_names.retain(|n| !src.previous_names.contains(n) || r.left.contains(n));
                changed |= c.previous_names.len() != before;
            }
            if changed {
                s.save_tasks();
            }
        }
        if changed || results.iter().any(|r| r.count > 0) {
            self.changed();
        }
    }

    /// Runs `f` only if no backup is running or queued, holding the queue so none can start meanwhile.
    pub fn when_idle<R>(&self, f: impl FnOnce() -> R) -> Option<R> {
        let q = self.queue.lock().unwrap();
        if !q.is_empty() || self.working.load(Ordering::SeqCst) {
            return None;
        }
        let r = f();
        drop(q);
        Some(r)
    }

    fn next_job(&self) -> Job {
        let mut q = self.queue.lock().unwrap();
        // Asking for the next job means the previous one is done.
        self.working.store(false, Ordering::SeqCst);
        loop {
            if let Some(j) = q.pop_front() {
                self.working.store(true, Ordering::SeqCst);
                return j;
            }
            q = self.queue_cv.wait(q).unwrap();
        }
    }

    fn worker_loop(&self) {
        loop {
            let job = self.next_job();
            let (task, logs_dir, global_filters, exact_progress, background_totals, expected_rate, full_next, estimates) = {
                let s = self.store.lock().unwrap();
                (
                    s.task(&job.task_id).cloned(),
                    s.logs_dir(),
                    s.settings.global_filters.clone(),
                    s.settings.exact_progress,
                    s.settings.background_totals,
                    backup::typical_rate(&s.history, &job.task_id),
                    s.states.get(&job.task_id).is_some_and(|st| st.full_next),
                    s.task(&job.task_id)
                        .map(|t| {
                            let full_next = s.states.get(&job.task_id).is_some_and(|st| st.full_next);
                            backup::source_estimates(&s.history, t, requested_mode(&job, t, full_next))
                        })
                        .unwrap_or_default(),
                )
            };
            let Some(task) = task else { continue };
            // A source renamed while its drive wasn't connected: its backups get the new name first,
            // so the run continues their chain.
            let renamed = rename_task_backups(&task);
            self.apply_renamed(&task, &renamed);
            let run_id = uuid::Uuid::new_v4().to_string();
            let requested = requested_mode(&job, &task, full_next);
            let later_bytes = |i: usize| estimates.iter().skip(i + 1).copied().sum::<Option<u64>>();
            let started_at = Local::now();
            if task.cobian_takeover {
                if let Err(message) = self.take_over_cobian(&task) {
                    let record = RunRecord {
                        id: run_id,
                        task_id: task.id.clone(),
                        task_name: task.name.clone(),
                        trigger: job.trigger,
                        mode: requested,
                        started_at,
                        finished_at: Local::now(),
                        status: RunStatus::Failed,
                        message,
                        files_copied: 0,
                        bytes_copied: 0,
                        backup_bytes: 0,
                        freed_bytes: 0,
                        files_deleted: 0,
                        files_failed: 0,
                        sources: Vec::new(),
                    };
                    self.finish_run(&task, record);
                    continue;
                }
            }
            let mut progress = Progress {
                run_id: run_id.clone(),
                task_id: task.id.clone(),
                task_name: task.name.clone(),
                mode: requested,
                phase: "scanning".into(),
                started_at,
                source_index: 1,
                source_count: task.sources.len(),
                source_path: task.sources.first().map(|s| s.path.clone()).unwrap_or_default(),
                files_done: 0,
                files_total: 0,
                bytes_done: 0,
                bytes_total: 0,
                totals: "unknown".into(),
                totals_scan: false,
                copy_started_at: None,
                expected_rate,
                later_bytes: later_bytes(0),
                earlier_bytes: 0,
                current_file: String::new(),
            };
            self.cancel.store(false, Ordering::SeqCst);
            self.discard.store(false, Ordering::SeqCst);
            // The setting is the starting point; the user can still skip during the run.
            self.skip_scan.store(!background_totals, Ordering::SeqCst);
            *self.current.lock().unwrap() = Some(progress.clone());
            self.changed();

            let filters = crate::filters::compile(
                task.filters
                    .iter()
                    .chain(global_filters.iter().filter(|_| task.use_global_filters)),
            );
            let ctx = backup::RunContext {
                engine: self.engine.as_ref(),
                filters: &filters,
                now: started_at,
                cancel: &self.cancel,
                discard: &self.discard,
                skip_scan: &self.skip_scan,
                fresh: job.fresh,
                scheduled: matches!(job.trigger, Trigger::Scheduled | Trigger::CatchUp),
                exact_progress,
                delete_failed: &|p, e, blocking| self.report_problem(&task, p, e, blocking),
                await_deletes: &|| self.await_delete_problems(&task.id),
            };
            // The bytes of the files finished so far, and the size of the one in progress (counted when it ends).
            let mut done_before = 0u64;
            let mut pending_size = 0u64;
            let mut last_emit = Instant::now();
            let sources = backup::run_task(
                &ctx,
                &task,
                requested,
                &|i| logs_dir.join(format!("{run_id}-{i}.log")),
                &mut |i, ev| {
                    let mut force = false;
                    match ev {
                        EngineEvent::SourceStarted => {
                            progress.source_index = i + 1;
                            progress.source_path = task.sources[i].path.clone();
                            progress.later_bytes = later_bytes(i);
                            // The last file of the previous source counts only now.
                            progress.earlier_bytes += done_before + pending_size;
                            progress.phase = "scanning".into();
                            (
                                progress.files_done,
                                progress.files_total,
                                progress.bytes_done,
                                progress.bytes_total,
                            ) = (0, 0, 0, 0);
                            progress.totals = "unknown".into();
                            progress.totals_scan = false;
                            progress.copy_started_at = None;
                            progress.current_file.clear();
                            pending_size = 0;
                            done_before = 0;
                            force = true;
                        }
                        EngineEvent::Phase(p) => {
                            progress.phase = p.into();
                            if p == "vss" && crate::elevation::cached() != crate::model::AdminPermission::On {
                                // Windows is about to ask for admin permission, possibly while the window is hidden
                                // in the tray: say so now, not when the run is over. A scheduled run (nobody may be
                                // there) only toasts; the window stays hidden.
                                let sound = self.store.lock().unwrap().settings.sound_failure.clone();
                                toast(
                                    &self.app,
                                    format!("נדרש אישור מנהל: {}", task.name),
                                    "קבצים פתוחים בתוכנה אחרת (כמו Outlook) מגובים דרך VSS. אשרו את בקשת ההרשאה של Windows, אחרת הם ידולגו".to_string(),
                                    &sound,
                                    ToastTarget::Tasks,
                                );
                                if !matches!(job.trigger, Trigger::Scheduled | Trigger::CatchUp) {
                                    crate::tray::show_main(&self.app);
                                }
                            }
                            if p == "copying" {
                                progress.copy_started_at.get_or_insert_with(Local::now);
                            }
                            force = true;
                        }
                        EngineEvent::TotalsPending => {
                            progress.totals_scan = true;
                            force = true;
                        }
                        EngineEvent::TotalsUnavailable => {
                            progress.totals_scan = false;
                            force = true;
                        }
                        EngineEvent::Totals { files, bytes, exact } => {
                            progress.files_total = files;
                            progress.bytes_total = bytes;
                            progress.totals = if exact { "exact" } else { "estimate" }.into();
                            if exact {
                                progress.totals_scan = false;
                            }
                            progress.phase = "copying".into();
                            progress.copy_started_at.get_or_insert_with(Local::now);
                            force = true;
                        }
                        EngineEvent::File { path, size } => {
                            done_before += pending_size;
                            progress.bytes_done = done_before;
                            pending_size = size;
                            progress.files_done += 1;
                            progress.current_file = path;
                        }
                        EngineEvent::FileBytes(n) => progress.bytes_done = done_before + n.min(pending_size),
                    }
                    if force || last_emit.elapsed() >= Duration::from_millis(200) {
                        last_emit = Instant::now();
                        *self.current.lock().unwrap() = Some(progress.clone());
                        let _ = self.app.emit("progress", &progress);
                    }
                },
            );

            let mut sources = sources;
            for (run, r) in sources.iter_mut().zip(&renamed) {
                if r.problems.is_empty() {
                    continue;
                }
                run.message = format!("{}. {}", run.message, r.problems.join(". "));
                if run.status == Some(RunStatus::Success) {
                    run.status = Some(RunStatus::Warning);
                }
            }
            let record = summarize(run_id, &task, job.trigger, requested, started_at, sources);
            self.finish_run(&task, record);
        }
    }

    /// Saves a finished run (state, history), clears the progress and notifies.
    fn finish_run(&self, task: &Task, record: RunRecord) {
        let settings = {
            let mut s = self.store.lock().unwrap();
            let st = s.states.entry(task.id.clone()).or_default();
            st.last_run_at = Some(record.finished_at);
            st.last_status = Some(record.status);
            st.last_message = Some(record.message.clone());
            st.status_seen = false;
            // A paused run left a folder to continue in; a full stop deleted it.
            st.resumable = record.status == RunStatus::Cancelled && backup::has_partial(task);
            if let Some(b) = crate::model::LastBackup::of(&record) {
                // The full backup the user asked for is done.
                if b.mode == BackupMode::Full {
                    st.full_next = false;
                }
                st.last_backup = Some(b);
            }
            s.save_states();
            s.add_history(record.clone());
            s.settings.clone()
        };
        *self.current.lock().unwrap() = None;
        self.changed();
        self.notify(&record, &settings);
    }

    /// The user chose to continue from Cobian's backups: they get this app's names now, right
    /// before the task's first run here. Every rename is recorded (to put the names back later).
    /// On an error nothing is backed up (that would write a full backup beside Cobian's).
    fn take_over_cobian(&self, task: &Task) -> Result<(), String> {
        let all = self.store.lock().unwrap().tasks.clone();
        let r = crate::cobianmigrate::apply(task, &all);
        let mut s = self.store.lock().unwrap();
        s.record_cobian_renames(task, &r.done);
        if !r.errors.is_empty() {
            let shown: Vec<&str> = r.errors.iter().take(3).map(String::as_str).collect();
            return Err(format!(
                "לא ניתן היה לשנות את השמות של {} מהגיבויים של Cobian ({}), ולכן הגיבוי לא התחיל. סגרו את Cobian ואת סייר הקבצים אם הם פתוחים על התיקיות האלה, והריצו את המשימה שוב",
                r.errors.len(),
                shown.join("; ")
            ));
        }
        if let Some(t) = s.tasks.iter_mut().find(|t| t.id == task.id) {
            t.cobian_takeover = false;
            s.save_tasks();
        }
        Ok(())
    }

    fn notify(&self, r: &RunRecord, settings: &Settings) {
        let (show, title, sound) = match r.status {
            RunStatus::Success => (settings.notify_success, format!("הגיבוי הצליח: {}", r.task_name), &settings.sound_success),
            RunStatus::Warning => (
                settings.notify_success || settings.notify_failure,
                format!("הגיבוי הסתיים עם אזהרות: {}", r.task_name),
                &settings.sound_failure,
            ),
            RunStatus::Failed => (settings.notify_failure, format!("הגיבוי נכשל: {}", r.task_name), &settings.sound_failure),
            RunStatus::Cancelled => (false, String::new(), &settings.sound_success),
        };
        if show {
            toast(&self.app, title, &r.message, sound, ToastTarget::Run(r.id.clone()));
        }
    }

    /// Fills in missing next-run times and clears them for disabled/manual tasks.
    pub fn refresh_next_runs(&self) {
        let now = Local::now();
        let mut s = self.store.lock().unwrap();
        let tasks = s.tasks.clone();
        let mut dirty = false;
        for t in &tasks {
            let st = s.states.entry(t.id.clone()).or_default();
            dirty |= refresh_slot(&mut st.next_run, t.enabled.then_some(&t.schedule), now);
            dirty |= refresh_slot(&mut st.next_full_run, t.full_schedule.as_ref().filter(|_| t.enabled), now);
        }
        if dirty {
            s.save_states();
        }
    }

    /// Starts tasks (or asks about them) when their drives get connected.
    fn drive_loop(&self) {
        let mut watch = DriveWatch::default();
        loop {
            let (tasks, paused, notify, sound) = {
                let s = self.store.lock().unwrap();
                let st = &s.settings;
                (s.tasks.clone(), st.scheduler_paused, st.notify_success, st.sound_success.clone())
            };
            // Polled even while paused, so unpausing doesn't fire for drives connected long ago.
            let connected = watch.poll(&tasks);
            // Even for tasks that are off: a renamed source's backups get the new name as soon as
            // its drive is back (before a backup is queued below, which would hold the rename off).
            let rename: Vec<String> = connected
                .iter()
                .filter(|t| t.sources.iter().any(|s| !s.previous_names.is_empty()))
                .map(|t| t.id.clone())
                .collect();
            if !rename.is_empty() {
                self.rename_backups_when_idle(Some(&rename));
            }
            let ready: Vec<Task> = connected.into_iter().filter(|t| t.enabled && !paused).cloned().collect();
            let mut changed = false;
            {
                // A question about a task that's gone, turned off, or whose drive was unplugged again is moot.
                let mut prompts = self.drive_prompts.lock().unwrap();
                let before = prompts.len();
                prompts.retain(|id| {
                    tasks
                        .iter()
                        .find(|t| t.id == *id)
                        .is_some_and(|t| t.enabled && t.on_drive_connect == DriveAction::Ask && watch.all_present(t))
                });
                changed |= prompts.len() != before;
            }
            let mut ask = false;
            for t in &ready {
                let drives = crate::drives::task_drives(t).join(", ");
                match t.on_drive_connect {
                    DriveAction::Run => {
                        let queued = self.enqueue(Job {
                            task_id: t.id.clone(),
                            mode: None,
                            trigger: Trigger::DriveConnected,
                            fresh: false,
                        });
                        if queued && notify {
                            toast(&self.app, format!("הכונן חובר - מתחיל גיבוי: {}", t.name), drives, &sound, ToastTarget::Tasks);
                        }
                    }
                    DriveAction::Ask => {
                        let busy = self.queued().iter().any(|j| j.task_id == t.id)
                            || self.current.lock().unwrap().as_ref().is_some_and(|p| p.task_id == t.id);
                        let mut prompts = self.drive_prompts.lock().unwrap();
                        if !busy && !prompts.contains(&t.id) {
                            prompts.push(t.id.clone());
                            ask = true;
                        }
                    }
                    DriveAction::Off => {}
                }
            }
            if ask || changed {
                self.changed();
            }
            if ask {
                // The question is a dialog in the main window, so bring it up (it may be in the tray).
                crate::tray::show_main(&self.app);
            }
            std::thread::sleep(DRIVE_TICK);
        }
    }

    fn scheduler_loop(&self) {
        loop {
            self.refresh_next_runs();
            let now = Local::now();
            let mut due = Vec::new();
            {
                let mut s = self.store.lock().unwrap();
                if !s.settings.scheduler_paused {
                    let tasks = s.tasks.clone();
                    let mut dirty = false;
                    for t in tasks.iter().filter(|t| t.enabled) {
                        let st = s.states.entry(t.id.clone()).or_default();
                        let main = take_due(&mut st.next_run, &t.schedule, now, t.catch_up);
                        let full = match &t.full_schedule {
                            Some(fs) => take_due(&mut st.next_full_run, fs, now, t.catch_up),
                            None => Slot::Idle,
                        };
                        dirty |= main != Slot::Idle || full != Slot::Idle;
                        // When both come due together, the full backup wins.
                        if let Slot::Run(trigger) = full {
                            due.push(Job {
                                task_id: t.id.clone(),
                                mode: Some(BackupMode::Full),
                                trigger,
                                fresh: false,
                            });
                        } else if let Slot::Run(trigger) = main {
                            due.push(Job {
                                task_id: t.id.clone(),
                                mode: None,
                                trigger,
                                fresh: false,
                            });
                        }
                    }
                    if dirty {
                        s.save_states();
                    }
                }
            }
            let any = !due.is_empty();
            for job in due {
                self.enqueue(job);
            }
            if any {
                self.changed();
            }
            std::thread::sleep(SCHEDULER_TICK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(h: u32, m: u32) -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 2, h, m, 0).unwrap()
    }

    #[test]
    fn slots_run_skip_and_advance() {
        let daily = Schedule::Daily { time: "03:00".into() };
        let mut next = Some(at(3, 0));
        assert_eq!(take_due(&mut next, &daily, at(2, 59), true), Slot::Idle);
        assert_eq!(take_due(&mut next, &daily, at(3, 0), true), Slot::Run(Trigger::Scheduled));
        assert_eq!(next, Some(Local.with_ymd_and_hms(2026, 10, 3, 3, 0, 0).unwrap()));

        // Missed by hours (PC was off): catch-up runs it, otherwise it's skipped.
        let mut next = Some(at(3, 0));
        assert_eq!(take_due(&mut next, &daily, at(9, 0), true), Slot::Run(Trigger::CatchUp));
        let mut next = Some(at(3, 0));
        assert_eq!(take_due(&mut next, &daily, at(9, 0), false), Slot::Skipped);
        assert!(next.unwrap() > at(9, 0));
    }

    #[test]
    fn full_next_turns_automatic_runs_full() {
        let task = Task { mode: BackupMode::Incremental, ..Default::default() };
        let job = |trigger, mode| Job { task_id: String::new(), mode, trigger, fresh: false };
        assert_eq!(requested_mode(&job(Trigger::Scheduled, None), &task, false), BackupMode::Incremental);
        for t in [Trigger::Scheduled, Trigger::CatchUp, Trigger::DriveConnected] {
            assert_eq!(requested_mode(&job(t, None), &task, true), BackupMode::Full);
        }
        // A manual run does what the user picked there.
        assert_eq!(requested_mode(&job(Trigger::Manual, None), &task, true), BackupMode::Incremental);
        assert_eq!(
            requested_mode(&job(Trigger::Scheduled, Some(BackupMode::Incremental)), &task, true),
            BackupMode::Incremental
        );
    }

    #[test]
    fn slot_refresh_follows_schedule() {
        let mut next = None;
        assert!(refresh_slot(
            &mut next,
            Some(&Schedule::Daily { time: "03:00".into() }),
            at(1, 0)
        ));
        assert_eq!(next, Some(at(3, 0)));
        assert!(refresh_slot(&mut next, Some(&Schedule::Manual), at(1, 0)));
        assert_eq!(next, None);
        assert!(!refresh_slot(&mut next, None, at(1, 0)));
    }
}

/// Where a click on a Windows notification leads. Every one of them brings the main window up.
#[derive(Clone, Debug, PartialEq)]
pub enum ToastTarget {
    /// Just the window (what the notification is about shows there as a dialog, or needs no place).
    Window,
    /// The task list, with the running backup on top.
    Tasks,
    /// The details of this run in the log.
    Run(String),
    /// The settings page (updates card).
    Settings,
    /// The update dialog.
    Update,
}

/// A notification was clicked: brings the window up, at what the notification was about.
pub fn activated(app: &AppHandle, target: &ToastTarget) {
    if let Some(core) = app.try_state::<Arc<Core>>() {
        core.follow(target);
    }
    if *target == ToastTarget::Update {
        if let Some(updates) = app.try_state::<Arc<crate::updater::Updates>>() {
            updates.request_prompt();
        }
    }
    crate::tray::show_main(app);
}

/// The app was started again while it runs (second instance): by the user, or by a click on a
/// notification (`args` holds its `backuper://` address, `protocol`).
pub fn reopened(app: &AppHandle, args: &[String]) {
    match crate::protocol::from_args(args) {
        Some(target) => activated(app, &target),
        None => crate::tray::show_main(app),
    }
}

/// Shows a Windows notification. `sound` is a toast sound name; empty plays nothing. A click on it
/// opens the main window at `target`.
pub fn toast(app: &AppHandle, title: impl Into<String>, body: impl Into<String>, sound: &str, target: ToastTarget) {
    let (title, body) = (title.into(), body.into());
    #[cfg(windows)]
    {
        use tauri_winrt_notification::{Sound, Toast};
        // Windows only shows notifications of an app id it knows: ours once installed (the
        // installer's shortcut carries it), PowerShell's when run from a build folder. Ours open
        // `backuper://` on a click, the only way a click in the notification center reaches us;
        // under PowerShell's id that address may not be this copy, so a dev build keeps the
        // in-process handler (pop-up clicks only).
        let shown = if crate::protocol::from_build_folder() {
            let handle = app.clone();
            Toast::new(Toast::POWERSHELL_APP_ID)
                .title(&title)
                .text1(&body)
                .sound(sound.parse::<Sound>().ok())
                .on_activated(move |_| {
                    activated(&handle, &target);
                    Ok(())
                })
                .show()
                .is_ok()
        } else {
            crate::protocol::show(&app.config().identifier, &title, &body, sound, &target).is_ok()
        };
        if shown {
            return;
        }
    }
    let mut n = app.notification().builder().title(title).body(body);
    if !sound.is_empty() {
        n = n.sound(sound);
    }
    let _ = n.show();
}
