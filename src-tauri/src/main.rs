// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autoconfig;
mod dav;

use std::collections::HashSet;
use std::net::TcpStream;

use imap::types::{Fetch, Flag};
use native_tls::TlsStream;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

type ImapSession = imap::Session<TlsStream<TcpStream>>;

/// One account (source or destination). Email + password is usually enough;
/// the server details are discovered, and only surface in the UI if discovery
/// fails.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub email: String,
    pub password: String,
    pub imap_host: Option<String>,
    pub imap_port: Option<u16>,
    pub dav_url: Option<String>,
}

#[derive(Debug, Clone)]
struct ImapCreds {
    host: String,
    port: u16,
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
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

// --- Probe (what can be migrated) -----------------------------------------

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImapProbe {
    /// "ok" | "needsHost" | "error"
    pub status: String,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub folders: Vec<FolderInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DavProbe {
    /// "ok" | "unavailable"
    pub status: String,
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

#[derive(Debug, Deserialize)]
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
enum Progress {
    Started { folders: usize },
    FolderStart { name: String, index: usize, folders: usize, source_total: u32 },
    Tick { folder: String, done: u32, total: u32 },
    FolderDone { report: FolderReport },
    Phase { label: String },
    Warning { message: String },
}

const PROGRESS_EVENT: &str = "migration://progress";
const FETCH_BATCH: usize = 20;
const ENVELOPE_BATCH: usize = 500;

fn emit(app: &tauri::AppHandle, p: Progress) {
    let _ = app.emit(PROGRESS_EVENT, p);
}

// --- IMAP ------------------------------------------------------------------

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

fn list_folders_sync(creds: &ImapCreds) -> Result<Vec<FolderInfo>, String> {
    let mut session = open_session(creds)?;
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
}

fn migrate_email(
    app: &tauri::AppHandle,
    src: &ImapCreds,
    dst: &ImapCreds,
    folders: &[String],
    dry_run: bool,
) -> Result<Vec<FolderReport>, String> {
    let mut src_s = open_session(src)?;
    let mut dst_s = open_session(dst)?;

    emit(app, Progress::Started { folders: folders.len() });

    let mut reports = Vec::new();
    for (i, name) in folders.iter().enumerate() {
        match copy_folder(app, &mut src_s, &mut dst_s, name, i, folders.len(), dry_run) {
            Ok(fr) => {
                emit(app, Progress::FolderDone { report: fr.clone() });
                reports.push(fr);
            }
            Err(e) => {
                emit(app, Progress::Warning { message: format!("{name}: {e}") });
                reports.push(FolderReport { folder: name.clone(), ..Default::default() });
            }
        }
    }

    let _ = src_s.logout();
    let _ = dst_s.logout();
    Ok(reports)
}

fn copy_folder(
    app: &tauri::AppHandle,
    src: &mut ImapSession,
    dst: &mut ImapSession,
    name: &str,
    index: usize,
    folders: usize,
    dry_run: bool,
) -> Result<FolderReport, String> {
    let _ = dst.create(name);

    let dmb = dst.select(name).map_err(|e| format!("select destination: {e}"))?;
    let dest_before = collect_ids_by_seq(dst, dmb.exists)?;

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

    let src_entries = collect_source_entries(src, &uids)?;

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

    if dry_run {
        fr.copied = to_copy.len() as u32;
        return Ok(fr);
    }

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
                            emit(app, Progress::Warning { message: format!("{name}: append failed: {e}") });
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

    let dmb2 = dst.select(name).map_err(|e| format!("re-select destination: {e}"))?;
    let dest_after = collect_ids_by_seq(dst, dmb2.exists)?;
    fr.missing = src_entries
        .iter()
        .filter_map(|(_, id)| id.as_ref())
        .filter(|id| !dest_after.contains(*id))
        .count() as u32;

    Ok(fr)
}

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

fn message_id(f: &Fetch) -> Option<String> {
    f.envelope()
        .and_then(|e| e.message_id.as_deref())
        .map(|b| String::from_utf8_lossy(b).trim().to_string())
        .filter(|s| !s.is_empty())
}

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

// --- Helpers to derive per-protocol creds from an Account ------------------

fn imap_creds(a: &Account) -> Result<ImapCreds, String> {
    let host = a
        .imap_host
        .clone()
        .ok_or("IMAP server is unknown for this account (set it under Advanced).")?;
    Ok(ImapCreds {
        host,
        port: a.imap_port.unwrap_or(993),
        username: a.email.clone(),
        password: a.password.clone(),
    })
}

fn dav_account(a: &Account) -> dav::DavAccount {
    dav::DavAccount {
        url: a.dav_url.clone(),
        username: a.email.clone(),
        password: a.password.clone(),
    }
}

// --- Commands --------------------------------------------------------------

/// Probe an account for everything Godwit can migrate: IMAP folders (with mail
/// server autodiscovery), calendars, and address books.
#[tauri::command]
async fn probe(account: Account) -> Result<Probe, String> {
    tokio::task::spawn_blocking(move || probe_blocking(&account))
        .await
        .map_err(|e| format!("task failed: {e}"))
}

fn probe_blocking(acc: &Account) -> Probe {
    Probe {
        imap: probe_imap(acc),
        calendars: probe_dav(acc, dav::DavKind::Calendar),
        contacts: probe_dav(acc, dav::DavKind::Contacts),
    }
}

fn probe_imap(acc: &Account) -> ImapProbe {
    let (host, port) = if let Some(h) = acc.imap_host.clone().filter(|h| !h.is_empty()) {
        (h, acc.imap_port.unwrap_or(993))
    } else if let Some(s) = autoconfig::discover_imap(&acc.email) {
        (s.host, s.port)
    } else {
        return ImapProbe { status: "needsHost".into(), ..Default::default() };
    };

    let creds = ImapCreds {
        host: host.clone(),
        port,
        username: acc.email.clone(),
        password: acc.password.clone(),
    };
    match list_folders_sync(&creds) {
        Ok(folders) => ImapProbe {
            status: "ok".into(),
            host: Some(host),
            port: Some(port),
            folders,
            error: None,
        },
        Err(e) => ImapProbe {
            status: "error".into(),
            host: Some(host),
            port: Some(port),
            folders: vec![],
            error: Some(e),
        },
    }
}

fn probe_dav(acc: &Account, kind: dav::DavKind) -> DavProbe {
    match dav::discover(&dav_account(acc), kind) {
        Ok(collections) => DavProbe { status: "ok".into(), collections, error: None },
        Err(e) => DavProbe { status: "unavailable".into(), collections: vec![], error: Some(e) },
    }
}

/// Run a migration for the selected folders / calendars / address books.
#[tauri::command]
async fn run_migration(
    app: tauri::AppHandle,
    source: Account,
    destination: Account,
    selection: Selection,
    dry_run: bool,
) -> Result<UnifiedReport, String> {
    tokio::task::spawn_blocking(move || run_blocking(&app, source, destination, selection, dry_run))
        .await
        .map_err(|e| format!("task failed: {e}"))?
}

fn run_blocking(
    app: &tauri::AppHandle,
    source: Account,
    destination: Account,
    selection: Selection,
    dry_run: bool,
) -> Result<UnifiedReport, String> {
    let mut report = UnifiedReport::default();

    if !selection.folders.is_empty() {
        let src = imap_creds(&source)?;
        let dst = imap_creds(&destination)?;
        report.folders = migrate_email(app, &src, &dst, &selection.folders, dry_run)?;
    }

    let src_dav = dav_account(&source);
    let dst_dav = dav_account(&destination);

    for pair in &selection.calendars {
        emit(app, Progress::Phase { label: format!("Calendar: {}", pair.name) });
        let r = dav::migrate(&src_dav, &pair.source, &dst_dav, &pair.dest, dav::DavKind::Calendar, dry_run)
            .unwrap_or_else(|e| {
                emit(app, Progress::Warning { message: format!("{}: {e}", pair.name) });
                dav::DavReport::default()
            });
        report.calendars.push(NamedDavReport { name: pair.name.clone(), report: r });
    }

    for pair in &selection.contacts {
        emit(app, Progress::Phase { label: format!("Address book: {}", pair.name) });
        let r = dav::migrate(&src_dav, &pair.source, &dst_dav, &pair.dest, dav::DavKind::Contacts, dry_run)
            .unwrap_or_else(|e| {
                emit(app, Progress::Warning { message: format!("{}: {e}", pair.name) });
                dav::DavReport::default()
            });
        report.contacts.push(NamedDavReport { name: pair.name.clone(), report: r });
    }

    Ok(report)
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![probe, run_migration])
        .run(tauri::generate_context!())
        .expect("error while running Godwit");
}
