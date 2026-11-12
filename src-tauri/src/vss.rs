//! Files another program holds open (Outlook's mail file, a database, a virtual machine) can't be
//! read normally: the program opened them without sharing. A Volume Shadow Copy is a read-only
//! snapshot of the whole drive that Windows keeps beside it, and files in it aren't locked.
//!
//! Making one needs admin rights, so the app starts itself again as admin with `--vss-copy`
//! (`elevation::run_as_admin`: Windows shows its permission prompt, or with permanent permission
//! a Task Scheduler task starts it without one): that copy (`elevated_main`) makes the
//! snapshot, copies the given files out of it, deletes the snapshot and reports per file.
//! The files to copy travel on the command line (nobody can swap them after the prompt; through
//! the task, in a request file in the user's temp folder); the copy refuses to write anywhere but into one of our dated backup folders.
//!
//! The snapshot is taken without the programs' cooperation (Outlook has no VSS writer), like
//! the one a power cut would leave: a mail file is fine, a database that was in the middle of a
//! transaction may need its own recovery when restored.

use crate::acl;
use crate::engine::native;
use crate::model::FileError;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The command-line switch of the admin copy: `--vss-copy <control path> <source> <destination>...`.
pub const SWITCH: &str = "--vss-copy";
/// How long an unattended run (scheduled) waits for somebody to answer Windows's prompt.
pub const UNATTENDED_WAIT: Duration = Duration::from_secs(180);
/// A command line holds 32767 characters; files that don't fit wait for the next time.
const MAX_PARAMS: usize = 28_000;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
pub fn is_in_use(code: Option<u32>) -> bool {
    matches!(code, Some(32 | 33))
}

pub struct Pair {
    pub src: PathBuf,
    pub dst: PathBuf,
}

#[derive(Debug, PartialEq)]
pub enum Copied {
    /// Bytes copied.
    Done(u64),
    Failed(String),
}

fn control_path() -> PathBuf {
    std::env::temp_dir().join(format!("backuper-vss-{}", uuid::Uuid::new_v4()))
}

