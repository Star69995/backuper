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
use std::path::Path;

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
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> isize;
    fn CloseHandle(h: isize) -> i32;
    fn WaitForSingleObject(h: isize, ms: u32) -> u32;
    fn GetExitCodeProcess(h: isize, code: *mut u32) -> i32;
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

/// Starts the app again as admin with `params` as its command line (Windows shows its permission
/// prompt) and returns once that copy has finished, with its exit code. Err = the prompt was
/// declined or the copy couldn't start. Blocks while the prompt is open.
pub fn run_elevated(params: &str) -> Result<u32, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(exe.as_os_str());
    let params = wide(std::ffi::OsStr::new(params));
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

/// Starts the app again as admin to delete `paths`. Returns once that copy has finished.
pub fn delete_elevated(paths: &[String]) -> Result<(), String> {
    // Our paths never end in a backslash or hold quotes, so plain quoting is enough.
    let mut params = DELETE_SWITCH.to_string();
    for p in paths {
        params.push_str(&format!(" \"{}\"", p.trim_end_matches('\\')));
    }
    run_elevated(&params).map(|_| ()).map_err(|e| {
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

/// The admin copy: `args` = the folders to delete. Takes ownership, grants full control, deletes.
/// Exit code = how many folders are left.
pub fn elevated_main(args: &[String]) -> i32 {
    enable_backup_privileges();
    let mut left = 0;
    for a in args {
        let path = Path::new(a);
        let ours = path.file_name().and_then(|n| n.to_str()).is_some_and(is_backup_folder_name);
        if !ours || !path.is_dir() {
            continue;
        }
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
}
