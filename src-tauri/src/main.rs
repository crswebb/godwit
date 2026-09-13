// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashSet;
use std::net::TcpStream;

use imap::types::{Fetch, Flag};
use native_tls::TlsStream;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

type ImapSession = imap::Session<TlsStream<TcpStream>>;

/// Connection details for a source or destination IMAP account.
///
/// Received from the UI, used for a single connection, never persisted or sent
/// anywhere else. (Secret storage via the OS keychain comes later.)
#[derive(Debug, Clone, Deserialize)]
pub struct ImapCreds {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateOptions {
    /// When true, walk everything and report counts but write nothing.
    pub dry_run: bool,
}

/// A single IMAP folder as reported by LIST.
#[derive(Debug, Serialize)]
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
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MigrationReport {
    pub folders: Vec<FolderReport>,
    pub copied: u32,
    pub skipped: u32,
    pub failed: u32,
}

impl MigrationReport {
    fn add(&mut self, fr: FolderReport) {
        self.copied += fr.copied;
        self.skipped += fr.skipped;
        self.failed += fr.failed;
        self.folders.push(fr);
    }
}

/// Progress events streamed to the UI during a migration.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Progress {
    Started { folders: usize },
    FolderStart { name: String, index: usize, folders: usize, source_total: u32 },
    Tick { folder: String, done: u32, total: u32 },
    FolderDone { report: FolderReport },
    Warning { message: String },
    Done { report: MigrationReport },
}

const PROGRESS_EVENT: &str = "migration://progress";
const FETCH_BATCH: usize = 20;

fn emit(app: &tauri::AppHandle, p: Progress) {
    let _ = app.emit(PROGRESS_EVENT, p);
}

// --- Connection ------------------------------------------------------------

fn open_session(creds: &ImapCreds) -> Result<ImapSession, String> {
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| format!("TLS setup failed: {e}"))?;

    let client = imap::connect((creds.host.as_str(), creds.port), creds.host.as_str(), &tls)
        .map_err(|e| format!("Connection failed: {e}"))?;

    client
        .login(&creds.username, &creds.password)
        .map_err(|(e, _client)| format!("Login failed: {e}"))
}

// --- Commands --------------------------------------------------------------

/// Connect to an IMAP server over TLS and return its folder list.
#[tauri::command]
async fn list_folders(creds: ImapCreds) -> Result<Vec<FolderInfo>, String> {
    tokio::task::spawn_blocking(move || {
        let mut session = open_session(&creds)?;
        let result = session
            .list(Some(""), Some("*"))
            .map_err(|e| format!("LIST failed: {e}"))
            .map(|names| {
                names
                    .iter()
                    .map(|n| FolderInfo {
                        name: n.name().to_string(),
                        delimiter: n.delimiter().map(str::to_string),
                        attributes: n.attributes().iter().map(|a| format!("{a:?}")).collect(),
                    })
                    .collect()
            });
        let _ = session.logout();
        result
    })
    .await
    .map_err(|e| format!("task failed: {e}"))?
}

/// Copy every folder + message from `source` to `destination`, preserving
/// flags and internal date, skipping messages already present (by Message-ID).
///
/// Read-only against the source (BODY.PEEK, never STORE). Never deletes anything.
#[tauri::command]
async fn migrate(
    app: tauri::AppHandle,
    source: ImapCreds,
    destination: ImapCreds,
    options: MigrateOptions,
) -> Result<MigrationReport, String> {
    tokio::task::spawn_blocking(move || migrate_blocking(&app, source, destination, options))
        .await
        .map_err(|e| format!("task failed: {e}"))?
}

fn migrate_blocking(
    app: &tauri::AppHandle,
    source: ImapCreds,
    destination: ImapCreds,
    opts: MigrateOptions,
) -> Result<MigrationReport, String> {
    let mut src = open_session(&source)?;
    let mut dst = open_session(&destination)?;

    // Collect owned folder names first so we're not borrowing the session
    // for the duration of the copy loop.
    let names: Vec<String> = {
        let list = src
            .list(Some(""), Some("*"))
            .map_err(|e| format!("LIST failed: {e}"))?;
        list.iter().map(|n| n.name().to_string()).collect()
    };

    emit(app, Progress::Started { folders: names.len() });

    let mut report = MigrationReport::default();
    for (i, name) in names.iter().enumerate() {
        match copy_folder(app, &mut src, &mut dst, name, i, names.len(), &opts) {
            Ok(fr) => {
                emit(app, Progress::FolderDone { report: fr.clone() });
                report.add(fr);
            }
            // A single bad folder (e.g. \Noselect container) shouldn't abort
            // the whole migration — record it and keep going.
            Err(e) => {
                emit(app, Progress::Warning { message: format!("{name}: {e}") });
                report.add(FolderReport { folder: name.clone(), ..Default::default() });
            }
        }
    }

    let _ = src.logout();
    let _ = dst.logout();

    emit(app, Progress::Done { report: report.clone() });
    Ok(report)
}

