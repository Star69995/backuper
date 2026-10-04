//! An index file inside every completed backup folder: path, size and modified time of each file
//! in it. Incrementals build "what's already backed up" from these instead of walking the whole
//! chain on the (often slow) destination drive. A folder without a valid index (made by an older
//! version, or the write failed) is walked once and gets one.
//!
//! The index describes the folder as written. Files deleted from a backup by hand afterwards
//! still count as backed up until the next full backup.

use crate::engine::{native, Filters, ListedFile};
use crate::tasklist::{set_attributes, HIDDEN, NORMAL};
use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// Hidden, in the backup folder's root.
pub const FILE_NAME: &str = ".backuper-index";
const MAGIC: &str = "backuper-index 1";
/// Last line: a file cut off anywhere (even mid-path) lacks it.
const END: &str = "end";

fn folder_name(folder: &Path) -> String {
    folder.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn is_index_file(path: &Path) -> bool {
    fs::read(path).is_ok_and(|b| b.starts_with(MAGIC.as_bytes()))
}

/// What a backup folder holds (its index file aside), and whether all of it could be read.
pub fn scan(folder: &Path) -> (Vec<ListedFile>, bool) {
    match native::list_files(folder, &Filters::default(), &AtomicBool::new(false)) {
        Ok(Some(l)) => (
            l.files.into_iter().filter(|f| !f.rel.eq_ignore_ascii_case(FILE_NAME)).collect(),
            l.failed_dirs == 0,
        ),
        _ => (Vec::new(), false),
    }
}

/// Header: magic, file count and the folder's name (an index copied from elsewhere, e.g. a source
/// that is itself a backup folder, doesn't count). Then one line per file, then END.
pub fn write(folder: &Path, files: &[ListedFile]) {
    let path = folder.join(FILE_NAME);
    if path.exists() {
        // A file by that name that isn't an index came from the source: leave it.
        if !is_index_file(&path) {
            return;
        }
        set_attributes(&path, NORMAL);
    }
    let mut s = format!("{MAGIC}\t{}\t{}\r\n", files.len(), folder_name(folder));
    for f in files {
        s.push_str(&format!("{}\t{}\t{}\r\n", f.size, f.modified, f.rel));
    }
    s.push_str(END);
    s.push_str("\r\n");
    if fs::write(&path, s).is_ok() {
        set_attributes(&path, HIDDEN);
    }
}

pub fn read(folder: &Path) -> Option<Vec<ListedFile>> {
    let text = fs::read_to_string(folder.join(FILE_NAME)).ok()?;
    let text = text.strip_suffix(&format!("\r\n{END}\r\n"))?;
    let mut lines = text.split("\r\n");
    let mut header = lines.next()?.splitn(3, '\t');
    if header.next()? != MAGIC {
        return None;
    }
    let count: usize = header.next()?.parse().ok()?;
    if header.next()? != folder_name(folder) {
        return None;
    }
    let files = lines
        .map(|l| {
            let mut p = l.splitn(3, '\t');
            Some(ListedFile {
                size: p.next()?.parse().ok()?,
                modified: p.next()?.parse().ok()?,
                rel: p.next()?.to_string(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    (files.len() == count).then_some(files)
}

/// The folder's index, made now if it's missing.
pub fn load_or_build(folder: &Path) -> Vec<ListedFile> {
    if let Some(files) = read(folder) {
        return files;
    }
    let (files, complete) = scan(folder);
    // Partly unreadable: use what was found (missing files just get copied again), don't save it.
    if complete {
        write(folder, &files);
    }
    files
}

/// Before a folder is reused for a new full backup (its index would be stale).
pub fn remove(folder: &Path) {
    let path = folder.join(FILE_NAME);
    if is_index_file(&path) {
        set_attributes(&path, NORMAL);
        let _ = fs::remove_file(path);
    }
}
