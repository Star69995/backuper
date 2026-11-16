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
//! What the helper may do is bounded, because anything running as the user can start the task
//! (`acl.rs`, "what the admin copy may do"): it does no more than the user could do themselves,
//! except what only admin rights allow. Everything beside the request - the control files - is
//! touched as the user (`acl::as_user`: the path comes from the request) and only made new, the
//! request is checked against the user's saved tasks like every admin copy (`acl::Allowed`; no
//! readable task list = refused), the shadow-copy copy reads the snapshot and writes as the user,
//! a delete needs a real (not linked, not reached through a link) backup folder the user may
//! delete inside, held open while its permissions change (`acl::pin`), and every request is noted
//! in `audit.log` next to the helper (admin-only folder). A request outside the bounds exits with
//! `acl::REFUSED` and the app asks Windows for permission instead, so an unusual case costs one
//! prompt, never a silent action.
//!
//! Full access (`grant(true)`, the user's choice in the same one prompt): the helper is not bounded
//! and never refuses for the reasons above - it does what an admin copy approved through
//! Windows's prompt does (the task-list check and the link checks stay), so no backup ever waits
//! for a prompt. The choice is a marker file beside the helper (`FULL_ACCESS_FILE`, admin-only
//! folder): nothing running as the user can switch it on. Its price: a program running as the
//! user can edit the task list and through it read any file of the computer.
//!
//! `HELPER_VERSION` = what the helper understands; a helper of another version (`version.txt`
//! beside it) counts as `Outdated` until the user approves again, which copies this version.

use crate::acl;
use crate::model::AdminPermission;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
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
const HELPER_VERSION: &str = "3";
/// How long a task that was started may take to pick its request up.
const START_WAIT: Duration = Duration::from_secs(30);
/// The helper refreshes `.alive` every few seconds; this long without = it is gone.
const ALIVE_WAIT: Duration = Duration::from_secs(30);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Beside the helper when the user gave full access (see the module docs).
const FULL_ACCESS_FILE: &str = "full-access";

static STATUS: Mutex<AdminPermission> = Mutex::new(AdminPermission::Unknown);
static FULL_ACCESS: AtomicBool = AtomicBool::new(false);

/// Whether the permission was given with full access (as of the last `refresh`).
pub fn full_access() -> bool {
    FULL_ACCESS.load(Ordering::SeqCst)
}

pub const NOT_ADMIN: &str = "חשבון Windows שלכם הוא משתמש רגיל, לא מנהל. הרשאה קבועה אפשרית רק בחשבון מנהל - בחשבון רגיל Windows מבקשת סיסמת מנהל בכל פעם";

/// The last known state (`refresh` reads it from Windows).
pub fn cached() -> AdminPermission {
    *STATUS.lock().unwrap()
}

