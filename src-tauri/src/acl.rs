//! Windows permissions on backup folders that won't delete.
//!
//! Backups don't copy permissions (robocopy `/COPY:DAT`), so their files get the destination's.
//! Still, a delete can be refused: the destination's own permissions are restrictive (a drive set
//! up on another PC), or the files belong to another account. Two ways out:
//! - `grant_own_tree`: the files are ours. An owner may always change an item's permissions, so the
//!   tree gets one that gives us full control - no admin needed.
//! - `delete_elevated`: they aren't. The app starts itself again as admin (Windows asks the user,
//!   UAC), and that copy (`elevated_main`) takes ownership, grants itself full control and deletes.

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// The command-line switch of the admin copy that deletes the folders given after it.
pub const DELETE_SWITCH: &str = "--delete-elevated";

const TOKEN_QUERY: u32 = 0x8;
const TOKEN_ADJUST_PRIVILEGES: u32 = 0x20;
const TOKEN_USER: i32 = 1;
const ACL_REVISION: u32 = 2;
const OBJECT_INHERIT_ACE: u32 = 0x1;
const CONTAINER_INHERIT_ACE: u32 = 0x2;
const FILE_ALL_ACCESS: u32 = 0x1F_01FF;
const SE_FILE_OBJECT: i32 = 1;
const OWNER_SECURITY_INFORMATION: u32 = 0x1;
const DACL_SECURITY_INFORMATION: u32 = 0x4;
const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
/// PROG_INVOKE_SETTING: call the progress function only on errors (it lets the walk go on).
const PROGRESS_INVOKE_ON_ERROR: i32 = 3;
const SE_PRIVILEGE_ENABLED: u32 = 0x2;
const SEE_MASK_NOCLOSEPROCESS: u32 = 0x40;
const SEE_MASK_NOASYNC: u32 = 0x100;
const INFINITE: u32 = 0xFFFF_FFFF;
const ERROR_CANCELLED: i32 = 1223;
const TOKEN_DUPLICATE: u32 = 0x2;
const TOKEN_IMPERSONATE: u32 = 0x4;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
/// SECURITY_IMPERSONATION_LEVEL / TOKEN_TYPE: an impersonation token that can open files.
const SECURITY_IMPERSONATION: i32 = 2;
const TOKEN_TYPE_IMPERSONATION: i32 = 2;
const FILE_LIST_DIRECTORY: u32 = 0x1;
const FILE_DELETE_CHILD: u32 = 0x40;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
/// Exit code of an admin copy that started through the permanent permission and refused the request
/// (outside what the user could do themselves): the caller then asks Windows for permission instead.
pub const REFUSED: i32 = 250;

#[repr(C)]
struct TokenPrivileges {
    count: u32,
    luid_low: u32,
    luid_high: i32,
    attributes: u32,
}

#[repr(C)]
struct ShellExecuteInfoW {
    cb_size: u32,
    f_mask: u32,
    hwnd: isize,
    lp_verb: *const u16,
    lp_file: *const u16,
    lp_parameters: *const u16,
    lp_directory: *const u16,
    n_show: i32,
    h_inst_app: isize,
    lp_id_list: *mut c_void,
    lp_class: *const u16,
    hkey_class: isize,
    dw_hot_key: u32,
    h_icon_or_monitor: isize,
    h_process: isize,
}

type ProgressFn = unsafe extern "system" fn(*const u16, u32, *mut i32, *mut c_void, *mut i32);

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: isize, access: u32, token: *mut isize) -> i32;
    fn GetTokenInformation(token: isize, class: i32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn InitializeAcl(acl: *mut c_void, len: u32, revision: u32) -> i32;
    fn AddAccessAllowedAceEx(acl: *mut c_void, revision: u32, flags: u32, mask: u32, sid: *const c_void) -> i32;
    fn TreeResetNamedSecurityInfoW(
        name: *mut u16,
        object_type: i32,
        info: u32,
        owner: *const c_void,
        group: *const c_void,
        dacl: *const c_void,
        sacl: *const c_void,
        keep_explicit: i32,
        progress: Option<ProgressFn>,
        invoke: i32,
        args: *mut c_void,
    ) -> u32;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut u32) -> i32;
    fn AdjustTokenPrivileges(token: isize, disable_all: i32, new: *const TokenPrivileges, len: u32, prev: *mut c_void, ret: *mut u32) -> i32;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> i32;
    fn DuplicateTokenEx(token: isize, access: u32, attrs: *const c_void, level: i32, kind: i32, new: *mut isize) -> i32;
    fn SetThreadToken(thread: *const isize, token: isize) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn GetShellWindow() -> isize;
    fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> isize;
    fn CloseHandle(h: isize) -> i32;
    fn WaitForSingleObject(h: isize, ms: u32) -> u32;
    fn GetExitCodeProcess(h: isize, code: *mut u32) -> i32;
    fn LocalFree(mem: *mut c_void) -> *mut c_void;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
}

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// The process's token (closed on drop).
struct Token(isize);