fn copy_folder(
    app: &tauri::AppHandle,
    src: &mut ImapSession,
    dst: &mut ImapSession,
    name: &str,
    index: usize,
    folders: usize,
    opts: &MigrateOptions,
) -> Result<FolderReport, String> {
    // Ensure the destination folder exists (ignore "already exists").
    let _ = dst.create(name);

    // Build the set of Message-IDs already on the destination, so re-runs
    // don't duplicate.
    let dmb = dst.select(name).map_err(|e| format!("select destination: {e}"))?;
    let mut seen: HashSet<String> = HashSet::new();
    if dmb.exists > 0 {
        let envs = dst
            .fetch("1:*", "ENVELOPE")
            .map_err(|e| format!("destination ENVELOPE fetch: {e}"))?;
        for f in envs.iter() {
            if let Some(id) = message_id(f) {
                seen.insert(id);
            }
        }
    }

    // Source is opened with SELECT but only ever read via BODY.PEEK, so its
    // messages are never marked \Seen or otherwise modified.
    let smb = src.select(name).map_err(|e| format!("select source: {e}"))?;
    let total = smb.exists;
    emit(
        app,
        Progress::FolderStart { name: name.to_string(), index, folders, source_total: total },
    );

    let mut uids: Vec<u32> = src
        .uid_search("ALL")
        .map_err(|e| format!("UID SEARCH: {e}"))?
        .into_iter()
        .collect();
    uids.sort_unstable();

    let mut fr = FolderReport { folder: name.to_string(), source_total: total, ..Default::default() };
    let mut done = 0u32;

    for chunk in uids.chunks(FETCH_BATCH) {
        let set = chunk.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let messages = src
            .uid_fetch(&set, "(FLAGS INTERNALDATE ENVELOPE BODY.PEEK[])")
            .map_err(|e| format!("UID FETCH: {e}"))?;

        for m in messages.iter() {
            done += 1;

            let body = match m.body() {
                Some(b) => b,
                None => {
                    fr.failed += 1;
                    continue;
                }
            };

            let id = message_id(m);
            if let Some(id) = &id {
                if seen.contains(id) {
                    fr.skipped += 1;
                    continue;
                }
            }

            if opts.dry_run {
                // Preview: this message is not a duplicate, so it *would* be
                // copied. (Duplicates were already counted as skipped above.)
                fr.copied += 1;
            } else {
                let flags = map_flags(m.flags());
                match dst.append_with_flags_and_date(name, body, &flags, m.internal_date()) {
                    Ok(()) => {
                        fr.copied += 1;
                        if let Some(id) = id {
                            seen.insert(id);
                        }
                    }
                    Err(e) => {
                        fr.failed += 1;
                        emit(app, Progress::Warning { message: format!("{name}: append failed: {e}") });
                    }
                }
            }

            if done % 25 == 0 || done == total {
                emit(app, Progress::Tick { folder: name.to_string(), done, total });
            }
        }
    }

    Ok(fr)
}

// --- Helpers ---------------------------------------------------------------

/// Extract a message's Message-ID from its ENVELOPE, for dedup.
fn message_id(f: &Fetch) -> Option<String> {
    f.envelope()
        .and_then(|e| e.message_id.as_deref())
        .map(|b| String::from_utf8_lossy(b).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Map source flags to the standard flags worth carrying to the destination.
/// (\Recent is server-managed; \Deleted and custom/keyword flags are dropped
/// for now.)
fn map_flags(src: &[Flag]) -> Vec<Flag<'static>> {
    let mut out = Vec::new();
    for f in src {
        match f {
            Flag::Seen => out.push(Flag::Seen),
            Flag::Answered => out.push(Flag::Answered),
            Flag::Flagged => out.push(Flag::Flagged),
            Flag::Draft => out.push(Flag::Draft),
            _ => {}
        }
    }
    out
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![list_folders, migrate])
        .run(tauri::generate_context!())
        .expect("error while running Godwit");
}
