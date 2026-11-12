//! Permanent admin permission: what needs admin rights (open files through VSS, `vss.rs`; backup
//! folders that won't delete, `acl.rs`) runs without Windows's permission prompt, so a scheduled
//! backup with nobody at the computer doesn't wait for a "yes" that never comes.
//!
//! The user approves once (`grant`: one prompt). That admin copy (`install_main`) puts a copy of
//! the app in Program Files (`helper_exe`: a folder only admins can write to, so nothing running
//! as the user can swap the program that runs as admin) and registers a Task Scheduler task that
//! runs it as the same user with the highest rights, without a prompt (`task_xml`: no trigger,
//! started on demand; the user may read and run it, not change it). It stays across restarts and
//! updates, until `revoke` or the uninstaller (`--admin-uninstall`).
//!
//! A request (`run_as_admin`) goes through a control file in the user's temp folder: the app
//! writes `<control>.req` (the admin copy's command line, JSON), starts the task with the control
//! path as `$(Arg0)` (PowerShell, the Task Scheduler's `RunEx`), the helper takes the request
//! (renames it to `.taken`), keeps `.alive` fresh while it works and writes `.exit`. A task that
//! doesn't start, or a permission that isn't there, falls back to the prompt.
//!
//! `HELPER_VERSION` = what the helper understands; a helper of another version (`version.txt`
//! beside it) counts as `Outdated` until the user approves again, which copies this version.

use crate::acl;
use crate::model::AdminPermission;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// The task's command line: `--admin-task <control path>`.
pub const TASK_SWITCH: &str = "--admin-task";
/// The admin copy that registers the task: `--admin-task-install <control path> <user SID>`.
pub const INSTALL_SWITCH: &str = "--admin-task-install";
/// The admin copy that removes it: `--admin-task-remove <control path> <user SID>`.
pub const REMOVE_SWITCH: &str = "--admin-task-remove";
/// The uninstaller (not elevated): removes the permission if there is one (Windows asks).
pub const UNINSTALL_SWITCH: &str = "--admin-uninstall";
/// A request that only checks the helper really runs as admin (exit code 0).
const PING: &str = "--admin-ping";
/// Raise when what the helper understands changes (switches, request format).
const HELPER_VERSION: &str = "1";
/// How long a task that was started may take to pick its request up.
const START_WAIT: Duration = Duration::from_secs(30);
/// The helper refreshes `.alive` every few seconds; this long without = it is gone.
const ALIVE_WAIT: Duration = Duration::from_secs(30);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static STATUS: Mutex<AdminPermission> = Mutex::new(AdminPermission::Unknown);

pub const NOT_ADMIN: &str = "חשבון Windows שלכם הוא משתמש רגיל, לא מנהל. הרשאה קבועה אפשרית רק בחשבון מנהל - בחשבון רגיל Windows מבקשת סיסמת מנהל בכל פעם";

/// The last known state (`refresh` reads it from Windows).
pub fn cached() -> AdminPermission {
    *STATUS.lock().unwrap()
}

/// Reads the state from Windows (the task and the helper; a fraction of a second) and remembers it.
pub fn refresh() -> AdminPermission {
    let status = check();
    *STATUS.lock().unwrap() = status;
    status
}

fn check() -> AdminPermission {
    if !acl::is_admin_account() {
        return AdminPermission::NotAdmin;
    }
    let Some(sid) = acl::user_sid_string() else { return AdminPermission::Off };
    let Some(xml) = query_task(&sid) else { return AdminPermission::Off };
    let exe = helper_exe(&sid);
    let points_to_helper = xml.to_lowercase().contains(&exe.to_string_lossy().to_lowercase());
    let version = std::fs::read_to_string(helper_dir(&sid).join("version.txt")).unwrap_or_default();
    if points_to_helper && exe.is_file() && version.trim() == HELPER_VERSION {
        AdminPermission::On
    } else {
        AdminPermission::Outdated
    }
}

