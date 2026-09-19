//! Domain types shared across the app: accounts, probe results, migration
//! selection and reports, and progress events. Pure data (plus the small
//! `emit` helper) — no protocol logic lives here.

use crate::dav;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

fn default_provider() -> String {
    "password".to_string()
}

/// One account (source or destination). `provider` is "password" (IMAP/DAV) or
/// "microsoft" (Graph via OAuth; `email` identifies the signed-in session).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub email: String,
    #[serde(default)]
    pub password: String,
    pub imap_host: Option<String>,
    pub imap_port: Option<u16>,
    pub dav_url: Option<String>,
    #[serde(default = "default_provider")]
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderInfo {
    pub name: String,
    pub delimiter: Option<String>,
    pub attributes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderReport {
    pub folder: String,
    pub source_total: u32,
    pub copied: u32,
    pub skipped: u32,
    pub failed: u32,
    pub missing: u32,
}

// --- Probe ----------------------------------------------------------------

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapProbe {
    pub status: String, // "ok" | "needsHost" | "error"
    pub host: Option<String>,
    pub port: Option<u16>,
    pub folders: Vec<FolderInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DavProbe {
    pub status: String, // "ok" | "unavailable"
    pub collections: Vec<dav::DavCollection>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub imap: ImapProbe,
    pub calendars: DavProbe,
    pub contacts: DavProbe,
}

// --- Migration selection + report -----------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DavPair {
    pub name: String,
    pub source: String,
    pub dest: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selection {
    pub folders: Vec<String>,
    pub calendars: Vec<DavPair>,
    pub contacts: Vec<DavPair>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedDavReport {
    pub name: String,
    #[serde(flatten)]
    pub report: dav::DavReport,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedReport {
    pub folders: Vec<FolderReport>,
    pub calendars: Vec<NamedDavReport>,
    pub contacts: Vec<NamedDavReport>,
}

// --- Progress events -------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Progress {
    Started { folders: usize },
    FolderStart { name: String, index: usize, folders: usize, source_total: u32 },
    Tick { folder: String, done: u32, total: u32 },
    FolderDone { report: FolderReport },
    Warning { message: String },
}

pub const PROGRESS_EVENT: &str = "migration://progress";

pub fn emit(app: &tauri::AppHandle, p: Progress) {
    let _ = app.emit(PROGRESS_EVENT, p);
}
