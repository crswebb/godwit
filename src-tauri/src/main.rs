// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dav;

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
    /// When true, work out what would move (envelope-only) but write nothing.
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
    /// Source Message-IDs still absent from the destination after copying
    /// (independent verification recount). 0 means fully verified.
    pub missing: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct MigrationReport {
    pub folders: Vec<FolderReport>,
    pub copied: u32,
    pub skipped: u32,
    pub failed: u32,
    pub missing: u32,
}

impl MigrationReport {
    fn add(&mut self, fr: FolderReport) {
        self.copied += fr.copied;
        self.skipped += fr.skipped;
        self.failed += fr.failed;
        self.missing += fr.missing;
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
/// Full-message (body) fetch batch — kept small to bound memory on large messages.
const FETCH_BATCH: usize = 20;
/// Envelope-only fetch batch — envelopes are tiny, so this can be large.
const ENVELOPE_BATCH: usize = 500;

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
/// flags and internal date, skipping messages already present (by Message-ID),
/// then verifying the destination independently.
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

    // Destination state before copying: which Message-IDs are already there.
    let dmb = dst.select(name).map_err(|e| format!("select destination: {e}"))?;
    let dest_before = collect_ids_by_seq(dst, dmb.exists)?;

    // Source state. SELECT (not STORE) + BODY.PEEK means the source is only
    // ever read — messages are never marked \Seen or otherwise modified.
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

    // Pass 1 (cheap): envelope-only, to learn each message's UID + Message-ID.
    let src_entries = collect_source_entries(src, &uids)?;

    // Messages missing from the destination are the ones to copy. Messages
    // with no Message-ID can't be deduped, so we always copy them.
    let to_copy: Vec<u32> = src_entries
        .iter()
        .filter(|(_, id)| match id {
            Some(id) => !dest_before.contains(id),
            None => true,
        })
        .map(|(uid, _)| *uid)
        .collect();

    let mut fr = FolderReport {
        folder: name.to_string(),
        source_total: total,
        skipped: src_entries.len().saturating_sub(to_copy.len()) as u32,
        ..Default::default()
    };

    if opts.dry_run {
        fr.copied = to_copy.len() as u32;
        return Ok(fr);
    }

    // Pass 2: fetch bodies only for the messages that need copying.
    let target = to_copy.len() as u32;
    let mut done = 0u32;
    for chunk in to_copy.chunks(FETCH_BATCH) {
        let set = chunk.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let messages = src
            .uid_fetch(&set, "(FLAGS INTERNALDATE BODY.PEEK[])")
            .map_err(|e| format!("UID FETCH: {e}"))?;

        for m in messages.iter() {
            done += 1;
            match m.body() {
                Some(body) => {
                    let flags = map_flags(m.flags());
                    match dst.append_with_flags_and_date(name, body, &flags, m.internal_date()) {
                        Ok(()) => fr.copied += 1,
                        Err(e) => {
                            fr.failed += 1;
                            emit(app, Progress::Warning {
                                message: format!("{name}: append failed: {e}"),
                            });
                        }
                    }
                }
                None => fr.failed += 1,
            }

            if done % 25 == 0 || done == target {
                emit(app, Progress::Tick { folder: name.to_string(), done, total: target });
            }
        }
    }

    // Verification: independently re-read the destination and confirm every
    // source Message-ID is now present.
    let dmb2 = dst.select(name).map_err(|e| format!("re-select destination: {e}"))?;
    let dest_after = collect_ids_by_seq(dst, dmb2.exists)?;
    fr.missing = src_entries
        .iter()
        .filter_map(|(_, id)| id.as_ref())
        .filter(|id| !dest_after.contains(*id))
        .count() as u32;

    Ok(fr)
}

// --- Helpers ---------------------------------------------------------------

/// Collect the Message-IDs in the currently-selectable mailbox by walking it in
/// sequence-number batches. `exists` is the message count from SELECT.
fn collect_ids_by_seq(session: &mut ImapSession, exists: u32) -> Result<HashSet<String>, String> {
    let mut ids = HashSet::new();
    if exists == 0 {
        return Ok(ids);
    }
    let mut start = 1u32;
    while start <= exists {
        let end = (start + ENVELOPE_BATCH as u32 - 1).min(exists);
        let seq = format!("{start}:{end}");
        let items = session
            .fetch(&seq, "(ENVELOPE)")
            .map_err(|e| format!("ENVELOPE fetch: {e}"))?;
        for f in items.iter() {
            if let Some(id) = message_id(f) {
                ids.insert(id);
            }
        }
        start = end + 1;
    }
    Ok(ids)
}

/// Fetch (UID, Message-ID) for every source message, envelope-only, in batches.
fn collect_source_entries(
    session: &mut ImapSession,
    uids: &[u32],
) -> Result<Vec<(u32, Option<String>)>, String> {
    let mut out = Vec::with_capacity(uids.len());
    for chunk in uids.chunks(ENVELOPE_BATCH) {
        let set = chunk.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let items = session
            .uid_fetch(&set, "(UID ENVELOPE)")
            .map_err(|e| format!("UID ENVELOPE fetch: {e}"))?;
        for f in items.iter() {
            if let Some(uid) = f.uid {
                out.push((uid, message_id(f)));
            }
        }
    }
    Ok(out)
}

/// Extract a message's Message-ID from its ENVELOPE, for dedup + verification.
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

/// Discover the calendars or address books for a CalDAV/CardDAV account.
#[tauri::command]
async fn dav_discover(
    account: dav::DavAccount,
    kind: dav::DavKind,
) -> Result<Vec<dav::DavCollection>, String> {
    tokio::task::spawn_blocking(move || dav::discover(&account, kind))
        .await
        .map_err(|e| format!("task failed: {e}"))?
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![list_folders, migrate, dav_discover])
        .run(tauri::generate_context!())
        .expect("error while running Godwit");
}
