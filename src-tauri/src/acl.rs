//! Windows permissions on backup folders that won't delete.
//!
//! Backups don't copy permissions (robocopy `/COPY:DAT`), so their files get the destination's.
//! Still, a delete can be refused: the destination's own permissions are restrictive (a drive set
//! up on another PC), or the files belong to another account. Two ways out:
//! - `grant_own_tree`: the files are ours. An owner may always change an item's permissions, so the
//!   tree gets one that gives us full control - no admin needed.
//! - `delete_elevated`: they aren't. The app starts itself again as admin (Windows asks the user,
//!   UAC), and that copy (`elevated_main`) takes ownership, grants itself full control and deletes.

use crate::backup;
use crate::model::Task;
use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The command-line switch of the admin copy that deletes the folders given after it.
pub const DELETE_SWITCH: &str = "--delete-elevated";
/// The app's data folder under AppData\Roaming (`identifier` in tauri.conf.json).
const APP_ID: &str = "org.tovtech.backuper";

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
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
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> i32;
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationProcess(process: isize, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
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
        Token::of(unsafe { GetCurrentProcess() }, access)
    }

    fn of(process: isize, access: u32) -> Option<Token> {
        let mut h = 0;
        (unsafe { OpenProcessToken(process, access, &mut h) } != 0).then_some(Token(h))
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

/// Turns on privileges of an admin copy (they let it work on files regardless of their
/// permissions). Each admin copy asks only for the ones its job needs.
pub fn enable_privileges(names: &[&str]) {
    if let Some(t) = Token::open(TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES) {
        for p in names {
            t.enable(p);
        }
    }
}

/// The account that started this process, as a SID: for an admin copy, the user who asked for it
/// (Windows starts the admin copy as a child of the asking process).
fn launcher_sid() -> Option<Vec<u32>> {
    // PROCESS_BASIC_INFORMATION: six pointer-sized fields, the last one is the parent's id.
    let mut info = [0usize; 6];
    let status = unsafe {
        NtQueryInformationProcess(GetCurrentProcess(), 0, info.as_mut_ptr().cast(), std::mem::size_of_val(&info) as u32, std::ptr::null_mut())
    };
    if status != 0 {
        return None;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, info[5] as u32) };
    if process == 0 {
        return None;
    }
    let sid = Token::of(process, TOKEN_QUERY).and_then(|t| t.user_sid());
    unsafe { CloseHandle(process) };
    sid
}

/// "S-1-5-21-...".
fn sid_string(sid: &[u32]) -> Option<String> {
    let mut p: *mut u16 = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid.as_ptr().cast(), &mut p) } == 0 {
        return None;
    }
    let s = unsafe {
        let len = (0..).take_while(|&i| *p.add(i) != 0).count();
        String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
    };
    unsafe { LocalFree(p.cast()) };
    Some(s)
}

/// Where the app of the user who started this process keeps its data. Usually that user is us.
/// When an admin of another account typed their password into Windows's prompt, the admin copy
/// runs as that admin, and the data is in the asking user's profile.
fn launcher_data_dir() -> Option<PathBuf> {
    let own = Token::open(TOKEN_QUERY)?.user_sid()?;
    let roaming = match launcher_sid() {
        Some(sid) if sid != own => {
            let key = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
                .open_subkey(format!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\{}", sid_string(&sid)?))
                .ok()?;
            let profile: String = key.get_value("ProfileImagePath").ok()?;
            if profile.contains('%') {
                return None;
            }
            PathBuf::from(profile).join(r"AppData\Roaming")
        }
        _ => dirs::data_dir()?,
    };
    Some(roaming.join(APP_ID))
}

/// What an admin copy may touch. It is started with a command line, and any program of the user
/// can start it with its own (Windows's prompt looks the same), so the command line is checked
/// against the saved tasks: only their backup folders, and only files of their sources.
/// The task list is a file the user's programs can write, so this narrows what a request can
/// reach rather than closing it; what it does close is reaching other places by name alone.
pub struct Allowed(pub(crate) Option<Vec<Task>>);

impl Allowed {
    pub fn of(tasks: &[Task]) -> Allowed {
        Allowed(Some(tasks.to_vec()))
    }

    /// For an admin copy: the task list of the user who asked. When it can't be read (None), only
    /// the folder names are checked, as before the list was.
    pub fn load() -> Allowed {
        let tasks = launcher_data_dir()
            .and_then(|d| std::fs::read(d.join("tasks.json")).ok())
            .and_then(|b| serde_json::from_slice::<Vec<Task>>(&b).ok())
            .map(|mut tasks| {
                tasks.iter_mut().for_each(Task::migrate);
                tasks
            });
        Allowed(tasks)
    }

    /// The source folder that `folder` is a backup of: `folder` sits directly in a task's
    /// destination and is named like the dated folders of one of its sources.
    fn source_of(&self, folder: &Path) -> Option<&str> {
        let name = folder.file_name()?.to_str()?;
        let dest = backup::normalize(&folder.parent()?.to_string_lossy());
        self.0
            .as_ref()?
            .iter()
            .filter(|t| backup::normalize(&t.destination) == dest)
            .flat_map(|t| &t.sources)
            .find(|s| {
                std::iter::once(&s.folder_name)
                    .chain(&s.previous_names)
                    .any(|n| backup::is_dated(&backup::sanitize_folder_name(n), name))
            })
            .map(|s| s.path.as_str())
    }

    /// Whether `folder` is a backup folder that may be deleted as admin.
    pub fn folder(&self, folder: &Path) -> bool {
        let named = folder.file_name().and_then(|n| n.to_str()).is_some_and(is_backup_folder_name);
        named && (self.0.is_none() || self.source_of(folder).is_some())
    }

    /// Whether `folder` is a backup folder of the source folder `source`.
    pub fn backup_of(&self, source: &Path, folder: &Path) -> bool {
        self.source_of(folder).is_some_and(|s| backup::normalize(s) == backup::normalize(&source.to_string_lossy()))
    }

    /// The backup folder that a copy of `src` to `dst` goes into, when the copy is one a backup
    /// makes: `dst` is inside a backup folder, at the same place `src` has in that folder's source.
    pub fn copy_root<'a>(&self, src: &Path, dst: &'a Path) -> Option<&'a Path> {
        dst.ancestors().skip(1).find(|folder| match &self.0 {
            None => folder.file_name().and_then(|n| n.to_str()).is_some_and(is_backup_folder_name),
            Some(_) => self.source_of(folder).zip(dst.strip_prefix(folder).ok()).is_some_and(|(source, rel)| {
                backup::normalize(&Path::new(source).join(rel).to_string_lossy()) == backup::normalize(&src.to_string_lossy())
            }),
        })
    }
}

