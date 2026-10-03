//! System tray icon and its menu.

use crate::core::Core;
use std::sync::Arc;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

const TRAY_ID: &str = "main";

pub struct TrayItems {
    pause: CheckMenuItem<Wry>,
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
    let open = MenuItem::with_id(app, "open", "פתח את Backuper", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "השהה תזמון", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "יציאה", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &pause, &PredefinedMenuItem::separator(app)?, &quit])?;
    app.manage(TrayItems { pause });

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
    let paused = core.store.lock().unwrap().settings.scheduler_paused;
    let running = core.current.lock().unwrap().as_ref().map(|p| p.task_name.clone());
    let tip = match (running, paused) {
        (Some(name), _) => format!("Backuper - מגבה: {name}"),
        (None, true) => "Backuper - התזמון מושהה".to_string(),
        (None, false) => "Backuper - פעיל".to_string(),
    };
    let _ = tray.set_tooltip(Some(tip));
    let _ = items.pause.set_checked(paused);
}
