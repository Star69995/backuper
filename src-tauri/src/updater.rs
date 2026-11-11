//! Self-update from the GitHub releases: `latest.json` (signed, made by `npm run release`)
//! is checked through tauri-plugin-updater, and the NSIS installer runs in passive mode.
//! Installing never interrupts a backup: it waits until nothing is running or queued.

use crate::core::Core;
use crate::model::UpdateStatus;
use chrono::Local;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// Lets the app finish starting (and a catch-up backup get queued) before the first check.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// A failed check (offline, GitHub down) is retried sooner.
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);
const TICK: Duration = Duration::from_secs(60);
/// No keyboard/mouse input for this long = the user is away, so "auto" may restart the app.
const AWAY_AFTER: Duration = Duration::from_secs(10 * 60);

/// The marker's content when the window should stay in the tray after the update.
const MARKER_HIDDEN: &str = "hidden";

pub struct Updates {
    status: Mutex<UpdateStatus>,
    /// The newer release that was found, and its installer once downloaded.
    pending: Mutex<Option<(Update, Option<Vec<u8>>)>>,
    /// A check or download is in progress.
    busy: AtomicBool,
    /// The last version a "new version" notification was shown for.
    notified: Mutex<Option<String>>,
    /// That notification was shown and the window hasn't been opened since. A click on it
    /// once it moved to the notification center doesn't reach our handler: Windows starts the
    /// app again (a second instance, which only brings this one up), so the next opening of the
    /// window is taken as the click.
    notified_unseen: AtomicBool,
    /// What the window has open with unsaved changes (`set_unsaved`, e.g. a task being edited).
    /// An install the user didn't just confirm waits while this isn't empty and asks first.
    unsaved: Mutex<Vec<String>>,
    core: Arc<Core>,
    app: AppHandle,
}

impl Updates {
    pub fn new(app: AppHandle, core: Arc<Core>) -> Arc<Self> {
        let status = UpdateStatus {
            current_version: app.package_info().version.to_string(),
            state: "idle".into(),
            ..Default::default()
        };
        Arc::new(Self {
            status: Mutex::new(status),
            pending: Mutex::new(None),
            busy: AtomicBool::new(false),
            notified: Mutex::new(None),
            notified_unseen: AtomicBool::new(false),
            unsaved: Mutex::new(Vec::new()),
            core,
            app,
        })
    }

    pub fn status(&self) -> UpdateStatus {
        self.status.lock().unwrap().clone()
    }

    fn set(&self, f: impl FnOnce(&mut UpdateStatus)) {
        f(&mut self.status.lock().unwrap());
        self.core.changed();
    }

