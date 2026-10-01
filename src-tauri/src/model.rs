//! Data types shared between the core and the UI (serialized as camelCase JSON).

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BackupMode {
    /// Destination becomes identical to the source, in a new dated folder.
    Full,
    /// Only new/changed files are copied into the latest dated folder.
    Incremental,
}

/// When a task runs. Times are local wall-clock strings: "HH:MM", "YYYY-MM-DDTHH:MM".
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Schedule {
    Manual,
    Once { at: String },
    Daily { time: String },
    /// days: 0 = Sunday .. 6 = Saturday
    Weekly { days: Vec<u8>, time: String },
    /// day: 1..=31, clamped to the month's last day
    Monthly { day: u8, time: String },
    Interval { minutes: u32 },
}

/// A rule for files/folders that are never backed up.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FilterRule {
    /// One or more extensions: "tmp", "tmp, log, .bak"
    Extension { value: String },
    /// Wildcard on the file name: "~$*", "Thumbs.db", "*.part"
    Pattern { value: String },
    /// Folder name anywhere in the tree ("node_modules") or a full path.
    Folder { value: String },
    /// Files larger than this many MB.
    LargerThan { mb: u64 },
    /// Files last modified more than this many days ago.
    OlderThan { days: u32 },
    Hidden,
    System,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub source: String,
    /// Root folder that holds this task's dated backup folders.
    pub destination: String,
    /// Prefix of the dated folders ("<prefix> 2026-10-01 03-00").
    pub folder_name: String,
    pub mode: BackupMode,
    pub schedule: Schedule,
    pub enabled: bool,
    /// How many dated full backups to keep (>= 1). Older ones are deleted after a successful full.
    pub keep_count: u32,
    /// Full backup: rename the previous folder and mirror into it instead of copying everything again.
    pub reuse_previous: bool,
    /// Incremental task: start a new dated full backup every N days (0 = never).
    pub full_every_days: u32,
    pub copy_empty_dirs: bool,
    /// Run a scheduled occurrence that was missed while the computer was off.
    pub catch_up: bool,
    pub filters: Vec<FilterRule>,
    /// Also apply the global filter rules from Settings.
    pub use_global_filters: bool,
}

impl Default for Task {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            source: String::new(),
            destination: String::new(),
            folder_name: String::new(),
            mode: BackupMode::Incremental,
            schedule: Schedule::Manual,
            enabled: true,
            keep_count: 1,
            reuse_previous: false,
            full_every_days: 0,
            copy_empty_dirs: false,
            catch_up: true,
            filters: Vec::new(),
            use_global_filters: true,
        }
    }
}

/// Runtime state per task, kept apart from the task config so edits/undo don't touch it.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskState {
    pub next_run: Option<DateTime<Local>>,
    pub last_run_at: Option<DateTime<Local>>,
    pub last_status: Option<RunStatus>,
    pub last_message: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Success,
    Warning,
    Failed,
    Cancelled,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Trigger {
    Manual,
    Scheduled,
    CatchUp,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: String,
    pub task_id: String,
    pub task_name: String,
    pub trigger: Trigger,
    pub mode: BackupMode,
    pub started_at: DateTime<Local>,
    pub finished_at: DateTime<Local>,
    pub status: RunStatus,
    pub message: String,
    pub target_folder: Option<String>,
    pub files_copied: u64,
    pub bytes_copied: u64,
    pub files_deleted: u64,
    pub files_failed: u64,
    pub errors: Vec<String>,
    pub exit_code: Option<i32>,
    pub log_file: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// "system" | "light" | "dark"
    pub theme: String,
    pub notify_success: bool,
    pub notify_failure: bool,
    pub scheduler_paused: bool,
    pub close_to_tray: bool,
    pub first_run_done: bool,
    /// Rules applied to every task that has use_global_filters.
    pub global_filters: Vec<FilterRule>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            notify_success: true,
            notify_failure: true,
            scheduler_paused: false,
            close_to_tray: true,
            first_run_done: false,
            global_filters: Vec::new(),
        }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub run_id: String,
    pub task_id: String,
    pub task_name: String,
    pub mode: BackupMode,
    /// "preparing" | "scanning" | "copying" | "finishing"
    pub phase: String,
    pub started_at: DateTime<Local>,
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current_file: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BackupFolder {
    pub name: String,
    pub path: String,
    pub created_at: DateTime<Local>,
    pub partial: bool,
}