impl Token {
    fn open(access: u32) -> Option<Token> {
        let mut h = 0;
        (unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) } != 0).then_some(Token(h))
    }

    /// The account the process runs as, as a SID (u32 words: a SID wants that alignment).
    fn user_sid(&self) -> Option<Vec<u32>> {
        let mut buf = vec![0u64; 64];
        let mut len = 0;
        if unsafe { GetTokenInformation(self.0, TOKEN_USER, buf.as_mut_ptr().cast(), (buf.len() * 8) as u32, &mut len) } == 0 {
            return None;
        }
        // TOKEN_USER starts with a pointer to the SID.
        let sid = buf[0] as usize as *const u8;
        let n = unsafe { GetLengthSid(sid.cast()) } as usize;
        let mut out = vec![0u32; n.div_ceil(4)];
        unsafe { std::ptr::copy_nonoverlapping(sid, out.as_mut_ptr().cast::<u8>(), n) };
        Some(out)
    }

    /// A `u32` piece of token information (TokenElevationType, TokenElevation).
    fn info_u32(&self, class: i32) -> Option<u32> {
        let mut v = 0u32;
        let mut len = 0;
        (unsafe { GetTokenInformation(self.0, class, (&mut v as *mut u32).cast(), 4, &mut len) } != 0).then_some(v)
    }

    fn enable(&self, privilege: &str) -> bool {
        let name = wide(std::ffi::OsStr::new(privilege));
        let mut luid = [0u32; 2];
        if unsafe { LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), luid.as_mut_ptr()) } == 0 {
            return false;
        }
        let tp = TokenPrivileges { count: 1, luid_low: luid[0], luid_high: luid[1] as i32, attributes: SE_PRIVILEGE_ENABLED };
        unsafe { AdjustTokenPrivileges(self.0, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut()) != 0 }
    }
}

impl Drop for Token {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

const TOKEN_ELEVATION_TYPE: i32 = 18;
const TOKEN_ELEVATION: i32 = 20;

/// The account the app runs as, as "S-1-5-21-...".
pub fn user_sid_string() -> Option<String> {
    let sid = Token::open(TOKEN_QUERY)?.user_sid()?;
    let mut out: *mut u16 = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid.as_ptr().cast(), &mut out) } == 0 || out.is_null() {
        return None;
    }
    let s = unsafe {
        let len = (0..).take_while(|&i| *out.add(i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(out, len))
    };
    unsafe { LocalFree(out.cast()) };
    Some(s)
}

/// Whether the app runs with admin rights right now (an elevated token).
pub fn is_elevated() -> bool {
    Token::open(TOKEN_QUERY).and_then(|t| t.info_u32(TOKEN_ELEVATION)).is_some_and(|v| v != 0)
}

/// Whether the account can become admin by itself, with a yes in Windows's prompt (an admin
/// account), rather than needing someone else's password (a standard account).
pub fn is_admin_account() -> bool {
    let Some(t) = Token::open(TOKEN_QUERY) else { return false };
    match t.info_u32(TOKEN_ELEVATION_TYPE) {
        // TokenElevationTypeFull / Limited: an admin account (elevated now, or able to).
        Some(2 | 3) => true,
        // Default: no split token - UAC is off, or a standard account. Only the first is elevated.
        _ => t.info_u32(TOKEN_ELEVATION).is_some_and(|v| v != 0),
    }
}

/// An ACL with one entry: full control for `sid`, inherited by everything below.
fn full_control_acl(sid: &[u32]) -> Option<Vec<u32>> {
    let size = 8 + 8 + sid.len() * 4 + 8; // ACL header + ACE header and mask + SID, with room to spare
    let mut acl = vec![0u32; size.div_ceil(4)];
    let ok = unsafe {
        InitializeAcl(acl.as_mut_ptr().cast(), (acl.len() * 4) as u32, ACL_REVISION) != 0
            && AddAccessAllowedAceEx(
                acl.as_mut_ptr().cast(),
                ACL_REVISION,
                OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
                FILE_ALL_ACCESS,
                sid.as_ptr().cast(),
            ) != 0
    };
    ok.then_some(acl)
}

/// Called for items the reset can't change: nothing to do, the walk goes on.
unsafe extern "system" fn keep_going(_: *const u16, _: u32, _: *mut i32, _: *mut c_void, _: *mut i32) {}

