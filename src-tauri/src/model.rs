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

/// A rule for which files get backed up (all exclusions, except `Include`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FilterRule {
    /// Back up only these: extensions ("lrcat, docx"; older rules may also hold wildcards/names
    /// like "*.lrcat", "notes.txt").
    Include {
        value: String,
    },
    /// Back up only files with these names, exactly as typed (a bare word is a whole name, not an
    /// extension): "notes.txt, report*"
    IncludeName {
        value: String,
    },
    /// One or more extensions: "tmp", "tmp, log, .bak"
    Extension {
        value: String,
    },
    /// Names or wildcards on the file name, comma-separated, exactly as typed: "~$*, Thumbs.db, *.part"
    Pattern {
        value: String,
    },
    /// Regular expression on the file name (case-insensitive, searched anywhere in the name).
    /// `include`: back up only matching files; otherwise skip them.
    /// `folders`: match folder names instead; a matching folder is skipped with everything in it
    /// (skipping is the only mode for folders, so `include` must be false).
    Regex {
        value: String,
        #[serde(default)]
        include: bool,
        #[serde(default)]
        folders: bool,
    },
    /// Folder name anywhere in the tree ("node_modules") or a full path.
    Folder {
        value: String,
    },
    /// One file, by its full path ("C:\\pagefile.sys"): usually added from a run's errors.
    File {
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

/// What a task does when its drives (destination / sources) get connected.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum DriveAction {
    #[default]
    Off,
    /// Queue a backup right away.
    Run,
    /// Show the window and ask whether to back up.
    Ask,
}

/// How long old backups are kept.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum KeepMode {
    /// The newest `keep_count` full backups (each with its incrementals).
    #[default]
    Count,
    /// Whatever is needed to restore any moment of the last `keep_days` days.
    Days,
    /// Nothing is deleted (except incomplete folders and, if enabled, empty incrementals).
    All,
}

/// One source folder of a task. Each source has its own chain of dated backup folders.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Source {
    pub path: String,
    /// Prefix of this source's dated folders ("<folderName> 2026-10-01 03h00 מלא").
    pub folder_name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Task {
    pub id: String,
    pub name: String,
    /// The task list (group) the task belongs to; empty = the general list.
    pub group: String,
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
    /// Imported from Cobian whose existing backups in the destination haven't been taken over or
    /// declined yet. Nothing runs (scheduled, drive-connect or manual) until the user decides.
    pub cobian_pending: bool,
    /// The user chose to continue from Cobian's backups in the destination. They are renamed right
    /// before the task's first run here (not when the choice is made), so until then Cobian still
    /// sees them. Cleared once every folder was renamed.
    pub cobian_takeover: bool,
    pub keep_mode: KeepMode,
    /// KeepMode::Count: how many full backups (each with its incrementals) to keep per source (>= 1).
    pub keep_count: u32,
    /// KeepMode::Days: how many days back backups are kept (>= 1).
    pub keep_days: u32,
    /// Delete old backups before a full backup starts (frees space) instead of after it succeeds.
    pub delete_before: bool,
    /// Full backup: rename the previous full folder and mirror into it instead of copying everything again.
    pub reuse_previous: bool,
    /// On a full backup, delete incremental folders that ended up empty (no changes at that run).
    pub delete_empty_incrementals: bool,
    pub copy_empty_dirs: bool,
    /// Run a scheduled occurrence that was missed while the computer was off.
    pub catch_up: bool,
    /// Start (or offer to start) a backup when the task's drives become available.
    pub on_drive_connect: DriveAction,
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
            group: String::new(),
            sources: Vec::new(),
            destination: String::new(),
            mode: BackupMode::Incremental,
            schedule: Schedule::Manual,
            full_schedule: None,
            enabled: true,
            cobian_pending: false,
            cobian_takeover: false,
            keep_mode: KeepMode::Count,
            keep_count: 1,
            keep_days: 30,
            delete_before: false,
            reuse_previous: false,
            delete_empty_incrementals: true,
            copy_empty_dirs: false,
            catch_up: true,
            on_drive_connect: DriveAction::Off,
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

/// One Cobian backup folder that a takeover renamed (`cobian-renames.json`), so its name can be put back.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CobianRename {
    pub task_id: String,
    /// The folder that holds it.
    pub destination: String,
    /// Cobian's name.
    pub from: String,
    /// The name it got here.
    pub to: String,
    pub kind: BackupMode,
    pub at: DateTime<Local>,
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
    /// The last run was paused and its folder is still there: the next run continues in it.
    pub resumable: bool,
    /// The newest backup that was written (a failed or stopped run doesn't replace it).
    pub last_backup: Option<LastBackup>,
}

