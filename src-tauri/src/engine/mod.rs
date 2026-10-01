//! Copy engine abstraction. The backup logic only talks to `CopyEngine`, so robocopy
//! can be swapped for another implementation later.

pub mod robocopy;

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

pub struct CopyJob {
    pub source: PathBuf,
    pub target: PathBuf,
    /// true: make target identical to source (delete extras). false: copy new/changed only.
    pub mirror: bool,
    pub copy_empty_dirs: bool,
    pub filters: Filters,
    /// Where the engine writes its detailed log (kept for the run log viewer).
    pub log_file: PathBuf,
}

/// Exclusions in engine terms (compiled from the user's filter rules).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Filters {
    /// Wildcards on file names.
    pub exclude_files: Vec<String>,
    /// Folder names or full paths.
    pub exclude_dirs: Vec<String>,
    /// Skip files larger than this many bytes.
    pub max_size: Option<u64>,
    /// Skip files last modified more than this many days ago.
    pub max_age_days: Option<u32>,
    pub exclude_hidden: bool,
    pub exclude_system: bool,
}

pub enum EngineEvent {
    /// Result of the pre-scan: how much work the real run will do.
    Totals { files: u64, bytes: u64 },
    /// A file copy started.
    File { path: String, size: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    /// Finished, but some items mismatched or were skipped.
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
    pub errors: Vec<String>,
    pub exit_code: Option<i32>,
}

pub struct CopyResult {
    pub outcome: Outcome,
    pub message: String,
    pub stats: CopyStats,
}

pub trait CopyEngine: Send + Sync {
    fn run(&self, job: &CopyJob, cancel: &AtomicBool, on_event: &mut dyn FnMut(EngineEvent)) -> CopyResult;
}