/// Gives the process's account full control of `dir` and everything under it (they inherit it from
/// `dir`; their own entries are dropped). With `take_owner` the account becomes the owner first
/// (needs admin: SeTakeOwnershipPrivilege). Items it can't change are skipped.
fn reset_tree(dir: &Path, take_owner: bool) -> io::Result<()> {
    let token = Token::open(TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES).ok_or_else(io::Error::last_os_error)?;
    let sid = token.user_sid().ok_or_else(io::Error::last_os_error)?;
    let acl = full_control_acl(&sid).ok_or_else(io::Error::last_os_error)?;
    let mut info = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
    if take_owner {
        info |= OWNER_SECURITY_INFORMATION;
    }
    let mut name = wide(dir.as_os_str());
    let err = unsafe {
        TreeResetNamedSecurityInfoW(
            name.as_mut_ptr(),
            SE_FILE_OBJECT,
            info,
            if take_owner { sid.as_ptr().cast() } else { std::ptr::null() },
            std::ptr::null(),
            acl.as_ptr().cast(),
            std::ptr::null(),
            0,
            Some(keep_going),
            PROGRESS_INVOKE_ON_ERROR,
            std::ptr::null_mut(),
        )
    };
    if err == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(err as i32))
    }
}

/// Our own files: full control for us, so they can be deleted. Not for items another account owns.
pub fn grant_own_tree(dir: &Path) -> io::Result<()> {
    reset_tree(dir, false)
}

/// Whether a folder name is one of ours: "<name> 2026-10-01 03h00[m15s][ מלא| אינקרמנטלי][.partial]" (or the older "03-00[-15]").
/// The admin copy deletes only such folders, whatever it is handed.
pub fn is_backup_folder_name(name: &str) -> bool {
    let re = regex::Regex::new(r"^.+ \d{4}-\d{2}-\d{2} \d{2}(-\d{2}(-\d{2})?|h\d{2}(m\d{2}s)?)( (מלא|אינקרמנטלי))?(\.partial)?$").unwrap();
    re.is_match(name)
}

/// A command line from `args`. Ours never end in a backslash or hold quotes, so plain quoting is enough.
pub fn command_line(args: &[String]) -> String {
    args.iter().map(|a| format!("\"{}\"", a.trim_end_matches('\\'))).collect::<Vec<_>>().join(" ")
}

/// Starts the app again as admin with `args` (Windows shows its permission prompt) and returns once
/// that copy has finished, with its exit code. Err = the prompt was declined or the copy couldn't
/// start. Blocks while the prompt is open. Use `elevation::run_as_admin`, which skips the prompt
/// when the user gave the app permanent permission.
pub fn run_elevated(args: &[String]) -> Result<u32, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(exe.as_os_str());
    let params = wide(std::ffi::OsStr::new(&command_line(args)));
    let mut info = ShellExecuteInfoW {
        cb_size: std::mem::size_of::<ShellExecuteInfoW>() as u32,
        f_mask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        hwnd: 0,
        lp_verb: verb.as_ptr(),
        lp_file: file.as_ptr(),
        lp_parameters: params.as_ptr(),
        lp_directory: std::ptr::null(),
        n_show: 0,
        h_inst_app: 0,
        lp_id_list: std::ptr::null_mut(),
        lp_class: std::ptr::null(),
        hkey_class: 0,
        dw_hot_key: 0,
        h_icon_or_monitor: 0,
        h_process: 0,
    };
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let e = io::Error::last_os_error();
        return Err(if e.raw_os_error() == Some(ERROR_CANCELLED) {
            "האישור של Windows בוטל".into()
        } else {
            format!("לא ניתן להפעיל פעולה בהרשאת מנהל: {e}")
        });
    }
    let mut code = 0;
    if info.h_process != 0 {
        unsafe {
            WaitForSingleObject(info.h_process, INFINITE);
            GetExitCodeProcess(info.h_process, &mut code);
            CloseHandle(info.h_process);
        }
    }
    Ok(code)
}

/// Deletes `paths` as admin (Windows asks first, unless the user gave permanent permission).
/// Returns once that copy has finished.
pub fn delete_elevated(paths: &[String]) -> Result<(), String> {
    let mut args = vec![DELETE_SWITCH.to_string()];
    args.extend(paths.iter().cloned());
    crate::elevation::run_as_admin(&args).map(|_| ()).map_err(|e| {
        if e.contains("בוטל") {
            "האישור של Windows בוטל - התיקיות לא נמחקו".into()
        } else {
            e
        }
    })
}

