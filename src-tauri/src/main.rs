// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};

/// Connection details for a source or destination IMAP account.
///
/// These are received from the UI, used for a single connection, and never
/// persisted or sent anywhere else. (Secret storage via the OS keychain comes
/// later — see docs/ARCHITECTURE.md.)
#[derive(Debug, Deserialize)]
pub struct ImapCreds {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

/// A single IMAP folder (mailbox) as reported by LIST.
#[derive(Debug, Serialize)]
pub struct FolderInfo {
    pub name: String,
    pub delimiter: Option<String>,
    pub attributes: Vec<String>,
}

/// Connect to an IMAP server over TLS and return its folder list.
///
/// The blocking `imap` work runs on a blocking thread so it never stalls the
/// async runtime / UI. Errors are returned as human-readable strings for now.
#[tauri::command]
async fn list_folders(creds: ImapCreds) -> Result<Vec<FolderInfo>, String> {
    tokio::task::spawn_blocking(move || list_folders_blocking(&creds))
        .await
        .map_err(|e| format!("task failed: {e}"))?
}

fn list_folders_blocking(creds: &ImapCreds) -> Result<Vec<FolderInfo>, String> {
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| format!("TLS setup failed: {e}"))?;

    // Implicit-TLS IMAP (port 993). STARTTLS on 143 is a later addition.
    let client = imap::connect((creds.host.as_str(), creds.port), creds.host.as_str(), &tls)
        .map_err(|e| format!("Connection failed: {e}"))?;

    let mut session = client
        .login(&creds.username, &creds.password)
        .map_err(|(e, _client)| format!("Login failed: {e}"))?;

    let result = (|| {
        let names = session
            .list(Some(""), Some("*"))
            .map_err(|e| format!("LIST failed: {e}"))?;

        let folders = names
            .iter()
            .map(|n| FolderInfo {
                name: n.name().to_string(),
                delimiter: n.delimiter().map(str::to_string),
                attributes: n.attributes().iter().map(|a| format!("{a:?}")).collect(),
            })
            .collect();

        Ok::<_, String>(folders)
    })();

    // Always try to close the session cleanly, regardless of the LIST outcome.
    let _ = session.logout();

    result
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![list_folders])
        .run(tauri::generate_context!())
        .expect("error while running Godwit");
}
