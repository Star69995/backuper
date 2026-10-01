mod backup;
mod commands;
mod core;
mod engine;
mod filters;
mod model;
mod schedule;
mod store;
mod tray;

use crate::core::Core;
use std::sync::Arc;
use tauri::{Manager, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

/// Passed by the autostart entry so the app starts straight to the tray.
const HIDDEN_ARG: &str = "--hidden";

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main(app)))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let store = store::Store::open(app.path().app_data_dir()?);
            let core = Core::new(handle.clone(), store);
            app.manage(core.clone());

            // First launch of an installed build: register to start with Windows.
            // (Skipped in dev so the debug exe is never registered.)
            {
                let mut s = core.store.lock().unwrap();
                if !s.settings.first_run_done {
                    if !cfg!(debug_assertions) {
                        let _ = handle.autolaunch().enable();
                    }
                    s.settings.first_run_done = true;
                    s.save_settings();
                }
            }

            tray::create(&handle)?;
            core.start();
            if !std::env::args().any(|a| a == HIDDEN_ARG) {
                tray::show_main(&handle);
            }
            Ok(())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