/// Turns on the privileges an admin copy needs to read and write files regardless of their permissions.
pub fn enable_backup_privileges() {
    if let Some(t) = Token::open(TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES) {
        for p in ["SeTakeOwnershipPrivilege", "SeRestorePrivilege", "SeBackupPrivilege"] {
            t.enable(p);
        }
    }
}

// ---- what the admin copy may do ----
//
// Started through the permanent permission (the Task Scheduler task, `elevation.rs`) the admin copy
// runs silently, so anything running as the user could ask it for things. It is therefore bounded:
// it does no more than the user could do themselves, except what only admin rights allow (reading a
// file another program holds open through a shadow copy, deleting a folder whose permissions are
// wrong). Every file operation that doesn't need admin rights is done as the user (`as_user`), so a
// path in a request is never trusted: the user's own permissions decide. Asked through Windows's
// prompt the user has agreed to this very request, so only the safety checks apply (`BOUNDED` off).

static BOUNDED: AtomicBool = AtomicBool::new(false);

/// Turns the bounds on (the admin copy started by the task does this first thing).
pub fn set_bounded(on: bool) {
    BOUNDED.store(on, Ordering::SeqCst);
}

pub fn is_bounded() -> bool {
    BOUNDED.load(Ordering::SeqCst)
}

