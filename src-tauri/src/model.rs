//! Data types shared between the core and the UI (serialized as camelCase JSON).

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BackupMode {
    /// A new dated folder identical to the source.
    Full,
    /// A new dated folder holding only files new/changed since the previous backup.
    Incremental,
}

/// When a task runs. Times are local wall-clock strings: "HH:MM", "YYYY-MM-DDTHH:MM".
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Schedule {
    Manual,
    Once {
        at: String,
    },
    Daily {
        time: String,
    },
    /// days: 0 = Sunday .. 6 = Saturday
    Weekly {
        days: Vec<u8>,
        time: String,
    },
    /// day: 1..=31, clamped to the month's last day
    Monthly {
        day: u8,
        time: String,
    },
    Interval {
        minutes: u32,
    },
}

/// A rule for files/folders that are never backed up.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FilterRule {
    /// One or more extensions: "tmp", "tmp, log, .bak"
    Extension {
        value: String,
    },
    /// Wildcard on the file name: "~$*", "Thumbs.db", "*.part"
    Pattern {
        value: String,
    },
    /// Folder name anywhere in the tree ("node_modules") or a full path.
    Folder {
        value: String,
    },
    /// Files larger than this many MB.
    LargerThan {
        mb: u64,
    },
    /// Files last modified more than this many days ago.
    OlderThan {
        days: u32,
    },
    Hidden,
    System,
}

/// One source folder of a task. Each source has its own chain of dated backup folders.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Source {
    pub path: String,
    /// Prefix of this source's dated folders ("<folderName> 2026-10-01 03-00 מלא").
    pub folder_name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Task {
    pub id: String,
    pub name: String,
    /// All sources run together, on the same schedule, with the same date in their folder names.
    pub sources: Vec<Source>,
    /// Root folder that holds the dated backup folders of all sources.
    pub destination: String,
    /// Backup type of the main schedule.
    pub mode: BackupMode,
    pub schedule: Schedule,
    /// Combined mode (with mode = Incremental): full backups run on this schedule, incrementals on `schedule`.
    pub full_schedule: Option<Schedule>,
    pub enabled: bool,
    /// How many full backups (each with its incrementals) to keep per source (>= 1).
    pub keep_count: u32,
    /// Delete old backups before a full backup starts (frees space) instead of after it succeeds.
    pub delete_before: bool,
    /// Full backup: rename the previous full folder and mirror into it instead of copying everything again.
    pub reuse_previous: bool,
    /// On a full backup, delete incremental folders that ended up empty (no changes at that run).
    pub delete_empty_incrementals: bool,
    pub copy_empty_dirs: bool,
    /// Run a scheduled occurrence that was missed while the computer was off.
    pub catch_up: bool,
    pub filters: Vec<FilterRule>,
    /// Also apply the global filter rules from Settings.
    pub use_global_filters: bool,
    /// Pre-multi-source fields, read once and migrated into `sources`.
    #[serde(skip_serializing)]
    pub source: String,
    #[serde(skip_serializing)]
    pub folder_name: String,
}

impl Default for Task {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            sources: Vec::new(),
            destination: String::new(),
            mode: BackupMode::Incremental,
            schedule: Schedule::Manual,
            full_schedule: None,
            enabled: true,
            keep_count: 1,
            delete_before: false,
            reuse_previous: false,
            delete_empty_incrementals: true,
            copy_empty_dirs: false,
            catch_up: true,
            filters: Vec::new(),
            use_global_filters: true,
            source: String::new(),
            folder_name: String::new(),
        }
    }
}

impl Task {
    /// Moves the legacy single `source` into `sources`.
    pub fn migrate(&mut self) {
        if self.sources.is_empty() && !self.source.is_empty() {
            self.sources.push(Source {
                path: std::mem::take(&mut self.source),
                folder_name: std::mem::take(&mut self.folder_name),
            });
        }
    }
}

/// Runtime state per task, kept apart from the task config so edits/undo don't touch it.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskState {
    pub next_run: Option<DateTime<Local>>,
    /// Next run of `full_schedule` (combined mode).
    pub next_full_run: Option<DateTime<Local>>,
    pub last_run_at: Option<DateTime<Local>>,
    pub last_status: Option<RunStatus>,
    pub last_message: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    // Ordered from best to worst, so the overall status of a run is the max.
    Success,
    Warning,
    Cancelled,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Trigger {
    Manual,
    Scheduled,
    CatchUp,
}

/// Result of one source within a run.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceRun {
    pub source: String,
    pub folder_name: String,
    pub mode: Option<BackupMode>,
    pub status: Option<RunStatus>,
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
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: String,
    pub task_id: String,
    pub task_name: String,
    pub trigger: Trigger,
    /// The requested mode (each source may still have needed a full; see `sources`).
    pub mode: BackupMode,
    pub started_at: DateTime<Local>,
    pub finished_at: DateTime<Local>,
    pub status: RunStatus,
    pub message: String,
    pub files_copied: u64,
    pub bytes_copied: u64,
    pub files_deleted: u64,
    pub files_failed: u64,
    #[serde(default)]
    pub sources: Vec<SourceRun>,
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
    /// The user's choice; the Windows startup entry is synced to it on every launch
    /// (an uninstall removes the entry, so a one-time registration isn't enough).
    pub start_with_windows: bool,
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
            start_with_windows: true,
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
    /// "scanning" | "deleting" | "copying"
    pub phase: String,
    pub started_at: DateTime<Local>,
    /// 1-based index of the source being backed up, out of source_count.
    pub source_index: usize,
    pub source_count: usize,
    pub source_path: String,
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
    pub kind: BackupMode,
    pub partial: bool,
}

/// The dated backups of one source.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SourceBackups {
    pub source: String,
    pub folder_name: String,
    pub backups: Vec<BackupFolder>,
}