    fn mode(&self) -> String {
        self.core.store.lock().unwrap().settings.update_mode.clone()
    }

    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || me.run_loop());
    }

    fn run_loop(&self) {
        std::thread::sleep(FIRST_CHECK_DELAY);
        let mut next_check = Instant::now();
        loop {
            let mode = self.mode();
            if mode != "off" && Instant::now() >= next_check {
                let ok = tauri::async_runtime::block_on(self.check()).is_ok();
                next_check = Instant::now() + if ok { CHECK_EVERY } else { RETRY_AFTER };
                if ok && mode == "notify" {
                    self.notify_available();
                }
            }
            let has_pending = self.pending.lock().unwrap().is_some();
            if has_pending && (mode == "auto" || self.status().install_waiting) {
                let _ = tauri::async_runtime::block_on(self.download());
            }
            self.install_if_due();
            std::thread::sleep(TICK);
        }
    }

    fn window_hidden(&self) -> bool {
        self.app.get_webview_window("main").is_some_and(|w| !w.is_visible().unwrap_or(true))
    }

    /// Installs the downloaded version on its own when that bothers no one: after "update now"
    /// had to wait for the backups, or in "auto" mode while the app isn't in use - the window is
    /// in the tray or minimized, or nobody touched the computer for `AWAY_AFTER`. Never with
    /// unsaved changes in the window (`install_if_idle`).
    fn install_if_due(&self) {
        let window = self.app.get_webview_window("main");
        let hidden = window.as_ref().is_some_and(|w| !w.is_visible().unwrap_or(true));
        let minimized = window.as_ref().is_some_and(|w| w.is_minimized().unwrap_or(false));
        if self.status().install_waiting {
            self.install_if_idle(hidden, false);
        } else if self.mode() == "auto" {
            if hidden || minimized {
                // A minimized window comes back in the tray, so the restart doesn't pop it up.
                self.install_if_idle(true, false);
            } else if user_away() {
                self.install_if_idle(false, false);
            }
        }
    }

    /// The window's list of things with unsaved changes; an install it didn't ask for waits for it.
    pub fn set_unsaved(&self, what: Vec<String>) {
        *self.unsaved.lock().unwrap() = what;
    }

    /// Looks for a newer release. Ok(true) = one is available.
    pub async fn check(&self) -> Result<bool, String> {
        if self.busy.swap(true, Ordering::SeqCst) {
            return Ok(self.pending.lock().unwrap().is_some());
        }
        self.set(|s| {
            s.state = "checking".into();
            s.error = None;
        });
        let result = async { self.app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string()) }.await;
        self.busy.store(false, Ordering::SeqCst);
        match result {
            Ok(Some(update)) => {
                let mut pending = self.pending.lock().unwrap();
                let same = pending.as_ref().is_some_and(|(u, _)| u.version == update.version);
                if !same {
                    *pending = Some((update.clone(), None));
                }
                let downloaded = pending.as_ref().is_some_and(|(_, b)| b.is_some());
                drop(pending);
                self.set(|s| {
                    s.state = if downloaded { "ready" } else { "available" }.into();
                    s.version = Some(update.version.clone());
                    s.notes = update.body.clone().filter(|b| !b.trim().is_empty());
                    s.checked_at = Some(Local::now());
                });
                Ok(true)
            }
            Ok(None) => {
                *self.pending.lock().unwrap() = None;
                self.set(|s| {
                    s.state = "upToDate".into();
                    s.version = None;
                    s.notes = None;
                    s.prompt = false;
                    s.checked_at = Some(Local::now());
                });
                Ok(false)
            }
            Err(e) => {
                let message = format!("לא ניתן לבדוק אם יש עדכון: {e}. בדקו את החיבור לאינטרנט ונסו שוב.");
                self.set(|s| {
                    s.state = "error".into();
                    s.error = Some(message.clone());
                });
                Err(message)
            }
        }
    }

    /// Downloads (and verifies) the installer of the pending release, if not done yet.
    async fn download(&self) -> Result<(), String> {
        let Some((update, None)) = self.pending.lock().unwrap().clone() else {
            return Ok(());
        };
        if self.busy.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.set(|s| {
            s.state = "downloading".into();
            s.downloaded = 0;
            s.total = None;
            s.error = None;
        });
        let mut last_emit = Instant::now();
        let result = update
            .download(
                |chunk, total| {
                    let mut s = self.status.lock().unwrap();
                    s.downloaded += chunk as u64;
                    s.total = total;
                    drop(s);
                    if last_emit.elapsed() >= Duration::from_millis(300) {
                        last_emit = Instant::now();
                        self.core.changed();
                    }
                },
                || {},
            )
            .await;
        self.busy.store(false, Ordering::SeqCst);
        match result {
            Ok(bytes) => {
                if let Some(p) = self.pending.lock().unwrap().as_mut().filter(|(u, _)| u.version == update.version) {
                    p.1 = Some(bytes);
                }
                self.set(|s| s.state = "ready".into());
                Ok(())
            }
            Err(e) => {
                let message = format!("הורדת העדכון נכשלה: {e}. בדקו את החיבור לאינטרנט ונסו שוב.");
                self.set(|s| {
                    s.state = "error".into();
                    s.error = Some(message.clone());
                    s.install_waiting = false;
                });
                Err(message)
            }
        }
    }

    /// The user's "update now": downloads, then installs right away or once the backups are done.
    /// Installs right away (unsaved changes or not: the user asked) unless a backup is running.
    pub async fn install_now(&self) -> Result<(), String> {
        if self.pending.lock().unwrap().is_none() && !self.check().await? {
            return Err("אין עדכון זמין - זו כבר הגרסה האחרונה.".into());
        }
        self.download().await?;
        self.set(|s| s.install_waiting = true);
        // Exits the app when it installs; otherwise the update loop retries every minute.
        self.install_if_idle(self.window_hidden(), true);
        Ok(())
    }

    /// Runs the downloaded installer (which closes the app and starts it again) if no backup is
    /// running or queued. `stay_hidden` = the restarted app stays in the tray. Without `force` (an
    /// install the user didn't just ask for) it also waits while the window has unsaved changes.
    fn install_if_idle(&self, stay_hidden: bool, force: bool) {
        let Some((update, Some(bytes))) = self.pending.lock().unwrap().clone() else {
            return;
        };
        if !force && !self.unsaved.lock().unwrap().is_empty() {
            return;
        }
        let marker = self.core.store.lock().unwrap().after_update_marker();
        let result = self.core.when_idle(|| {
            let _ = std::fs::write(&marker, if stay_hidden { MARKER_HIDDEN } else { "" });
            // The process exits without cleanup, which would leave a dead icon in the tray.
            crate::tray::set_visible(&self.app, false);
            // Returns only on failure.
            update.install(&bytes)
        });
        if let Some(Err(e)) = result {
            let _ = std::fs::remove_file(&marker);
            crate::tray::set_visible(&self.app, true);
            self.set(|s| {
                s.state = "error".into();
                s.error = Some(format!("התקנת העדכון נכשלה: {e}. נסו שוב, או הורידו את הגרסה החדשה מדף ההורדות."));
                s.install_waiting = false;
            });
        }
    }

    /// Asks the window to show the update dialog (it reads `UpdateStatus.prompt` from the
    /// snapshot, so this works however long the window takes to come up).
    pub fn request_prompt(&self) {
        self.notified_unseen.store(false, Ordering::SeqCst);
        if self.pending.lock().unwrap().is_some() {
            self.set(|s| s.prompt = true);
        }
    }

    /// The main window was brought up (tray, second instance, notification).
    pub fn window_shown(&self) {
        if self.notified_unseen.load(Ordering::SeqCst) {
            self.request_prompt();
        }
    }

    pub fn dismiss_prompt(&self) {
        self.set(|s| s.prompt = false);
    }

    fn notify_available(&self) {
        let Some(version) = self.status().version else { return };
        let mut notified = self.notified.lock().unwrap();
        if notified.as_deref() == Some(version.as_str()) {
            return;
        }
        *notified = Some(version.clone());
        self.notified_unseen.store(true, Ordering::SeqCst);
        let sound = self.core.store.lock().unwrap().settings.sound_success.clone();
        let title = format!("גרסה חדשה של Backuper זמינה: {version}");
        let body = "לחצו כאן כדי לפתוח את Backuper ולעדכן.";
        #[cfg(windows)]
        {
            use tauri_winrt_notification::{Sound, Toast};
            let app = self.app.clone();
            let id = self.app.config().identifier.clone();
            let mut toast = Toast::new(&id).title(&title).text1(body).on_activated(move |_| {
                open_update_page(&app);
                Ok(())
            });
            if let Ok(s) = sound.parse::<Sound>() {
                toast = toast.sound(Some(s));
            }
            if toast.show().is_ok() {
                return;
            }
        }
        crate::core::toast(&self.app, title, body, &sound);
    }
}

