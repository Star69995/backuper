mod backup;
mod cobian;
mod commands;
mod core;
mod drives;
mod engine;
mod filters;
mod manifest;
mod model;
mod schedule;
mod store;
mod tasklist;
mod tray;
mod updater;

use crate::core::Core;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::webview::PageLoadEvent;
use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

/// Passed by the autostart entry so the app starts straight to the tray.
const HIDDEN_ARG: &str = "--hidden";

/// Whether the main window still has to be shown once its page has loaded (normal, non-tray start).
static SHOW_ON_LOAD: AtomicBool = AtomicBool::new(false);

/// Lays the native title bar out right-to-left (icon and title on the right, buttons on the
/// left), as on Hebrew Windows. NOINHERITLAYOUT keeps the webview child windows unmirrored.
fn mirror_title_bar(window: &tauri::WebviewWindow) {
    #[link(name = "user32")]
    extern "system" {
        fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
        fn SetWindowLongPtrW(hwnd: isize, index: i32, value: isize) -> isize;
        fn SetWindowPos(hwnd: isize, after: isize, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
    }
    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_LAYOUTRTL: isize = 0x0040_0000;
    const WS_EX_NOINHERITLAYOUT: isize = 0x0010_0000;
    // SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED
    const FLAGS: u32 = 0x1 | 0x2 | 0x4 | 0x10 | 0x20;
    let Ok(hwnd) = window.hwnd() else { return };
    let hwnd = hwnd.0 as isize;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYOUTRTL | WS_EX_NOINHERITLAYOUT);
        SetWindowPos(hwnd, 0, 0, 0, 0, 0, FLAGS);
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main(app)))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();
            let store = store::Store::open(app.path().app_data_dir()?);
            let core = Core::new(handle.clone(), store);
            app.manage(core.clone());

            // Keep the Windows startup entry in line with the user's choice.
            let start_with_windows = core.store.lock().unwrap().settings.start_with_windows;
            let _ = commands::apply_autostart(&handle, start_with_windows);

            if let Some(w) = app.get_webview_window("main") {
                mirror_title_bar(&w);
            }
            tray::create(&handle)?;
            // Started hidden, the window may stay closed for a while; the notice waits there too.
            let notice = {
                let s = core.store.lock().unwrap();
                s.notice.clone().map(|n| (n, s.settings.sound_failure.clone()))
            };
            if let Some((n, sound)) = notice {
                crate::core::toast(&handle, n.title, n.message, &sound);
            }
            core.start();
            let updates = updater::Updates::new(handle.clone(), core.clone());
            app.manage(updates.clone());
            updates.start();
            // An update installed from the tray restarts into the tray, whatever the original args.
            let hidden = match updater::after_update(&handle, &core) {
                Some(hidden) => hidden,
                None => std::env::args().any(|a| a == HIDDEN_ARG),
            };
            if !hidden {
                SHOW_ON_LOAD.store(true, Ordering::SeqCst);
                tray::show_main(&handle);
            }
            Ok(())
        })
        // Showing during setup can lose to the window that launched us (e.g. the installer's
        // finish page), so show and bring it to the front again once the page has loaded.
        .on_page_load(|webview, payload| {
            if payload.event() == PageLoadEvent::Finished
                && webview.label() == "main"
                && SHOW_ON_LOAD.swap(false, Ordering::SeqCst)
            {
                tray::show_main(webview.app_handle());
            }
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let core = window.state::<Arc<Core>>();
                if core.store.lock().unwrap().settings.close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::save_tasks,
            commands::delete_tasks,
            commands::reorder_tasks,
            commands::run_tasks,
            commands::cancel_task,
            commands::get_history,
            commands::clear_history,
            commands::read_log,
            commands::list_backups,
            commands::delete_backup,
            commands::folder_size,
            commands::open_path,
            commands::preview_schedule,
            commands::save_settings,
            commands::set_autostart,
            commands::import_tasks,
            commands::export_tasks,
            commands::list_task_snapshots,
            commands::dismiss_notice,
            commands::answer_drive_prompts,
            commands::test_sound,
            commands::check_updates,
            commands::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
