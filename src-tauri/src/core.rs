//! The running core: a single worker that executes queued backups one at a time,
//! and the built-in scheduler that enqueues due tasks. UI-independent apart from events.

use crate::backup;
use crate::drives::DriveWatch;
use crate::engine::{robocopy::Robocopy, CopyEngine, EngineEvent};
use crate::model::{BackupMode, DriveAction, Progress, RunRecord, RunStatus, Schedule, Settings, SourceRun, Task, Trigger};
use crate::schedule;
use crate::store::Store;
use chrono::{Duration as ChronoDuration, Local};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

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
}

pub struct Core {
    pub store: Mutex<Store>,
    queue: Mutex<VecDeque<Job>>,
    queue_cv: Condvar,
    pub current: Mutex<Option<Progress>>,
    cancel: AtomicBool,
    /// The worker took a job and hasn't finished it yet (set and cleared under the queue lock).
    working: AtomicBool,
    /// Tasks waiting for the user to answer "the drive was connected - back up now?".
    drive_prompts: Mutex<Vec<String>>,
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
            working: AtomicBool::new(false),
            drive_prompts: Mutex::new(Vec::new()),
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
    }

    /// Tells the UI to refetch its snapshot.
    pub fn changed(&self) {
        let _ = self.app.emit("snapshot-changed", ());
        crate::tray::refresh(&self.app);
    }

    pub fn queued(&self) -> Vec<Job> {
        self.queue.lock().unwrap().iter().cloned().collect()
    }

    /// Returns false if the task is already queued or running.
    pub fn enqueue(&self, job: Job) -> bool {
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
            });
        }
        self.changed();
    }

    /// Cancels a queued or running task.
    pub fn cancel(&self, task_id: &str) {
        self.queue.lock().unwrap().retain(|j| j.task_id != task_id);
        if self.current.lock().unwrap().as_ref().is_some_and(|p| p.task_id == task_id) {
            self.cancel.store(true, Ordering::SeqCst);
        }
        self.changed();
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
            let (task, logs_dir, global_filters) = {
                let s = self.store.lock().unwrap();
                (s.task(&job.task_id).cloned(), s.logs_dir(), s.settings.global_filters.clone())
            };
            let Some(task) = task else { continue };
            let run_id = uuid::Uuid::new_v4().to_string();
            let requested = job.mode.unwrap_or(task.mode);
            let started_at = Local::now();
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
                current_file: String::new(),
            };
            self.cancel.store(false, Ordering::SeqCst);
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
            };
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
                            progress.phase = "scanning".into();
                            (
                                progress.files_done,
                                progress.files_total,
                                progress.bytes_done,
                                progress.bytes_total,
                            ) = (0, 0, 0, 0);
                            progress.current_file.clear();
                            pending_size = 0;
                            force = true;
                        }
                        EngineEvent::Phase(p) => {
                            progress.phase = p.into();
                            force = true;
                        }
                        EngineEvent::Totals { files, bytes } => {
                            progress.files_total = files;
                            progress.bytes_total = bytes;
                            progress.phase = "copying".into();
                            force = true;
                        }
                        EngineEvent::File { path, size } => {
                            progress.bytes_done += pending_size;
                            pending_size = size;
                            progress.files_done += 1;
                            progress.current_file = path;
                        }
                    }
                    if force || last_emit.elapsed() >= Duration::from_millis(200) {
                        last_emit = Instant::now();
                        *self.current.lock().unwrap() = Some(progress.clone());
                        let _ = self.app.emit("progress", &progress);
                    }
                },
            );

            let record = summarize(run_id, &task, job.trigger, requested, started_at, sources);
            let settings = {
                let mut s = self.store.lock().unwrap();
                let st = s.states.entry(task.id.clone()).or_default();
                st.last_run_at = Some(record.finished_at);
                st.last_status = Some(record.status);
                st.last_message = Some(record.message.clone());
                s.save_states();
                s.add_history(record.clone());
                s.settings.clone()
            };
            *self.current.lock().unwrap() = None;
            self.changed();
            self.notify(&record, &settings);
        }
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
            toast(&self.app, title, &r.message, sound);
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
            let ready: Vec<Task> = watch
                .poll(&tasks)
                .into_iter()
                .filter(|t| t.enabled && !paused)
                .cloned()
                .collect();
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
                        });
                        if queued && notify {
                            toast(&self.app, format!("הכונן חובר - מתחיל גיבוי: {}", t.name), drives, &sound);
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
                            });
                        } else if let Slot::Run(trigger) = main {
                            due.push(Job {
                                task_id: t.id.clone(),
                                mode: None,
                                trigger,
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

/// Shows a Windows notification. `sound` is a toast sound name; empty plays nothing.
pub fn toast(app: &AppHandle, title: impl Into<String>, body: impl Into<String>, sound: &str) {
    let mut n = app.notification().builder().title(title).body(body);
    if !sound.is_empty() {
        n = n.sound(sound);
    }
    let _ = n.show();
}
