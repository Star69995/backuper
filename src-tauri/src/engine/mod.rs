//! Copy engine abstraction. Full backups go through `CopyEngine`, so robocopy can be swapped
//! for another implementation later. Incrementals use `native` (list the source, copy the changes).

pub mod native;
pub mod robocopy;

use crate::model::FileError;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;

/// Make `target` identical to `source` (a full backup).
pub struct CopyJob {
    pub source: PathBuf,
    pub target: PathBuf,
    pub copy_empty_dirs: bool,
    pub filters: Filters,
    /// Where the engine writes its detailed log (kept for the run log viewer).
    pub log_file: PathBuf,
    /// How the progress totals are found before the copy starts.
    pub totals: TotalsSource,
    /// Totals (files, bytes) that a background scan finds while the copy already runs: the engine
    /// reports them as exact when they arrive. `None` in the message = the scan was given up on
    /// (skipped by the user, or the source couldn't be read). No receiver = no scan is running.
    pub late_totals: Option<Receiver<Option<(u64, u64)>>>,
}

/// Where a mirror's progress totals come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TotalsSource {
    /// A list-only pass first: exact, but the whole tree is walked twice (slow on big drives).
    Scan,
    /// Known from the last backup's index (files, bytes): no extra pass, approximate percentages.
    Estimate(u64, u64),
    /// Nothing known and no extra pass: the copy shows counters only.
    Unknown,
}

/// Folders that are never worth backing up (relevant when the source is a drive root).
pub const ALWAYS_EXCLUDED_DIRS: [&str; 2] = ["$RECYCLE.BIN", "System Volume Information"];

/// Filters in engine terms (compiled from the user's filter rules).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Filters {
    /// Wildcards on file names; when not empty, only matching files are copied.
    pub include_files: Vec<String>,
    /// Wildcards on file names.
    pub exclude_files: Vec<String>,
    /// Regular expressions on file names; when not empty, only files matching all of them are copied.
    /// robocopy can't do these: `native::resolve_regex` turns them into plain names for it.
    pub include_regex: Vec<String>,
    /// Regular expressions on file names; matching files are skipped.
    pub exclude_regex: Vec<String>,
    /// Folder names or full paths.
    pub exclude_dirs: Vec<String>,
    /// Regular expressions on folder names; matching folders are skipped with their contents.
    /// robocopy can't do these either: `native::resolve_regex` turns them into plain folder names.
    pub exclude_dir_regex: Vec<String>,
    /// Skip files larger than this many bytes.
    pub max_size: Option<u64>,
    /// Skip files last modified more than this many days ago.
    pub max_age_days: Option<u32>,
    pub exclude_hidden: bool,
    pub exclude_system: bool,
}

/// A file under some root (a source that passed the filters, or a file in a backup folder).
#[derive(Debug, Clone)]
pub struct ListedFile {
    /// Path relative to the root.
    pub rel: String,
    pub size: u64,
    /// Last modified, as a Windows FILETIME (100ns ticks since 1601, UTC).
    pub modified: u64,
}

pub enum EngineEvent {
    /// The run moved on to the next source folder.
    SourceStarted,
    /// "scanning" | "deleting" (old backups, before a full) | "copying"
    Phase(&'static str),
    /// How much work the copy will do. `exact` = counted (a scan), not estimated from an older backup.
    Totals { files: u64, bytes: u64, exact: bool },
    /// The copy runs without exact totals for now: a background scan will report them (`Totals`).
    TotalsPending,
    /// That scan was given up on: there won't be exact totals for this source.
    TotalsUnavailable,
    /// A file copy started.
    File { path: String, size: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    /// Finished, but some items mismatched or failed.
    Warning,
    Failed,
    Cancelled,
}

#[derive(Debug, Default)]
pub struct CopyStats {
    pub files_copied: u64,
    pub bytes_copied: u64,
    pub files_deleted: u64,
    pub files_failed: u64,
    pub errors: Vec<FileError>,
    pub exit_code: Option<i32>,
}

pub struct CopyResult {
    pub outcome: Outcome,
    pub message: String,
    pub stats: CopyStats,
}

pub trait CopyEngine: Send + Sync {
    /// Mirror source into target.
    fn mirror(&self, job: &CopyJob, cancel: &AtomicBool, on_event: &mut dyn FnMut(EngineEvent)) -> CopyResult;
}