/// Reads the state from Windows (the task and the helper; a fraction of a second) and remembers it.
pub fn refresh() -> AdminPermission {
    let status = check();
    let full = status == AdminPermission::On && acl::user_sid_string().is_some_and(|sid| helper_dir(&sid).join(FULL_ACCESS_FILE).is_file());
    FULL_ACCESS.store(full, Ordering::SeqCst);
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
    let out = hidden(Command::new(crate::engine::system_tool("schtasks.exe")).args(["/query", "/tn", &task_path(sid), "/xml"])).output().ok()?;
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

const CONTROL_PREFIX: &str = "backuper-admin-";

fn control_path() -> PathBuf {
    std::env::temp_dir().join(format!("{CONTROL_PREFIX}{}", uuid::Uuid::new_v4()))
}

fn side(control: &Path, ext: &str) -> PathBuf {
    let mut s = control.as_os_str().to_os_string();
    s.push(".");
    s.push(ext);
    PathBuf::from(s)
}

fn cleanup(control: &Path) {
    for ext in ["req", "taken", "alive", "exit", "exit.tmp", "result"] {
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
            // Outside what the helper may do silently: the user is asked, as without the permission.
            Ok(code) if code == acl::REFUSED as u32 => {}
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
    crate::vss::powershell(&script).map(|_| ())
}

/// Gives the app permanent permission: one Windows prompt, then a check that the task runs as
/// admin without one. `full` = full access (see the module docs). Blocks while the prompt is open.
pub fn grant(full: bool) -> Result<(), String> {
    if !acl::is_admin_account() {
        refresh();
        return Err(NOT_ADMIN.into());
    }
    let sid = acl::user_sid_string().ok_or("לא ניתן לזהות את חשבון Windows")?;
    admin_step(INSTALL_SWITCH, &sid, Some(if full { "full" } else { "bounded" })).inspect_err(|_| {
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
    let result = admin_step(REMOVE_SWITCH, &sid, None);
    refresh();
    result
}

/// Runs `switch` as admin through Windows's prompt (never through the task: installing goes
/// first, and removing must work even when the task is broken) and reads its report.
fn admin_step(switch: &str, sid: &str, option: Option<&str>) -> Result<(), String> {
    let control = control_path();
    let mut args = vec![switch.to_string(), control.to_string_lossy().to_string(), sid.to_string()];
    args.extend(option.map(str::to_string));
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
            let _ = admin_step(REMOVE_SWITCH, &sid, None);
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
    // As the user (the path comes from the command line), and only as a new file.
    let _ = acl::as_user(|| crate::vss::write_new(&side(control, "result"), &text));
    code
}

/// `args` = control path, user SID, "full" | "bounded". Copies this exe beside the helper's
/// version file in Program Files and registers the task.
pub fn install_main(args: &[String]) -> i32 {
    let [control, sid, access] = args else { return 2 };
    let control = Path::new(control);
    if !valid_control(control) {
        return 2;
    }
    report(control, install(sid, access == "full"))
}

fn valid_sid(sid: &str) -> bool {
    sid.starts_with("S-1-") && sid.chars().all(|c| c.is_ascii_digit() || c == '-' || c == 'S')
}

fn install(sid: &str, full: bool) -> Result<(), String> {
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
    let marker = dir.join(FULL_ACCESS_FILE);
    if full {
        std::fs::write(&marker, "").map_err(|e| e.to_string())?;
    } else if marker.exists() {
        std::fs::remove_file(&marker).map_err(|e| e.to_string())?;
    }
    // Through the Task Scheduler's own API (RegisterTask), which takes the access rules as they are.
    // The XML waits beside the helper (admin-only folder): nothing running as the user can change
    // what gets registered.
    let xml_path = dir.join("task.xml");
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
    if !valid_control(control) {
        return 2;
    }
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
        let out = hidden(Command::new(crate::engine::system_tool("schtasks.exe")).args(["/delete", "/tn", &task_path(sid), "/f"])).output();
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

/// What a control path must look like: as `control_path` makes them, with nothing in it that
/// moves it somewhere else. The admin copy writes its side files next to it, so it takes no other name.
fn valid_control(control: &Path) -> bool {
    let re = regex::Regex::new(&format!("^{CONTROL_PREFIX}[0-9a-f]{{8}}(-[0-9a-f]{{4}}){{3}}-[0-9a-f]{{12}}$")).unwrap();
    acl::plain_absolute(control) && control.file_name().and_then(|n| n.to_str()).is_some_and(|n| re.is_match(n))
}

/// One line per request in `audit.log` beside the helper, for whoever wonders what ran as admin.
/// The folder is admin-only; the log is cut to its newest half when it gets big.
fn audit(request: &[String], outcome: &str) {
    use std::io::Write;
    let Some(sid) = acl::user_sid_string() else { return };
    let path = helper_dir(&sid).join("audit.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 512 * 1024) {
        let _ = std::fs::rename(&path, path.with_extension("old"));
    }
    let what: String = request.iter().take(8).map(|a| a.replace(['\r', '\n'], " ")).collect::<Vec<_>>().join(" | ");
    let what: String = what.chars().take(600).collect();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}\t{} ({} args)\t{}\t{}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), request.first().map_or("", String::as_str), request.len().saturating_sub(1), outcome, what);
    }
}

/// The task: `args` = the control path. Takes the request, runs it, writes the exit code.
pub fn task_main(args: &[String]) -> i32 {
    // Silent, so bounded (see the module docs), unless the user gave full access: the marker sits
    // beside this exe, in the admin-only folder. Nothing below trusts a path of the request.
    acl::set_from_task();
    let full = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(FULL_ACCESS_FILE).is_file())).unwrap_or(false);
    acl::set_bounded(!full);
    let Some(control) = args.first() else { return 2 };
    let control = PathBuf::from(control);
    if !valid_control(&control) {
        return 2;
    }
    let taken = side(&control, "taken");
    let alive = side(&control, "alive");
    // The control files: touched as the user only. A request to touch someone else's files with the
    // admin's rights (a link in the path, a path into a protected folder) fails here.
    // The side files are new files only, never written over (or through) something already there.
    let (request, mut beating): (Vec<String>, std::fs::File) = match acl::as_user(|| {
        // Taken back by the app (it gave up waiting), or someone else's.
        std::fs::rename(side(&control, "req"), &taken)?;
        let text = std::fs::read_to_string(&taken)?;
        let beating = std::fs::OpenOptions::new().write(true).create_new(true).open(&alive)?;
        Ok((serde_json::from_str(&text).unwrap_or_default(), beating))
    }) {
        Ok(r) => r,
        Err(_) => return 2,
    };
    // The heartbeat goes through the handle opened above, not through the name again.
    std::thread::spawn(move || {
        use std::io::{Seek, Write};
        for beat in 0u64.. {
            let _ = beating.seek(std::io::SeekFrom::Start(0)).and_then(|_| beating.write_all(beat.to_string().as_bytes()));
            let _ = beating.set_modified(SystemTime::now());
            std::thread::sleep(Duration::from_secs(3));
        }
    });
    let code = match request.first().map(String::as_str) {
        Some(crate::vss::SWITCH) => crate::vss::elevated_main(&request[1..]),
        Some(acl::DELETE_SWITCH) => acl::elevated_main(&request[1..]),
        Some(PING) => i32::from(!acl::is_elevated()),
        _ => 2,
    };
    let access = if full { "full access" } else { "bounded" };
    audit(&request, &if code == acl::REFUSED { format!("refused ({access})") } else { format!("exit {code} ({access})") });
    let exit = side(&control, "exit");
    let tmp = side(&control, "exit.tmp");
    let _ = acl::as_user(|| {
        crate::vss::write_new(&tmp, &code.to_string())?;
        std::fs::rename(&tmp, &exit)?;
        let _ = std::fs::remove_file(&taken);
        Ok(())
    });
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
    fn only_the_apps_own_control_paths_are_served() {
        assert!(valid_control(&control_path()));
        assert!(!valid_control(Path::new(r"C:\Users\a\AppData\Local\Temp\backuper-admin-1234")));
        assert!(!valid_control(Path::new(r"C:\Temp\backuper-admin-5f0c2a1e-3b4d-4c5e-8f60-718293a4b5c6.exe")));
        assert!(!valid_control(Path::new(r"C:\Windows\System32\drivers\etc\hosts")));
        assert!(!valid_control(Path::new(r"C:\Users\a\Temp\..\..\Windows\backuper-admin-1")));
        assert!(!valid_control(Path::new(r"Temp\backuper-admin-1")));
        assert!(!valid_control(Path::new(r"C:\Temp\backuper-admin-")));
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