/// Size of a task's newest backup, shown in the task list.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LastBackup {
    pub at: DateTime<Local>,
    /// Size of the backup folders it wrote (all sources).
    pub bytes: u64,
    /// Full if any source made a full backup.
    pub mode: BackupMode,
}

impl LastBackup {
    /// The backup a run left behind, if it wrote one.
    pub fn of(r: &RunRecord) -> Option<Self> {
        if r.backup_bytes == 0 || !matches!(r.status, RunStatus::Success | RunStatus::Warning) {
            return None;
        }
        let full = r.mode == BackupMode::Full || r.sources.iter().any(|s| s.mode == Some(BackupMode::Full));
        Some(Self {
            at: r.finished_at,
            bytes: r.backup_bytes,
            mode: if full { BackupMode::Full } else { BackupMode::Incremental },
        })
    }
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
    /// The task's drive was connected.
    DriveConnected,
}

/// One file or folder that couldn't be copied. `code` is the Win32 error code (2 = not found,
/// 32 = in use...), which the UI turns into an explanation; `message` is the system's own text.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FileError {
    pub path: String,
    pub code: Option<u32>,
    pub message: String,
    /// What the file looked like when it failed (attributes, reparse tag, size, destination...), for
    /// debugging: shown only in the copied error text. Empty = nothing recorded.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub debug: String,
}

impl FileError {
    /// Older history stored errors as "path - message" (from std: "... (os error N)").
    fn from_legacy(s: &str) -> Self {
        let (path, message) = s.rsplit_once(" - ").unwrap_or(("", s));
        let code = message
            .rsplit_once("(os error ")
            .and_then(|(_, rest)| rest.trim_end_matches(')').parse().ok());
        FileError {
            path: path.to_string(),
            code,
            message: message.to_string(),
            debug: String::new(),
        }
    }
}

fn file_errors<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<FileError>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Full(FileError),
        Legacy(String),
    }
    Ok(Vec::<Repr>::deserialize(d)?
        .into_iter()
        .map(|r| match r {
            Repr::Full(e) => e,
            Repr::Legacy(s) => FileError::from_legacy(&s),
        })
        .collect())
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
    /// Size of the finished backup folder (0 if it didn't complete).
    pub backup_bytes: u64,
    /// Space freed by deleting older backups of this source.
    pub freed_bytes: u64,
    pub files_deleted: u64,
    pub files_failed: u64,
    #[serde(deserialize_with = "file_errors")]
    pub errors: Vec<FileError>,
    pub exit_code: Option<i32>,
    pub log_file: Option<String>,
    /// How the source was copied (engine command line, exit code, volumes, files that were saved
    /// by a second try...), for debugging: shown only in the copied error text.
    pub debug: Vec<String>,
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
    #[serde(default)]
    pub backup_bytes: u64,
    #[serde(default)]
    pub freed_bytes: u64,
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
    /// Windows toast sound name ("Default", "IM", "Mail", "Reminder", "SMS"). Empty = silent.
    pub sound_success: String,
    /// Used for failures, warnings and notices.
    pub sound_failure: String,
    pub scheduler_paused: bool,
    pub close_to_tray: bool,
    /// The user's choice; the Windows startup entry is synced to it on every launch
    /// (an uninstall removes the entry, so a one-time registration isn't enough).
    pub start_with_windows: bool,
    /// Rules applied to every task that has use_global_filters.
    pub global_filters: Vec<FilterRule>,
    /// Folder that always holds a copy of the task list (e.g. on the backup drive). Empty = off.
    pub task_list_copy_dir: String,
    /// "auto" (download, install when idle and in the tray) | "notify" | "off".
    pub update_mode: String,
    /// Before a full copy, walk the whole tree once just to get exact progress totals. Off: the
    /// copy starts at once and a scan of the source beside it finds the totals (see `backup::full_totals`).
    pub exact_progress: bool,
    /// While a full copy runs, scan the source beside it to find how much there is to copy (the
    /// percentage and the finish time). Off: counters only, plus the estimate from the last backup.
    pub background_totals: bool,
}

/// A message for the user that waits in the UI until dismissed (e.g. the task list was recovered).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub title: String,
    pub message: String,
    /// A file or folder the user may want to see.
    pub path: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            notify_success: true,
            notify_failure: true,
            sound_success: "Default".into(),
            sound_failure: "Reminder".into(),
            scheduler_paused: false,
            close_to_tray: true,
            start_with_windows: true,
            global_filters: Vec::new(),
            task_list_copy_dir: String::new(),
            update_mode: "auto".into(),
            exact_progress: false,
            background_totals: true,
        }
    }
}