/// A click on the "new version" notification: brings the window up with the update dialog.
#[cfg(windows)]
fn open_update_page(app: &AppHandle) {
    if let Some(updates) = app.try_state::<Arc<Updates>>() {
        updates.request_prompt();
    }
    crate::tray::show_main(app);
}

/// Called at startup: if this launch follows an update, removes the marker, says so, and
/// returns whether the window should stay in the tray.
pub fn after_update(app: &AppHandle, core: &Core) -> Option<bool> {
    let marker = core.store.lock().unwrap().after_update_marker();
    let content = std::fs::read_to_string(&marker).ok()?;
    let _ = std::fs::remove_file(&marker);
    crate::core::toast(
        app,
        format!("Backuper עודכן לגרסה {}", app.package_info().version),
        "הגיבויים והמשימות ממשיכים כרגיל.",
        "",
    );
    Some(content.trim() == MARKER_HIDDEN)
}

/// Nobody has used the keyboard or mouse for `AWAY_AFTER` (any app, not just ours).
#[cfg(windows)]
fn user_away() -> bool {
    #[repr(C)]
    struct LastInputInfo {
        size: u32,
        time: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetLastInputInfo(info: *mut LastInputInfo) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount() -> u32;
    }
    let mut info = LastInputInfo { size: std::mem::size_of::<LastInputInfo>() as u32, time: 0 };
    // SAFETY: plain Win32 calls with a correctly sized struct.
    unsafe {
        if GetLastInputInfo(&mut info) == 0 {
            return false;
        }
        u64::from(GetTickCount().wrapping_sub(info.time)) >= AWAY_AFTER.as_millis() as u64
    }
}

#[cfg(not(windows))]
fn user_away() -> bool {
    false
}
