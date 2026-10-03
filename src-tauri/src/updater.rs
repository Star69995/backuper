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
            let window_hidden = self
                .app
                .get_webview_window("main")
                .is_some_and(|w| !w.is_visible().unwrap_or(true));
            // Unasked, only from the tray, so it never closes a window the user is looking at.
            if self.status().install_waiting || (mode == "auto" && window_hidden) {
                self.install_if_idle(window_hidden);
            }
            std::thread::sleep(TICK);
        }
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
    pub async fn install_now(&self) -> Result<(), String> {
        if self.pending.lock().unwrap().is_none() && !self.check().await? {
            return Err("אין עדכון זמין - זו כבר הגרסה האחרונה.".into());
        }
        self.download().await?;
        self.set(|s| s.install_waiting = true);
        // Exits the app when it installs; otherwise the update loop retries every minute.
        let hidden = self.app.get_webview_window("main").is_some_and(|w| !w.is_visible().unwrap_or(true));
        self.install_if_idle(hidden);
        Ok(())
    }

    /// Runs the downloaded installer (which closes the app and starts it again) if no backup is
    /// running or queued. `stay_hidden` = the restarted app stays in the tray.
    fn install_if_idle(&self, stay_hidden: bool) {
        let Some((update, Some(bytes))) = self.pending.lock().unwrap().clone() else {
            return;
        };
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

    fn notify_available(&self) {
        let Some(version) = self.status().version else { return };
        let mut notified = self.notified.lock().unwrap();
        if notified.as_deref() == Some(version.as_str()) {
            return;
        }
        *notified = Some(version.clone());
        let sound = self.core.store.lock().unwrap().settings.sound_success.clone();
        crate::core::toast(
            &self.app,
            format!("גרסה חדשה של Backuper זמינה: {version}"),
            "פתחו את Backuper ובחרו \"עדכן עכשיו\" בהגדרות.",
            &sound,
        );
    }
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