fn hidden(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

/// The task's XML as Windows has it, None = no such task (or not readable).
fn query_task(sid: &str) -> Option<String> {
    let out = hidden(Command::new("schtasks.exe").args(["/query", "/tn", &task_path(sid), "/xml"])).output().ok()?;
    out.status.success().then(|| decode(&out.stdout))
}

/// schtasks's output: UTF-16 when it says so, else 8-bit (the parts we look for are ASCII).
fn decode(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes.iter().skip(1).step_by(2).filter(|b| **b == 0).count() * 2 > bytes.len() / 2 {
        let words: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&words)
    } else {
        String::from_utf8_lossy(bytes).to_string()
    }
}

const TASK_FOLDER: &str = "Backuper";

/// One task per Windows account (each runs as its own user).
fn task_name(sid: &str) -> String {
    format!("Admin {sid}")
}

fn task_path(sid: &str) -> String {
    format!("\\{TASK_FOLDER}\\{}", task_name(sid))
}

fn program_files() -> PathBuf {
    std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
}

fn helper_dir(sid: &str) -> PathBuf {
    program_files().join("Backuper").join(format!("admin-{sid}"))
}

fn helper_exe(sid: &str) -> PathBuf {
    helper_dir(sid).join("backuper-admin.exe")
}

fn control_path() -> PathBuf {
    std::env::temp_dir().join(format!("backuper-admin-{}", uuid::Uuid::new_v4()))
}