/// The admin copy: `args` = the folders to delete. Takes ownership, grants full control, deletes.
/// Exit code = how many folders are left.
pub fn elevated_main(args: &[String]) -> i32 {
    enable_privileges(&["SeTakeOwnershipPrivilege", "SeRestorePrivilege", "SeBackupPrivilege"]);
    let allowed = Allowed::load();
    let mut left = 0;
    for a in args {
        let path = Path::new(a);
        // A link named like a backup folder leads somewhere else: never through it.
        let ours = allowed.folder(path) && crate::engine::native::find(path).is_some_and(|i| i.is_dir() && !i.is_link());
        if !ours {
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

    fn allowed() -> Allowed {
        use crate::model::Source;
        Allowed::of(&[Task {
            destination: r"K:\Backup".into(),
            sources: vec![Source {
                path: r"C:\Users\me\Documents".into(),
                folder_name: "Docs".into(),
                previous_names: vec!["Old".into()],
                ..Default::default()
            }],
            ..Default::default()
        }])
    }

    #[test]
    fn only_folders_of_saved_tasks_are_deleted_as_admin() {
        let a = allowed();
        assert!(a.folder(Path::new(r"K:\Backup\Docs 2026-11-10 14h25 מלא")));
        assert!(a.folder(Path::new(r"k:\backup\Docs 2026-11-10 14h25 אינקרמנטלי.partial")));
        assert!(a.folder(Path::new(r"K:\Backup\Old 2026-11-10 14-25")));
        // Named like a backup folder, but not of a task: another name, another place, deeper.
        assert!(!a.folder(Path::new(r"K:\Backup\Other 2026-11-10 14h25 מלא")));
        assert!(!a.folder(Path::new(r"C:\Windows\Docs 2026-11-10 14h25 מלא")));
        assert!(!a.folder(Path::new(r"K:\Backup\x\Docs 2026-11-10 14h25 מלא")));
        assert!(!a.folder(Path::new(r"K:\Backup")));
        // No task list to check against: the name alone, as before.
        assert!(Allowed(None).folder(Path::new(r"C:\x\Other 2026-11-10 14h25 מלא")));
        assert!(!Allowed(None).folder(Path::new(r"C:\Windows")));
    }

    #[test]
    fn a_copy_as_admin_goes_from_a_source_to_its_place_in_its_backup_folder() {
        let a = allowed();
        let folder = r"K:\Backup\Docs 2026-11-10 14h25 מלא.partial";
        let dst = PathBuf::from(format!(r"{folder}\Outlook\mail.pst"));
        assert_eq!(a.copy_root(Path::new(r"c:\users\me\documents\Outlook\mail.pst"), &dst), Some(Path::new(folder)));
        // Another file of the computer, or the right file to another place in the folder.
        assert_eq!(a.copy_root(Path::new(r"C:\Windows\System32\config\SAM"), &dst), None);
        assert_eq!(a.copy_root(Path::new(r"C:\Users\me\Documents\mail.pst"), &dst), None);
        let elsewhere = PathBuf::from(r"C:\x\Docs 2026-11-10 14h25 מלא\Outlook\mail.pst");
        assert_eq!(a.copy_root(Path::new(r"C:\Users\me\Documents\Outlook\mail.pst"), &elsewhere), None);
        assert!(a.backup_of(Path::new(r"C:\Users\me\Documents\"), Path::new(folder)));
        assert!(!a.backup_of(Path::new(r"C:\Users\me"), Path::new(folder)));
    }

    #[test]
    fn the_admin_copy_finds_the_asking_users_data() {
        // The tests' parent (cargo) runs as the same user, like the app that asks for an admin copy.
        let own = Token::open(TOKEN_QUERY).and_then(|t| t.user_sid()).unwrap();
        assert_eq!(launcher_sid(), Some(own.clone()));
        assert_eq!(launcher_data_dir(), dirs::data_dir().map(|d| d.join(APP_ID)));
        assert!(sid_string(&own).is_some_and(|s| s.starts_with("S-1-5-")));
        assert!(include_str!("../tauri.conf.json").contains(&format!("\"identifier\": \"{APP_ID}\"")));
    }
}