fn side(control: &Path, ext: &str) -> PathBuf {
    let mut s = control.as_os_str().to_os_string();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

/// Copies the files from a shadow copy of their drives, one result per pair (same order). Err =
/// nothing was copied: the permission prompt was declined, nobody answered it in `wait` (unattended
/// runs; None waits for as long as it takes), the run was cancelled, or the snapshot failed.
/// Blocks while Windows's prompt is open and while the files are copied.
pub fn copy_open_files(pairs: &[Pair], wait: Option<Duration>, cancel: &AtomicBool) -> Result<Vec<Copied>, String> {
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let control = control_path();
    let mut args = vec![SWITCH.to_string(), control.to_string_lossy().to_string()];
    let mut len = acl::command_line(&args).len();
    let mut sent = 0;
    for p in pairs {
        let next = [p.src.to_string_lossy().to_string(), p.dst.to_string_lossy().to_string()];
        let more = acl::command_line(&next).len() + 1;
        if len + more > MAX_PARAMS {
            break;
        }
        len += more;
        args.extend(next);
        sent += 1;
    }
    if sent == 0 {
        return Err("נתיב הקובץ ארוך מדי".into());
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(crate::elevation::run_as_admin(&args));
    });
    let started = Instant::now();
    let (started_file, cancel_file) = (side(&control, "started"), side(&control, "cancel"));
    let outcome = loop {
        match rx.recv_timeout(Duration::from_millis(300)) {
            Ok(r) => break r,
            Err(mpsc::RecvTimeoutError::Disconnected) => break Err("הפעולה בהרשאת מנהל נקטעה".to_string()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let running = started_file.exists();
        if cancel.load(Ordering::SeqCst) {
            // The admin copy stops between files; one that hasn't started yet exits right away.
            let _ = std::fs::write(&cancel_file, "");
            if !running {
                cleanup(&control);
                return Err("הגיבוי בוטל".into());
            }
        } else if !running && wait.is_some_and(|w| started.elapsed() > w) {
            let _ = std::fs::write(&cancel_file, "");
            return Err("אף אחד לא אישר את בקשת ההרשאה של Windows בזמן, ולכן הקבצים הפתוחים לא גובו".into());
        }
    };
    let text = std::fs::read_to_string(side(&control, "result")).ok();
    cleanup(&control);
    outcome?;
    let mut results = parse_results(text.as_deref().unwrap_or(""), sent)?;
    // Whatever didn't fit on the command line is for the next run.
    results.extend((sent..pairs.len()).map(|_| Copied::Failed("יותר מדי קבצים בבת אחת - יגובו בפעם הבאה".into())));
    Ok(results)
}

fn cleanup(control: &Path) {
    for ext in ["started", "cancel", "result"] {
        let _ = std::fs::remove_file(side(control, ext));
    }
}

/// The admin copy's report: one line per file (`OK<tab>bytes` | `ERR<tab>code<tab>message`), or
/// `FATAL<tab>message` when no snapshot could be made, then `END`.
fn parse_results(text: &str, expected: usize) -> Result<Vec<Copied>, String> {
    let mut lines = text.lines().peekable();
    if let Some(l) = lines.peek() {
        if let Some(msg) = l.strip_prefix("FATAL\t") {
            return Err(msg.to_string());
        }
    }
    let mut out = Vec::new();
    let mut ended = false;
    for l in lines {
        let mut p = l.splitn(3, '\t');
        match p.next() {
            Some("OK") => out.push(Copied::Done(p.next().and_then(|b| b.parse().ok()).unwrap_or(0))),
            Some("ERR") => {
                let code = p.next().unwrap_or("");
                let msg = p.next().unwrap_or("");
                out.push(Copied::Failed(if code.is_empty() { msg.to_string() } else { format!("{msg} (שגיאה {code})") }));
            }
            Some("END") => ended = true,
            _ => {}
        }
    }
    if !ended || out.len() != expected {
        return Err("תהליך ההעתקה בהרשאת מנהל לא הסתיים כמו שצריך (ייתכן שבוטל, או שהמשתמש דחה את בקשת ההרשאה)".into());
    }
    Ok(out)
}

// ---- the admin copy ----

pub(crate) fn powershell(script: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("לא ניתן להפעיל את PowerShell: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        Ok(stdout)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!("{stdout} {err}").trim().to_string())
    }
}

struct Shadow {
    id: String,
    /// `\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopyN`
    device: String,
}

impl Shadow {
    /// A snapshot of a drive root ("C:\"): readable by any program running as admin, until deleted.
    fn create(root: &str) -> Result<Shadow, String> {
        let script = format!(
            "$r = Invoke-CimMethod -ClassName Win32_ShadowCopy -MethodName Create -Arguments @{{Volume='{root}'; Context='ClientAccessible'}}; \
             if ($r.ReturnValue -ne 0) {{ Write-Output ('ERR ' + $r.ReturnValue); exit 1 }}; \
             $s = Get-CimInstance -ClassName Win32_ShadowCopy | Where-Object {{ $_.ID -eq $r.ShadowID }}; \
             Write-Output ('OK|' + $r.ShadowID + '|' + $s.DeviceObject)"
        );
        let out = powershell(&script).map_err(|e| format!("Windows לא הצליחה ליצור תצלום מצב (VSS) של הכונן {root}: {e}"))?;
        let mut p = out.lines().last().unwrap_or("").trim().splitn(3, '|');
        match (p.next(), p.next(), p.next()) {
            (Some("OK"), Some(id), Some(device)) if !device.is_empty() => Ok(Shadow { id: id.to_string(), device: device.to_string() }),
            _ => Err(format!("Windows לא הצליחה ליצור תצלום מצב (VSS) של הכונן {root}: {out}")),
        }
    }

    fn delete(&self) {
        let _ = powershell(&format!(
            "Get-CimInstance -ClassName Win32_ShadowCopy | Where-Object {{ $_.ID -eq '{}' }} | Remove-CimInstance",
            self.id
        ));
    }
}

/// "C:\Users\x\a.pst" -> ("C:\", "Users\x\a.pst"); None for anything but a local drive path.
fn split_drive(p: &Path) -> Option<(String, String)> {
    let s = p.to_string_lossy().replace('/', "\\");
    let b = s.as_bytes();
    if b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
        Some((s[..3].to_uppercase(), s[3..].to_string()))
    } else {
        None
    }
}

/// The admin copy only ever writes inside one of our dated backup folders.
fn inside_backup_folder(dst: &Path) -> bool {
    dst.ancestors().skip(1).any(|a| a.file_name().and_then(|n| n.to_str()).is_some_and(acl::is_backup_folder_name))
}

/// `args` = the control path, then source and destination of each file. Exit code 0 = a report was written.
pub fn elevated_main(args: &[String]) -> i32 {
    let Some((control, rest)) = args.split_first() else { return 2 };
    let control = Path::new(control);
    if side(control, "cancel").exists() {
        return 3;
    }
    let _ = std::fs::write(side(control, "started"), "");
    acl::enable_backup_privileges();
    let pairs: Vec<Pair> = rest.chunks_exact(2).map(|c| Pair { src: PathBuf::from(&c[0]), dst: PathBuf::from(&c[1]) }).collect();
    let report = report_of(control, &pairs);
    let _ = std::fs::write(side(control, "result"), report);
    0
}

fn report_of(control: &Path, pairs: &[Pair]) -> String {
    let mut shadows: Vec<(String, Result<Shadow, String>)> = Vec::new();
    let mut lines = Vec::new();
    for p in pairs {
        if side(control, "cancel").exists() {
            lines.push("ERR\t\tהגיבוי בוטל".to_string());
            continue;
        }
        let Some((root, rel)) = split_drive(&p.src).filter(|_| inside_backup_folder(&p.dst)) else {
            lines.push("ERR\t\tגיבוי דרך VSS אפשרי רק לקבצים בכונן מקומי, אל תיקיית גיבוי".to_string());
            continue;
        };
        if !shadows.iter().any(|(r, _)| *r == root) {
            let s = Shadow::create(&root);
            shadows.push((root.clone(), s));
        }
        let line = match &shadows.iter().find(|(r, _)| *r == root).unwrap().1 {
            Err(e) => format!("ERR\t\t{}", one_line(e)),
            Ok(s) => {
                let from = PathBuf::from(format!("{}\\{rel}", s.device.trim_end_matches('\\')));
                match native::copy_from_snapshot(&from, &p.dst) {
                    Ok(n) => format!("OK\t{n}"),
                    Err(e) => format!("ERR\t{}\t{}", e.raw_os_error().map(|c| c.to_string()).unwrap_or_default(), one_line(&e.to_string())),
                }
            }
        };
        lines.push(line);
    }
    for (_, s) in &shadows {
        if let Ok(s) = s {
            s.delete();
        }
    }
    // Every snapshot failed: the first error says why, and the caller shows it once for the lot.
    if let Some((_, Err(e))) = shadows.first().filter(|_| shadows.iter().all(|(_, s)| s.is_err())) {
        return format!("FATAL\t{}\n", one_line(e));
    }
    lines.push("END".to_string());
    lines.join("\n") + "\n"
}

fn one_line(s: &str) -> String {
    s.replace(['\r', '\n', '\t'], " ")
}

// ---- used by a backup run and by "back up the open files now" ----

/// The files of `errors` that another program holds open and that sit under `source`, as
/// (index in `errors`, path relative to the source).
fn open_file_errors(source: &Path, errors: &[FileError]) -> Vec<(usize, String)> {
    let prefix = format!("{}\\", source.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase());
    errors
        .iter()
        .enumerate()
        .filter(|(_, e)| is_in_use(e.code))
        .filter_map(|(i, e)| {
            let path = e.path.replace('/', "\\");
            let rel = path.to_lowercase().starts_with(&prefix).then(|| path.get(prefix.len()..)).flatten()?;
            (!rel.is_empty() && !rel.ends_with('\\')).then(|| (i, rel.to_string()))
        })
        .collect()
}

/// What `save_open_files` did.
#[derive(Default)]
pub struct Saved {
    pub files: u64,
    pub bytes: u64,
    /// The saved files as they now are in the backup folder (for its index).
    pub listed: Vec<crate::engine::ListedFile>,
    /// Why some or all couldn't be saved (for the run's message).
    pub problem: Option<String>,
}

/// Copies the open files of `errors` (see `is_in_use`) from a shadow copy into `folder`, the backup
/// folder of `source`. The ones saved leave `errors`; the others keep their error, with the reason.
pub fn save_open_files(source: &Path, folder: &Path, errors: &mut Vec<FileError>, wait: Option<Duration>, cancel: &AtomicBool) -> Saved {
    let open = open_file_errors(source, errors);
    let mut saved = Saved::default();
    if open.is_empty() {
        return saved;
    }
    let pairs: Vec<Pair> = open.iter().map(|(_, rel)| Pair { src: source.join(rel), dst: folder.join(rel) }).collect();
    let results = match copy_open_files(&pairs, wait, cancel) {
        Ok(r) => r,
        Err(e) => {
            saved.problem = Some(format!("הקבצים הפתוחים לא גובו דרך VSS: {e}"));
            return saved;
        }
    };
    let mut done = Vec::new();
    let mut first_problem = None;
    for (((idx, _), pair), result) in open.iter().zip(&pairs).zip(results) {
        match result {
            Copied::Done(n) => {
                saved.files += 1;
                saved.bytes += n;
                if let (Some(item), Ok(rel)) = (native::find(&pair.dst), pair.dst.strip_prefix(folder)) {
                    saved.listed.push(crate::engine::ListedFile { rel: rel.to_string_lossy().to_string(), size: item.size, modified: item.modified });
                }
                done.push(*idx);
            }
            Copied::Failed(msg) => {
                errors[*idx].debug = format!("VSS: {msg}");
                first_problem.get_or_insert(msg);
            }
        }
    }
    if let Some(p) = first_problem {
        saved.problem = Some(format!("חלק מהקבצים הפתוחים לא גובו דרך VSS: {p}"));
    }
    let mut i = 0;
    errors.retain(|_| {
        let keep = !done.contains(&i);
        i += 1;
        keep
    });
    saved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_sharing_violations_count_as_open_files() {
        assert!(is_in_use(Some(32)) && is_in_use(Some(33)));
        assert!(!is_in_use(Some(5)) && !is_in_use(None));
    }

    #[test]
    fn open_files_are_the_in_use_errors_under_the_source() {
        let err = |path: &str, code| FileError { path: path.into(), code: Some(code), ..Default::default() };
        let errors = vec![
            err(r"C:\Users\Inbai\Documents\Outlook\mail.pst", 32),
            err(r"C:\Users\Inbai\Documents\x.txt", 5),
            err(r"c:\users\inbai\documents\lock.db", 33),
            err(r"D:\Elsewhere\a.pst", 32),
            err(r"C:\Users\Inbai\Documents\folder\", 32),
        ];
        let found = open_file_errors(Path::new(r"C:\Users\Inbai\Documents"), &errors);
        assert_eq!(found, vec![(0, r"Outlook\mail.pst".to_string()), (2, "lock.db".to_string())]);
    }

    #[test]
    fn report_is_read_per_file() {
        let ok = parse_results("OK\t10\nERR\t5\tגישה נדחתה\nEND\n", 2).unwrap();
        assert_eq!(ok[0], Copied::Done(10));
        assert!(matches!(&ok[1], Copied::Failed(m) if m.contains("שגיאה 5")));
        // Cut off (no END), the wrong number of files, or no snapshot at all: nothing is trusted.
        assert!(parse_results("OK\t10\n", 1).is_err());
        assert!(parse_results("OK\t10\nEND\n", 2).is_err());
        assert_eq!(parse_results("FATAL\tno snapshot\n", 1), Err("no snapshot".to_string()));
    }

    #[test]
    fn the_admin_copy_writes_only_into_backup_folders() {
        assert!(inside_backup_folder(Path::new(r"K:\cobian\Docs\Documents 2026-11-12 02h34 אינקרמנטלי.partial\Outlook\a.pst")));
        assert!(!inside_backup_folder(Path::new(r"C:\Windows\System32\a.dll")));
        assert!(!inside_backup_folder(Path::new(r"K:\cobian\Docs\Documents 2026-11-12 02h34 אינקרמנטלי")));
    }

    #[test]
    fn drive_paths_are_split_from_the_root() {
        assert_eq!(split_drive(Path::new(r"c:\Users\a.pst")), Some((r"C:\".to_string(), r"Users\a.pst".to_string())));
        assert_eq!(split_drive(Path::new(r"\\server\share\a.pst")), None);
    }
}