/// The signed-in user's own (not elevated) token, copied from the desktop shell - an impersonation
/// token, closed never (the admin copy is short-lived). None = no shell is running.
fn shell_user_token() -> Option<isize> {
    static TOKEN: OnceLock<Option<isize>> = OnceLock::new();
    *TOKEN.get_or_init(|| unsafe {
        let hwnd = GetShellWindow();
        if hwnd == 0 {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process == 0 {
            return None;
        }
        let mut shell = 0isize;
        let ok = OpenProcessToken(process, TOKEN_DUPLICATE | TOKEN_QUERY, &mut shell) != 0;
        CloseHandle(process);
        if !ok {
            return None;
        }
        let mut copy = 0isize;
        let ok = DuplicateTokenEx(shell, TOKEN_IMPERSONATE | TOKEN_QUERY, std::ptr::null(), SECURITY_IMPERSONATION, TOKEN_TYPE_IMPERSONATION, &mut copy) != 0;
        CloseHandle(shell);
        ok.then_some(copy)
    })
}

/// Reverts the thread to the process's own token when dropped.
struct Impersonating;

impl Drop for Impersonating {
    fn drop(&mut self) {
        unsafe { SetThreadToken(std::ptr::null(), 0) };
    }
}

/// Runs `f` as the signed-in user (their rights, not the admin's). Without a shell to take the
/// user from: refused when bounded, run as is otherwise (asked through Windows's prompt).
pub fn as_user<T>(f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    match shell_user_token() {
        Some(token) => {
            if unsafe { SetThreadToken(std::ptr::null(), token) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let _back = Impersonating;
            f()
        }
        None if is_bounded() => Err(io::Error::new(io::ErrorKind::PermissionDenied, "לא נמצא משתמש מחובר שאפשר לפעול בשמו")),
        None => f(),
    }
}

/// A path the admin copy accepts: absolute, and nothing in it that moves it somewhere else (`..`, `.`).
/// A name check on the text of a path means nothing when the path can climb out of the folder named.
pub fn plain_absolute(p: &Path) -> bool {
    p.is_absolute() && p.components().all(|c| !matches!(c, Component::ParentDir | Component::CurDir))
}

/// Whether the user could read `path` themselves: opening it as them is allowed, or fails only
/// because another program has it open (the case the shadow copy is for).
pub fn user_can_read(path: &Path) -> bool {
    match as_user(|| std::fs::File::open(path).map(drop)) {
        Ok(()) => true,
        // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION: the permission check passed, the lock didn't.
        Err(e) => matches!(e.raw_os_error(), Some(32 | 33)),
    }
}

/// Whether the user may delete things inside `dir` (the right to delete a child, the one a delete
/// of a backup folder in it needs).
fn user_can_delete_in(dir: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    as_user(|| {
        std::fs::OpenOptions::new()
            .access_mode(FILE_LIST_DIRECTORY | FILE_DELETE_CHILD)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(dir)
            .map(drop)
    })
    .is_ok()
}

fn is_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

/// What a delete request asks for, judged. `Ok(true)` = go ahead, `Ok(false)` = not ours / not
/// there (skipped quietly, as before), `Err` = the bounds refuse it.
fn judge_delete(path: &Path) -> Result<bool, ()> {
    let ours = path.file_name().and_then(|n| n.to_str()).is_some_and(is_backup_folder_name);
    // A link (junction, symlink) named like a backup folder would send the ownership change and
    // the delete somewhere else; a path with `..` in it names something other than it says.
    if !ours || !plain_absolute(path) || is_reparse_point(path) || !path.is_dir() {
        return Ok(false);
    }
    if is_bounded() && !path.parent().is_some_and(user_can_delete_in) {
        return Err(());
    }
    Ok(true)
}

/// The admin copy: `args` = the folders to delete. Takes ownership, grants itself full control, deletes.
/// Exit code = how many folders are left, `REFUSED` when the bounds refused a folder (nothing is deleted then).
pub fn elevated_main(args: &[String]) -> i32 {
    enable_backup_privileges();
    let mut todo = Vec::new();
    for a in args {
        match judge_delete(Path::new(a)) {
            Ok(true) => todo.push(Path::new(a)),
            Ok(false) => {}
            Err(()) => return REFUSED,
        }
    }
    let mut left = 0;
    for path in todo {
        let _ = reset_tree(path, true);
        if crate::engine::native::remove_tree(path).is_err() && path.exists() {
            left += 1;
        }
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dated_backup_folders_count_as_ours() {
        for ok in [
            "מסמכים ענבל 2026-11-10 14-25 מלא",
            "Docs 2026-11-10 14-25-07 אינקרמנטלי",
            "Docs 2026-11-10 14-25 מלא.partial",
            "Docs 2026-11-10 14-25",
        ] {
            assert!(is_backup_folder_name(ok), "{ok}");
        }
        for bad in ["Docs", "Windows", "Docs 2026-11-10", "2026-11-10 14-25 מלא", "Docs 2026-11-10 14-25 other"] {
            assert!(!is_backup_folder_name(bad), "{bad}");
        }
    }

    #[test]
    fn paths_that_climb_out_are_not_plain() {
        assert!(plain_absolute(Path::new(r"C:\Backups\Docs 2026-11-10 14-25 מלא")));
        assert!(!plain_absolute(Path::new(r"C:\Backups\Docs 2026-11-10 14-25 מלא\..\..\Windows\x.dll")));
        assert!(!plain_absolute(Path::new(r"Backups\Docs 2026-11-10 14-25 מלא")));
    }

    fn junction(link: &Path, target: &Path) {
        let ok = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(link).arg(target).output().unwrap().status.success();
        assert!(ok, "mklink /J");
    }

    #[test]
    fn a_link_named_like_a_backup_folder_is_not_deleted_through() {
        let root = std::env::temp_dir().join(format!("backuper-acl-{}", uuid::Uuid::new_v4()));
        let target = root.join("precious");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("keep.txt"), "x").unwrap();
        let link = root.join("Docs 2026-11-10 14-25 מלא");
        junction(&link, &target);
        assert_eq!(judge_delete(&link), Ok(false));
        assert_eq!(elevated_main(&[link.to_string_lossy().to_string()]), 0);
        assert!(target.join("keep.txt").exists());
        // The same name as a real folder is fine.
        let real = root.join("Docs 2026-11-11 14-25 מלא");
        std::fs::create_dir_all(&real).unwrap();
        assert_eq!(judge_delete(&real), Ok(true));
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The permission reset of a backup folder must not reach through a junction inside it into
    /// whatever the junction points at (the admin copy runs it with ownership rights).
    #[test]
    fn the_permission_reset_does_not_follow_a_junction_inside() {
        let root = std::env::temp_dir().join(format!("backuper-acl-{}", uuid::Uuid::new_v4()));
        let outside = root.join("outside");
        let backup = root.join("Docs 2026-11-10 14-25 מלא");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(&backup).unwrap();
        junction(&backup.join("link"), &outside);
        // `outside`: no inherited rights, read only for the user - a file can't be created in it.
        let me = format!("{}:(OI)(CI)R", std::env::var("USERNAME").unwrap());
        let ok = std::process::Command::new("icacls").arg(&outside).args(["/inheritance:r", "/grant:r", &me]).output().unwrap().status.success();
        assert!(ok, "icacls");
        assert!(std::fs::write(outside.join("probe.txt"), "x").is_err(), "outside should be read only before");
        grant_own_tree(&backup).unwrap();
        let reached = std::fs::write(outside.join("probe.txt"), "x").is_ok();
        // Put the folder back in a state the cleanup can remove.
        let _ = std::process::Command::new("icacls").arg(&outside).args(["/reset", "/T"]).output();
        let _ = std::fs::remove_dir(backup.join("link"));
        let _ = std::fs::remove_dir_all(&root);
        assert!(!reached, "the reset followed the junction");
    }
}
