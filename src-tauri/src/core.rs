//! The running core: a single worker that executes queued backups one at a time,
//! and the built-in scheduler that enqueues due tasks. UI-independent apart from events.

use crate::backup;
use crate::engine::{robocopy::Robocopy, CopyEngine, EngineEvent};
use crate::model::{BackupMode, Progress, RunRecord, RunStatus, Schedule, Trigger};
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

/// A scheduled time this late is treated as missed (computer was off or asleep).
const MISSED_GRACE_MINUTES: i64 = 2;
const SCHEDULER_TICK: Duration = Duration::from_secs(5);

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
            engine: Box::new(Robocopy),
            app,
        })
    }

    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || me.worker_loop());
        let me = self.clone();
        std::thread::spawn(move || me.scheduler_loop());
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
        if running.as_deref() == Some(job.task_id.as_str()) || q.iter().any(|j| j.task_id == job.task_id) {
            return false;
        }
        q.push_back(job);
        drop(q);
        self.queue_cv.notify_one();
        self.changed();
        true
    }

    /// Cancels a queued or running task.
    pub fn cancel(&self, task_id: &str) {
        self.queue.lock().unwrap().retain(|j| j.task_id != task_id);
        if self.current.lock().unwrap().as_ref().is_some_and(|p| p.task_id == task_id) {
            self.cancel.store(true, Ordering::SeqCst);
        }
        self.changed();
    }

    fn next_job(&self) -> Job {
        let mut q = self.queue.lock().unwrap();
        loop {
            if let Some(j) = q.pop_front() {
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
            let log_file = logs_dir.join(format!("{run_id}.log"));
            let requested = job.mode.unwrap_or(task.mode);
            let started_at = Local::now();
            let mut progress = Progress {
                run_id: run_id.clone(),
                task_id: task.id.clone(),
                task_name: task.name.clone(),
                mode: requested,
                phase: "scanning".into(),
                started_at,
                files_done: 0,
                files_total: 0,
                bytes_done: 0,
                bytes_total: 0,
                current_file: String::new(),
            };
            self.cancel.store(false, Ordering::SeqCst);
            *self.current.lock().unwrap() = Some(progress.clone());
            self.changed();

            let mut pending_size = 0u64;
            let mut last_emit = Instant::now();
            let out = backup::run_backup(self.engine.as_ref(), &task, requested, &global_filters, log_file.clone(), &self.cancel, &mut |ev| {
                match ev {
                    EngineEvent::Totals { files, bytes } => {
                        progress.files_total = files;
                        progress.bytes_total = bytes;
                        progress.phase = "copying".into();
                    }
                    EngineEvent::File { path, size } => {
                        progress.bytes_done += pending_size;
                        pending_size = size;
                        progress.files_done += 1;
                        progress.current_file = path;
                    }
                }
                if last_emit.elapsed() >= Duration::from_millis(200) {
                    last_emit = Instant::now();
                    *self.current.lock().unwrap() = Some(progress.clone());
                    let _ = self.app.emit("progress", &progress);
                }
            });

            let record = RunRecord {
                id: run_id,
                task_id: task.id.clone(),
                task_name: task.name.clone(),
                trigger: job.trigger,
                mode: out.mode,
                started_at,
                finished_at: Local::now(),
                status: out.status,
                message: out.message.clone(),
                target_folder: out.target_folder,
                files_copied: out.stats.files_copied,
                bytes_copied: out.stats.bytes_copied,
                files_deleted: out.stats.files_deleted,
                files_failed: out.stats.files_failed,
                errors: out.stats.errors,
                exit_code: out.stats.exit_code,
                log_file: log_file.exists().then(|| log_file.to_string_lossy().to_string()),
            };
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
            self.notify(&record, settings.notify_success, settings.notify_failure);
        }
    }

    fn notify(&self, r: &RunRecord, on_success: bool, on_failure: bool) {
        let (show, title) = match r.status {
            RunStatus::Success => (on_success, format!("הגיבוי הצליח: {}", r.task_name)),
            RunStatus::Warning => (on_success || on_failure, format!("הגיבוי הסתיים עם אזהרות: {}", r.task_name)),
            RunStatus::Failed => (on_failure, format!("הגיבוי נכשל: {}", r.task_name)),
            RunStatus::Cancelled => (false, String::new()),
        };
        if show {
            let _ = self.app.notification().builder().title(title).body(&r.message).show();
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
            let active = t.enabled && t.schedule != Schedule::Manual;
            if !active && st.next_run.is_some() {
                st.next_run = None;
                dirty = true;
            } else if active && st.next_run.is_none() {
                st.next_run = schedule::next_after(&t.schedule, now);
                dirty |= st.next_run.is_some();
            }
        }
        if dirty {
            s.save_states();
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
                        let Some(next) = st.next_run else { continue };
                        if now < next {
                            continue;
                        }
                        let missed = now - next > ChronoDuration::minutes(MISSED_GRACE_MINUTES);
                        if !missed {
                            due.push(Job { task_id: t.id.clone(), mode: None, trigger: Trigger::Scheduled });
                        } else if t.catch_up {
                            due.push(Job { task_id: t.id.clone(), mode: None, trigger: Trigger::CatchUp });
                        }
                        st.next_run = schedule::next_after(&t.schedule, now);
                        dirty = true;
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
