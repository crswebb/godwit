// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autoconfig;
mod commands;
mod connectors;
mod convert;
mod dav;
mod domain;
mod engine;
mod microsoft;
mod oauth;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::probe,
            commands::run_migration,
            commands::ms_sign_in,
            commands::google_sign_in
        ])
        .run(tauri::generate_context!())
        .expect("error while running Godwit");
}
