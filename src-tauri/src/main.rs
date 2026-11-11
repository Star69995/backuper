// Release builds are a GUI app (no console window).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    backuper_lib::elevated_delete_if_asked();
    backuper_lib::run()
}
