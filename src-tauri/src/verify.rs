//! The content check of a quick full backup (`Task.verify_content`). A quick full backup updates the
//! previous full in place and leaves alone every file whose size and modified time match the
//! source, so a copy that was damaged on its way to the disk (or later, on the disk) with the right
//! size and time would stay damaged for good. Before the update, the files it would leave alone are
//! read from both sides and compared byte by byte; a copy that differs or can't be read is removed
//! (or, when it can't be, given a modified time that makes it differ), so the update copies it again.
//!
//! The backup side is read with FILE_FLAG_NO_BUFFERING on a local drive: what Windows still holds in
//! memory from the copy would otherwise be compared instead of what is on the disk.

use crate::engine::ListedFile;
use crate::model::VerifyReport;
use std::alloc::{alloc, dealloc, Layout};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Read size. A multiple of every sector size, as unbuffered reads need.
const CHUNK: usize = 4 << 20;
/// Unbuffered reads need a buffer aligned to the sector size (4096 covers 512e and 4Kn drives).
const ALIGN: usize = 4096;
const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
/// Offline / cloud placeholders (OneDrive "online only"): reading them downloads the file.
const ATTR_OFFLINE: u32 = 0x1000;
const ATTR_RECALL_ON_OPEN: u32 = 0x4_0000;
const ATTR_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
/// How many mismatched paths a report keeps.
const MAX_PATHS: usize = 100;

/// A heap buffer aligned for unbuffered reads.
struct Buffer {
    ptr: *mut u8,
    layout: Layout,
}

impl Buffer {
    fn new() -> Self {
        let layout = Layout::from_size_align(CHUNK, ALIGN).expect("buffer layout");
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        Self { ptr, layout }
    }

    fn get(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, CHUNK) }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr, self.layout) }
    }
}

unsafe impl Send for Buffer {}

/// Fills `buf` unless the file ends first (a shorter count = the end of the file).
fn read_full(f: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// One read of the backup copy. Unbuffered, a read returns the whole chunk unless the file ends,
/// and a second read from the unaligned end would fail, so it reads only once.
fn read_target(f: &mut File, buf: &mut [u8], unbuffered: bool) -> io::Result<usize> {
    if !unbuffered {
        return read_full(f, buf);
    }
    loop {
        match f.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            r => return r,
        }
    }
}

enum Compared {
    Same,
    Differs,
    /// The source couldn't be read: not checked.
    SourceError,
    Cancelled,
}

fn compare(
    src: &mut File,
    dst: &mut File,
    unbuffered: bool,
    size: u64,
    bufs: &mut (Buffer, Buffer),
    cancel: &AtomicBool,
    on_bytes: &mut dyn FnMut(u64),
) -> Compared {
    // The two sides are usually on two drives: read both at once when the file is big enough to matter.
    let parallel = size > CHUNK as u64;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Compared::Cancelled;
        }
        let (a, b) = (&mut bufs.0, &mut bufs.1);
        let (rs, rd) = if parallel {
            std::thread::scope(|scope| {
                let target = scope.spawn(|| read_target(dst, b.get(), unbuffered));
                let rs = read_full(src, a.get());
                (
                    rs,
                    target.join().unwrap_or_else(|_| Err(io::Error::other("read failed"))),
                )
            })
        } else {
            (read_full(src, a.get()), read_target(dst, b.get(), unbuffered))
        };
        let Ok(n) = rs else { return Compared::SourceError };
        // A copy that can't be read (a bad sector: CRC error) is as bad as one that differs.
        let Ok(m) = rd else { return Compared::Differs };
        if n != m || a.get()[..n] != b.get()[..m] {
            return Compared::Differs;
        }
        on_bytes(n as u64);
        if n < CHUNK {
            return Compared::Same;
        }
    }
}

fn open_source(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_SEQUENTIAL_SCAN)
        .open(path)
}

fn open_target(path: &Path, unbuffered: bool) -> io::Result<(File, bool)> {
    if unbuffered {
        if let Ok(f) = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_NO_BUFFERING | FILE_FLAG_SEQUENTIAL_SCAN)
            .open(path)
        {
            return Ok((f, true));
        }
    }
    Ok((open_source(path)?, false))
}

/// Makes sure the update copies `path` again: removes it, or when that fails, gives it a modified
/// time no source file has, so it no longer counts as up to date.
fn replace_later(path: &Path) -> bool {
    crate::tasklist::set_attributes(path, crate::tasklist::NORMAL);
    if fs::remove_file(path).is_ok() {
        return true;
    }
    OpenOptions::new()
        .write(true)
        .open(path)
        .and_then(|f| f.set_modified(std::time::UNIX_EPOCH))
        .is_ok()
}

/// Compares each of `files` (relative to both roots, listed in the source with the size and time
/// the backup copy has too) between `source` and `target`. Copies that differ are dealt with by
/// `replace_later`. `on_progress(file, files_done, bytes_done)`. None = cancelled.
pub fn check(
    source: &Path,
    target: &Path,
    files: &[ListedFile],
    cancel: &AtomicBool,
    on_progress: &mut dyn FnMut(&str, u64, u64),
) -> Option<VerifyReport> {
    let unbuffered = crate::engine::native::is_local_path(target);
    let mut bufs = (Buffer::new(), Buffer::new());
    let mut report = VerifyReport::default();
    let (mut files_done, mut bytes_done) = (0u64, 0u64);
    for f in files {
        if cancel.load(Ordering::SeqCst) {
            return None;
        }
        on_progress(&f.rel, files_done, bytes_done);
        let src_path = source.join(&f.rel);
        let dst_path = target.join(&f.rel);
        let base = bytes_done;
        let outcome = (|| {
            let Ok(mut src) = open_source(&src_path) else {
                return Compared::SourceError;
            };
            let Ok(meta) = src.metadata() else {
                return Compared::SourceError;
            };
            if meta.file_attributes() & (ATTR_OFFLINE | ATTR_RECALL_ON_OPEN | ATTR_RECALL_ON_DATA_ACCESS) != 0 {
                return Compared::SourceError;
            }
            // Changed since the listing: the update copies it anyway.
            if meta.file_size() != f.size || meta.last_write_time() != f.modified {
                return Compared::SourceError;
            }
            let Ok((mut dst, unbuffered)) = open_target(&dst_path, unbuffered) else {
                return Compared::SourceError;
            };
            let r = compare(&mut src, &mut dst, unbuffered, f.size, &mut bufs, cancel, &mut |n| {
                bytes_done += n;
                on_progress(&f.rel, files_done, bytes_done);
            });
            // Written to while it was read: not a damaged copy.
            if matches!(r, Compared::Differs)
                && src
                    .metadata()
                    .is_ok_and(|m| m.file_size() != f.size || m.last_write_time() != f.modified)
            {
                return Compared::SourceError;
            }
            r
        })();
        match outcome {
            Compared::Same => {
                report.files += 1;
                report.bytes += f.size;
            }
            Compared::Differs => {
                report.files += 1;
                report.bytes += f.size;
                report.mismatched += 1;
                if report.paths.len() < MAX_PATHS {
                    report.paths.push(src_path.to_string_lossy().to_string());
                }
                if !replace_later(&dst_path) {
                    report.unfixed += 1;
                }
            }
            Compared::SourceError => report.skipped += 1,
            Compared::Cancelled => return None,
        }
        files_done += 1;
        bytes_done = base + f.size;
    }
    on_progress("", files_done, bytes_done);
    Some(report)
}