fn side(control: &Path, ext: &str) -> PathBuf {
    let mut s = control.as_os_str().to_os_string();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

fn cleanup(control: &Path) {
    for ext in ["req", "taken", "alive", "exit", "result", "xml"] {
        let _ = std::fs::remove_file(side(control, ext));
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Who may do what with the task: admins and SYSTEM everything, the user read and run it - not
/// change what it runs.
fn task_sddl(sid: &str) -> String {
    format!("D:P(A;;FA;;;BA)(A;;FA;;;SY)(A;;GRGX;;;{sid})")
}

/// The task: no trigger (only started on demand), runs as `sid` with the highest rights that
/// account has, only while it is logged on (no password stored).
fn task_xml(sid: &str, exe: &Path) -> String {
    let sid = xml_escape(sid);
    let exe = xml_escape(&exe.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Author>Backuper</Author>
    <Description>Backuper: גיבוי קבצים פתוחים (VSS) ומחיקת גיבויים ישנים בהרשאת מנהל, בלי לבקש אישור בכל פעם. נוצר באישור המשתמש, ואפשר לבטל בהגדרות של Backuper.</Description>
  </RegistrationInfo>
  <Principals>
    <Principal id="Author">
      <UserId>{sid}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>6</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>{TASK_SWITCH} "$(Arg0)"</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

// ---- used by the app ----

/// Runs the admin copy of the app with `args` (`vss::SWITCH ...`, `acl::DELETE_SWITCH ...`) and
/// returns its exit code once it is done. With permanent permission through the task, no prompt;
/// otherwise (or when the task doesn't start) Windows asks. Blocks until done.
pub fn run_as_admin(args: &[String]) -> Result<u32, String> {
    if cached() == AdminPermission::On {
        match through_task(args) {
            Ok(code) => return Ok(code),
            Err(TaskError::Failed(e)) => return Err(e),
            // Removed or broken behind our back: read the state again, and ask this time.
            Err(TaskError::NotStarted(_)) => {
                refresh();
            }
        }
    }
    acl::run_elevated(args)
}

enum TaskError {
    /// Nothing ran: the caller may ask Windows instead.
    NotStarted(String),
    /// The helper took the request and then stopped.
    Failed(String),
}

fn through_task(args: &[String]) -> Result<u32, TaskError> {
    let sid = acl::user_sid_string().ok_or_else(|| TaskError::NotStarted("לא ניתן לזהות את חשבון Windows".into()))?;
    let control = control_path();
    let req = side(&control, "req");
    let json = serde_json::to_string(args).unwrap_or_default();
    std::fs::write(&req, json).map_err(|e| TaskError::NotStarted(e.to_string()))?;
    if let Err(e) = start_task(&sid, &control) {
        cleanup(&control);
        return Err(TaskError::NotStarted(e));
    }
    let started = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(200));
        let exit = side(&control, "exit");
        if exit.exists() {
            let code = std::fs::read_to_string(&exit).ok().and_then(|t| t.trim().parse().ok());
            cleanup(&control);
            return code.ok_or_else(|| TaskError::Failed("הפעולה בהרשאת מנהל לא דיווחה על התוצאה".into()));
        }
        if req.exists() {
            // Not picked up yet. If taking it back works, the helper never will.
            if started.elapsed() > START_WAIT && std::fs::remove_file(&req).is_ok() {
                cleanup(&control);
                return Err(TaskError::NotStarted("המשימה המתוזמנת של ההרשאה הקבועה לא התחילה".into()));
            }
            continue;
        }
        let fresh = std::fs::metadata(side(&control, "alive"))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .is_some_and(|age| age < ALIVE_WAIT);
        if !fresh && started.elapsed() > START_WAIT {
            cleanup(&control);
            return Err(TaskError::Failed("הפעולה בהרשאת מנהל נקטעה באמצע".into()));
        }
    }
}

/// Starts the task with `control` as its argument (Task Scheduler `RunEx`; schtasks can't pass one).
fn start_task(sid: &str, control: &Path) -> Result<(), String> {
    let q = |s: &str| s.replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop'; $s = New-Object -ComObject Schedule.Service; $s.Connect(); \
         $t = $s.GetFolder('\\{TASK_FOLDER}').GetTask('{}'); $null = $t.RunEx('{}', 0, 0, '')",
        q(&task_name(sid)),
        q(&control.to_string_lossy()),
    );
    let out = hidden(Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script]))
        .output()
        .map_err(|e| format!("לא ניתן להפעיל את PowerShell: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Gives the app permanent permission: one Windows prompt, then a check that the task runs as
/// admin without one. Blocks while the prompt is open.
pub fn grant() -> Result<(), String> {
    if !acl::is_admin_account() {
        refresh();
        return Err(NOT_ADMIN.into());
    }
    let sid = acl::user_sid_string().ok_or("לא ניתן לזהות את חשבון Windows")?;
    admin_step(INSTALL_SWITCH, &sid).inspect_err(|_| {
        refresh();
    })?;
    // Proof: a request through the task, no prompt, comes back as admin.
    let ping = through_task(&[PING.to_string()]);
    let status = refresh();
    match ping {
        Ok(0) if status == AdminPermission::On => Ok(()),
        Ok(_) => Err("ההרשאה נרשמה, אבל הבדיקה רצה בלי הרשאת מנהל. ודאו שחשבון Windows שלכם הוא חשבון מנהל ונסו שוב".into()),
        Err(TaskError::NotStarted(e) | TaskError::Failed(e)) => Err(format!("ההרשאה נרשמה, אבל הבדיקה נכשלה: {e}. נסו שוב")),
    }
}

/// Takes the permission back (Windows asks: removing an admin task needs admin too).
pub fn revoke() -> Result<(), String> {
    let sid = acl::user_sid_string().ok_or("לא ניתן לזהות את חשבון Windows")?;
    let result = admin_step(REMOVE_SWITCH, &sid);
    refresh();
    result
}

/// Runs `switch` as admin through Windows's prompt (never through the task: installing goes
/// first, and removing must work even when the task is broken) and reads its report.
fn admin_step(switch: &str, sid: &str) -> Result<(), String> {
    let control = control_path();
    let args = [switch.to_string(), control.to_string_lossy().to_string(), sid.to_string()];
    let ran = acl::run_elevated(&args);
    let report = std::fs::read_to_string(side(&control, "result")).unwrap_or_default();
    cleanup(&control);
    ran?;
    match report.trim().split_once('\t') {
        _ if report.trim() == "OK" => Ok(()),
        Some(("ERR", msg)) => Err(msg.to_string()),
        _ => Err("הפעולה בהרשאת מנהל לא הסתיימה כמו שצריך".into()),
    }
}

/// The uninstaller: removes the permission if this account has one (Windows asks). Exit code 0.
pub fn uninstall_main() -> i32 {
    if let Some(sid) = acl::user_sid_string() {
        if query_task(&sid).is_some() || helper_dir(&sid).exists() {
            let _ = admin_step(REMOVE_SWITCH, &sid);
        }
    }
    0
}

// ---- the admin side ----

fn report(control: &Path, result: Result<(), String>) -> i32 {
    let (text, code) = match result {
        Ok(()) => ("OK".to_string(), 0),
        Err(e) => (format!("ERR\t{}", e.replace(['\r', '\n', '\t'], " ")), 1),
    };
    let _ = std::fs::write(side(control, "result"), text);
    code
}

/// `args` = control path, user SID. Copies this exe beside the helper's version file in Program
/// Files and registers the task.
pub fn install_main(args: &[String]) -> i32 {
    let [control, sid] = args else { return 2 };
    let control = Path::new(control);
    report(control, install(control, sid))
}

fn valid_sid(sid: &str) -> bool {
    sid.starts_with("S-1-") && sid.chars().all(|c| c.is_ascii_digit() || c == '-' || c == 'S')
}

fn install(control: &Path, sid: &str) -> Result<(), String> {
    if !acl::is_elevated() {
        return Err("הפעולה לא קיבלה הרשאת מנהל".into());
    }
    if !valid_sid(sid) {
        return Err("זיהוי החשבון לא תקין".into());
    }
    let dir = helper_dir(sid);
    std::fs::create_dir_all(&dir).map_err(|e| format!("לא ניתן ליצור את התיקייה {}: {e}", dir.display()))?;
    let exe = helper_exe(sid);
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    if exe.exists() {
        // A helper that is running can't be overwritten, but can be renamed out of the way.
        let old = dir.join("backuper-admin.old");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(&exe, &old);
        let _ = std::fs::remove_file(&old);
    }
    std::fs::copy(&me, &exe).map_err(|e| format!("לא ניתן להעתיק את התוכנה אל {}: {e}", dir.display()))?;
    std::fs::write(dir.join("version.txt"), HELPER_VERSION).map_err(|e| e.to_string())?;
    // Through the Task Scheduler's own API (RegisterTask), which takes the access rules as they are.
    let xml_path = side(control, "xml");
    let xml = task_xml(sid, &exe);
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
    std::fs::write(&xml_path, bytes).map_err(|e| e.to_string())?;
    let q = |s: &str| s.replace('\'', "''");
    // 6 = TASK_CREATE_OR_UPDATE, 3 = TASK_LOGON_INTERACTIVE_TOKEN.
    let script = format!(
        "$ErrorActionPreference='Stop'; $s = New-Object -ComObject Schedule.Service; $s.Connect(); \
         try {{ $f = $s.GetFolder('\\{TASK_FOLDER}') }} catch {{ $f = $s.GetFolder('\\').CreateFolder('{TASK_FOLDER}') }}; \
         $xml = Get-Content -Raw -Encoding Unicode -LiteralPath '{}'; \
         $null = $f.RegisterTask('{}', $xml, 6, $null, $null, 3, '{}')",
        q(&xml_path.to_string_lossy()),
        q(&task_name(sid)),
        q(&task_sddl(sid)),
    );
    let result = crate::vss::powershell(&script);
    let _ = std::fs::remove_file(&xml_path);
    result.map(|_| ()).map_err(|e| format!("מתזמן המשימות של Windows לא רשם את ההרשאה: {e}"))
}

/// `args` = control path, user SID. Removes the task and the helper.
pub fn remove_main(args: &[String]) -> i32 {
    let [control, sid] = args else { return 2 };
    let control = Path::new(control);
    report(control, remove(sid))
}

fn remove(sid: &str) -> Result<(), String> {
    if !acl::is_elevated() {
        return Err("הפעולה לא קיבלה הרשאת מנהל".into());
    }
    if !valid_sid(sid) {
        return Err("זיהוי החשבון לא תקין".into());
    }
    let mut problem = None;
    if query_task(sid).is_some() {
        let out = hidden(Command::new("schtasks.exe").args(["/delete", "/tn", &task_path(sid), "/f"])).output();
        if !out.as_ref().is_ok_and(|o| o.status.success()) {
            problem = Some("מתזמן המשימות של Windows לא מחק את ההרשאה".to_string());
        }
    }
    let dir = helper_dir(sid);
    if dir.exists() && std::fs::remove_dir_all(&dir).is_err() {
        // A helper still running: gone at the next restart.
        for f in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            delete_at_restart(&f.path());
        }
        delete_at_restart(&dir);
    }
    // The shared parent, once no account has a helper in it.
    if let Some(parent) = dir.parent() {
        let _ = std::fs::remove_dir(parent);
    }
    problem.map_or(Ok(()), Err)
}

fn delete_at_restart(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    const MOVEFILE_DELAY_UNTIL_REBOOT: u32 = 0x4;
    let w: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe { MoveFileExW(w.as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) };
}

/// The task: `args` = the control path. Takes the request, runs it, writes the exit code.
pub fn task_main(args: &[String]) -> i32 {
    let Some(control) = args.first() else { return 2 };
    let control = PathBuf::from(control);
    let taken = side(&control, "taken");
    // Taken back by the app (it gave up waiting), or someone else's.
    if std::fs::rename(side(&control, "req"), &taken).is_err() {
        return 2;
    }
    let request: Vec<String> = std::fs::read_to_string(&taken).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let alive = side(&control, "alive");
    let _ = std::fs::write(&alive, "");
    {
        let alive = alive.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(3));
            let _ = std::fs::write(&alive, "");
        });
    }
    let code = match request.first().map(String::as_str) {
        Some(crate::vss::SWITCH) => crate::vss::elevated_main(&request[1..]),
        Some(acl::DELETE_SWITCH) => acl::elevated_main(&request[1..]),
        Some(PING) => i32::from(!acl::is_elevated()),
        _ => 2,
    };
    let exit = side(&control, "exit");
    let tmp = side(&control, "exit.tmp");
    let _ = std::fs::write(&tmp, code.to_string());
    let _ = std::fs::rename(&tmp, &exit);
    let _ = std::fs::remove_file(&taken);
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_runs_the_helper_with_the_control_path() {
        let xml = task_xml("S-1-5-21-1-2-3-1001", Path::new(r"C:\Program Files\Backuper\admin-S-1-5-21-1-2-3-1001\backuper-admin.exe"));
        assert!(xml.contains(r#"<Arguments>--admin-task "$(Arg0)"</Arguments>"#));
        assert!(xml.contains("<UserId>S-1-5-21-1-2-3-1001</UserId>"));
        assert!(task_sddl("S-1-5-21-1-2-3-1001").ends_with("(A;;GRGX;;;S-1-5-21-1-2-3-1001)"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
    }

    #[test]
    fn only_sids_are_accepted() {
        assert!(valid_sid("S-1-5-21-1-2-3-1001"));
        assert!(!valid_sid("S-1-5\" & calc"));
        assert!(!valid_sid("x"));
    }

    #[test]
    fn schtasks_output_is_read_in_either_encoding() {
        let utf16: Vec<u8> = "<Command>C:\\x</Command>".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode(&utf16), "<Command>C:\\x</Command>");
        assert_eq!(decode(b"<Command>C:\\x</Command>"), "<Command>C:\\x</Command>");
    }
}
