//! System tray icon and its menu.

use crate::core::Core;
use crate::model::RunStatus;
use std::sync::Arc;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::image::Image;
use tauri::{AppHandle, Manager, Wry};

const TRAY_ID: &str = "main";

pub struct TrayItems {
    pause: CheckMenuItem<Wry>,
    /// The badge the tray icon shows now (a `Badge` as u8), so the icon is only replaced when it changes.
    badged: std::sync::atomic::AtomicU8,
}

/// The dot on the tray icon, worst first: it shows the most important thing that needs a look.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Badge {
    None = 0,
    /// Orange "!": a task's last run finished with warnings.
    Warning = 1,
    /// Red "!": a task's last run failed.
    Error = 2,
}

impl Badge {
    fn from_u8(n: u8) -> Badge {
        match n {
            1 => Badge::Warning,
            2 => Badge::Error,
            _ => Badge::None,
        }
    }
}

/// The app icon with a colored dot with a white "!" in its bottom-right corner.
fn badge_icon(base: &Image<'_>, badge: Badge) -> Image<'static> {
    let color = match badge {
        Badge::Error => [220, 38, 38],
        Badge::Warning => [234, 138, 0],
        _ => [234, 138, 0],
    };
    let (w, h) = (base.width() as i32, base.height() as i32);
    let mut px = base.rgba().to_vec();
    let size = w.min(h) as f32;
    let r = size * 0.26;
    let (cx, cy) = (w as f32 - r - size * 0.02, h as f32 - r - size * 0.02);
    let mut put = |x: i32, y: i32, c: [u8; 3], a: f32| {
        if x < 0 || y < 0 || x >= w || y >= h || a <= 0.0 {
            return;
        }
        let i = ((y * w + x) * 4) as usize;
        let old_a = px[i + 3] as f32 / 255.0;
        let out_a = a + old_a * (1.0 - a);
        for k in 0..3 {
            let v = (c[k] as f32 * a + px[i + k] as f32 * old_a * (1.0 - a)) / out_a.max(0.001);
            px[i + k] = v.round() as u8;
        }
        px[i + 3] = (out_a * 255.0).round() as u8;
    };
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            // White ring, then red fill (1px soft edge).
            put(x, y, [255, 255, 255], (r + 1.0 - d).clamp(0.0, 1.0));
            put(x, y, color, (r - size * 0.05 + 1.0 - d).clamp(0.0, 1.0));
        }
    }
    // The "!": a bar and a dot.
    let half = (size * 0.045).max(0.8);
    let bar = (cy - r * 0.60, cy + r * 0.0);
    let dot = (cy + r * 0.45, half * 1.0);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let in_bar = (fx - cx).abs() <= half && fy >= bar.0 && fy <= bar.1;
            let in_dot = ((fx - cx).powi(2) + (fy - dot.0).powi(2)).sqrt() <= dot.1;
            if in_bar || in_dot {
                put(x, y, [255, 255, 255], 1.0);
            }
        }
    }
    Image::new_owned(px, base.width(), base.height())
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        // Windows won't let a background process take focus; briefly going always-on-top
        // brings the window in front of whatever is active (installer, Explorer).
        let _ = w.set_always_on_top(true);
        let _ = w.set_focus();
        let _ = w.set_always_on_top(false);
    }
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "פתיחת Backuper", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "השהיית התזמון", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "יציאה", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &pause, &PredefinedMenuItem::separator(app)?, &quit])?;
    app.manage(TrayItems {
        pause,
        badged: std::sync::atomic::AtomicU8::new(0),
    });

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("Backuper")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "pause" => {
                let core = app.state::<Arc<Core>>();
                {
                    let mut s = core.store.lock().unwrap();
                    s.settings.scheduler_paused = !s.settings.scheduler_paused;
                    s.save_settings();
                }
                core.changed();
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    refresh(app);
    Ok(())
}

pub fn set_visible(app: &AppHandle, visible: bool) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_visible(visible);
    }
}

/// Syncs the tooltip and the pause checkbox with the current state.
pub fn refresh(app: &AppHandle) {
    let (Some(core), Some(items), Some(tray)) = (
        app.try_state::<Arc<Core>>(),
        app.try_state::<TrayItems>(),
        app.tray_by_id(TRAY_ID),
    ) else {
        return;
    };
    let (paused, failed, warned) = {
        let s = core.store.lock().unwrap();
        // The latest run of each enabled task; the next run replaces it.
        let count = |status| {
            s.tasks
                .iter()
                .filter(|t| t.enabled)
                .filter_map(|t| s.states.get(&t.id))
                .filter(|st| st.last_status == Some(status))
                .count()
        };
        (s.settings.scheduler_paused, count(RunStatus::Failed), count(RunStatus::Warning))
    };
    let running = core.current.lock().unwrap().as_ref().map(|p| p.task_name.clone());
    let mut tip = match (running, paused) {
        (Some(name), _) => format!("Backuper - מגבה: {name}"),
        (None, true) => "Backuper - התזמון מושהה".to_string(),
        (None, false) => "Backuper - פעיל".to_string(),
    };
    match failed {
        0 => {}
        1 => tip.push_str("\nשגיאה בגיבוי אחד"),
        n => tip.push_str(&format!("\nשגיאה ב-{n} גיבויים")),
    }
    match warned {
        0 => {}
        1 => tip.push_str("\nאזהרה בגיבוי אחד"),
        n => tip.push_str(&format!("\nאזהרה ב-{n} גיבויים")),
    }
    let badge = if failed > 0 {
        Badge::Error
    } else if warned > 0 {
        Badge::Warning
    } else {
        Badge::None
    };
    let was = Badge::from_u8(items.badged.swap(badge as u8, std::sync::atomic::Ordering::Relaxed));
    if was != badge {
        if let Some(base) = app.default_window_icon() {
            let _ = tray.set_icon(Some(if badge == Badge::None { base.clone() } else { badge_icon(base, badge) }));
        }
    }
    let _ = tray.set_tooltip(Some(tip));
    let _ = items.pause.set_checked(paused);
}
