//! Tauri command layer — thin wrappers that run the work on a blocking thread
//! and delegate to the connectors/engine. No logic lives here.

use crate::connectors;
use crate::domain::{Account, Probe, Selection, UnifiedReport};
use crate::microsoft;

/// Probe an account for everything Godwit can migrate.
#[tauri::command]
pub async fn probe(account: Account) -> Result<Probe, String> {
    tokio::task::spawn_blocking(move || connectors::probe(&account))
        .await
        .map_err(|e| format!("task failed: {e}"))
}

/// Run a migration for the selected folders / calendars / address books.
#[tauri::command]
pub async fn run_migration(
    app: tauri::AppHandle,
    source: Account,
    destination: Account,
    selection: Selection,
    dry_run: bool,
) -> Result<UnifiedReport, String> {
    tokio::task::spawn_blocking(move || connectors::run(&app, source, destination, selection, dry_run))
        .await
        .map_err(|e| format!("task failed: {e}"))?
}

/// Sign in to a Microsoft 365 account via OAuth (opens the system browser).
#[tauri::command]
pub async fn ms_sign_in(client_id: String) -> Result<microsoft::MsAccount, String> {
    tokio::task::spawn_blocking(move || microsoft::sign_in(&client_id))
        .await
        .map_err(|e| format!("task failed: {e}"))?
}

/// Sign in to a Google account via OAuth (opens the system browser).
#[tauri::command]
pub async fn google_sign_in(client_id: String) -> Result<connectors::google::GoogleAccount, String> {
    tokio::task::spawn_blocking(move || connectors::google::sign_in(&client_id))
        .await
        .map_err(|e| format!("task failed: {e}"))?
}