/// Where the app's self-update stands (not persisted).
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current_version: String,
    /// "idle" | "checking" | "upToDate" | "available" | "downloading" | "ready" | "error"
    pub state: String,
    /// The newer version, once one was found.
    pub version: Option<String>,
    /// Its release notes.
    pub notes: Option<String>,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub checked_at: Option<DateTime<Local>>,
    pub error: Option<String>,
    /// The user asked to install; it waits for the running/queued backups to finish.
    pub install_waiting: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub run_id: String,
    pub task_id: String,
    pub task_name: String,
    pub mode: BackupMode,
    /// "scanning" | "deleting" | "pruning" | "copying"
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
    /// Where the totals come from: "unknown" (counters only) | "estimate" (from the previous
    /// backup) | "exact".
    pub totals: String,
    /// A scan beside the copy is still finding the exact totals (the user can skip it).
    pub totals_scan: bool,
    /// When copying began for the current source (not the scan before it): the time estimate's clock.
    pub copy_started_at: Option<DateTime<Local>>,
    /// Bytes per second of this task's last real copy, for an estimate before this run has a speed.
    pub expected_rate: Option<u64>,
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
    /// Size of the folder's files, filled by `list_task_backups` (None: not known, e.g. a partial folder).
    pub bytes: Option<u64>,
}

/// The dated backups of one source.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SourceBackups {
    pub source: String,
    pub folder_name: String,
    pub backups: Vec<BackupFolder>,
}

/// What a task's next full backup needs on the destination drive (see `backup::full_space`).
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FullSpace {
    /// About the size of the previous full backup(s), all sources.
    pub needed: u64,
    /// What delete-before removes first (0 without it), which counts as free space.
    pub freed: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_from_older_versions_get_the_defaults() {
        let s: Settings = serde_json::from_str(r#"{"theme": "dark"}"#).unwrap();
        assert_eq!(s.theme, "dark");
        assert!(s.background_totals, "the background scan is on unless switched off");
        let s: Settings = serde_json::from_str(r#"{"backgroundTotals": false}"#).unwrap();
        assert!(!s.background_totals);
    }

    #[test]
    fn reads_legacy_string_errors() {
        let run: SourceRun = serde_json::from_str(
            r#"{"errors": [
                "C:\\a - b\\x.o - The system cannot find the file specified. (os error 2)",
                {"path": "C:\\y", "code": 32, "message": "in use"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(run.errors[0].path, r"C:\a - b\x.o");
        assert_eq!(run.errors[0].code, Some(2));
        assert_eq!(run.errors[1].code, Some(32));
    }
}

/// What the rule editor shows about a regex filter (see `regexcheck`).
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegexCheck {
    /// Why the pattern is invalid; the other fields stay empty then.
    pub error: Option<String>,
    /// The pattern read left to right: each piece of it and what it means.
    pub parts: Vec<RegexPart>,
    pub note: Option<String>,
    /// Result of the try-out name (None = no name given).
    pub matched: Option<bool>,
    pub matched_text: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegexPart {
    pub token: String,
    pub meaning: String,
}

/// A path to run through all the filter rules (the filter check in the rule editor).
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FilterProbe {
    /// Full path, or a path relative to the task's source. Ending with `\` = a folder.
    pub path: String,
    /// Used when the path isn't on disk (on disk, the real values are used).
    pub size_mb: Option<f64>,
    pub age_days: Option<u32>,
    pub hidden: bool,
    pub system: bool,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum RuleEffect {
    /// Doesn't change what happens to this path.
    #[default]
    None,
    /// Leaves the path out of the backup.
    Skips,
    /// A "back up only" rule the path matches.
    Keeps,
    /// Can't tell: the rule needs the size or date of a file that isn't on disk.
    Unknown,
    /// The rule isn't complete or valid, so it wasn't checked.
    Invalid,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RuleResult {
    pub effect: RuleEffect,
    /// Hebrew: how (e.g. the skipped folder on the way), or why it wasn't checked.
    pub detail: Option<String>,
}

/// The real file or folder the check used.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OnDisk {
    pub size: u64,
    /// RFC 3339.
    pub modified: Option<String>,
    pub hidden: bool,
    pub system: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FilterCheck {
    /// None = nothing to check (no path, or not a usable one: see `notes`).
    pub kept: Option<bool>,
    pub folder: bool,
    /// The checked path as a full path (a relative one is read under the task's first source); what a
    /// "skip this" rule added from the check should hold. None when there is no source to read it under.
    pub full_path: Option<String>,
    /// The same path read as a folder, when it isn't on disk and wasn't typed with a trailing `\`
    /// (so it could be a file or a folder). Its own `as_folder` is always None.
    pub as_folder: Option<Box<FilterCheck>>,
    /// Hebrew: why it is left out (None when kept).
    pub reason: Option<String>,
    pub on_disk: Option<OnDisk>,
    pub notes: Vec<String>,
    /// One per rule, in the order given.
    pub rules: Vec<RuleResult>,
    pub global_rules: Vec<RuleResult>,
}
