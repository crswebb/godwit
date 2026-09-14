// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autoconfig;
mod dav;
mod mail;
mod microsoft;

use mail::{MailReader, MailWriter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

impl Account {
    fn is_microsoft(&self) -> bool {
        self.provider == "microsoft"
    }
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

fn emit(app: &tauri::AppHandle, p: Progress) {
    let _ = app.emit(PROGRESS_EVENT, p);
}

// --- Account -> protocol creds --------------------------------------------

fn imap_creds(a: &Account) -> Result<mail::ImapCreds, String> {
    let host = a
        .imap_host
        .clone()
        .filter(|h| !h.is_empty())
        .ok_or("IMAP server is unknown for this account (set it under Advanced).")?;
    Ok(mail::ImapCreds {
        host,
        port: a.imap_port.unwrap_or(993),
        username: a.email.clone(),
        password: a.password.clone(),
    })
}

fn dav_account(a: &Account) -> dav::DavAccount {
    dav::DavAccount { url: a.dav_url.clone(), username: a.email.clone(), password: a.password.clone() }
}

fn make_reader(a: &Account) -> Result<Box<dyn MailReader>, String> {
    if a.is_microsoft() {
        Ok(Box::new(mail::GraphMail::new(a.email.clone())))
    } else {
        Ok(Box::new(mail::ImapReader::new(mail::open_session(&imap_creds(a)?)?)))
    }
}

fn make_writer(a: &Account) -> Result<Box<dyn MailWriter>, String> {
    if a.is_microsoft() {
        Ok(Box::new(mail::GraphMail::new(a.email.clone())))
    } else {
        Ok(Box::new(mail::ImapWriter::new(mail::open_session(&imap_creds(a)?)?)))
    }
}

fn list_folders_sync(creds: &mail::ImapCreds) -> Result<Vec<FolderInfo>, String> {
    let mut session = mail::open_session(creds)?;
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

// --- Mail copy (protocol-agnostic) ----------------------------------------

fn migrate_mail(
    app: &tauri::AppHandle,
    reader: &mut dyn MailReader,
    writer: &mut dyn MailWriter,
    pairs: &[(String, String)],
    dry_run: bool,
) -> Vec<FolderReport> {
    emit(app, Progress::Started { folders: pairs.len() });
    let mut reports = Vec::new();
    for (i, (src, dst)) in pairs.iter().enumerate() {
        match copy_folder(app, reader, writer, src, dst, i, pairs.len(), dry_run) {
            Ok(fr) => {
                emit(app, Progress::FolderDone { report: fr.clone() });
                reports.push(fr);
            }
            Err(e) => {
                emit(app, Progress::Warning { message: format!("{src}: {e}") });
                let folder = if src == dst { src.clone() } else { format!("{src} → {dst}") };
                reports.push(FolderReport { folder, ..Default::default() });
            }
        }
    }
    reports
}

#[allow(clippy::too_many_arguments)]
fn copy_folder(
    app: &tauri::AppHandle,
    reader: &mut dyn MailReader,
    writer: &mut dyn MailWriter,
    src_folder: &str,
    dst_folder: &str,
    index: usize,
    folders: usize,
    dry_run: bool,
) -> Result<FolderReport, String> {
    writer.ensure_folder(dst_folder)?;
    let dest_before = writer.existing_message_ids(dst_folder)?;
    let src = reader.list(src_folder)?;
    let total = src.len() as u32;
    emit(app, Progress::FolderStart { name: src_folder.to_string(), index, folders, source_total: total });

    let to_copy: Vec<_> = src
        .iter()
        .filter(|m| match &m.message_id {
            Some(id) => !dest_before.contains(id),
            None => true,
        })
        .collect();

    let display = if src_folder == dst_folder {
        src_folder.to_string()
    } else {
        format!("{src_folder} → {dst_folder}")
    };
    let mut fr = FolderReport {
        folder: display,
        source_total: total,
        skipped: (src.len() - to_copy.len()) as u32,
        ..Default::default()
    };

    if dry_run {
        fr.copied = to_copy.len() as u32;
        return Ok(fr);
    }

    let target = to_copy.len() as u32;
    let mut done = 0u32;
    for m in to_copy {
        done += 1;
        match reader.fetch_mime(src_folder, m) {
            Ok(mime) => match writer.append(dst_folder, &mime, m) {
                Ok(()) => fr.copied += 1,
                Err(e) => {
                    fr.failed += 1;
                    emit(app, Progress::Warning { message: format!("{src_folder}: {e}") });
                }
            },
            Err(e) => {
                fr.failed += 1;
                emit(app, Progress::Warning { message: format!("{src_folder}: {e}") });
            }
        }
        if done % 25 == 0 || done == target {
            emit(app, Progress::Tick { folder: src_folder.to_string(), done, total: target });
        }
    }

    let dest_after = writer.existing_message_ids(dst_folder)?;
    fr.missing = src
        .iter()
        .filter_map(|m| m.message_id.as_ref())
        .filter(|id| !dest_after.contains(*id))
        .count() as u32;

    Ok(fr)
}

// --- Folder mapping between hosts -----------------------------------------

fn is_inbox(name: &str) -> bool {
    name.eq_ignore_ascii_case("INBOX")
}

fn leaf_of(name: &str, delim: Option<&str>) -> String {
    match delim {
        Some(d) if !d.is_empty() => name.rsplit(d).next().unwrap_or(name).to_string(),
        _ => name.to_string(),
    }
}

/// Re-express a folder path using the destination's hierarchy delimiter.
fn translate_path(name: &str, src: Option<&str>, dst: Option<&str>) -> String {
    match (src, dst) {
        (Some(s), Some(d)) if s != d && !s.is_empty() && !d.is_empty() => name.replace(s, d),
        _ => name.to_string(),
    }
}

/// Normalise a folder leaf name to a special-use category, if any.
fn canonical_special(leaf: &str) -> Option<&'static str> {
    match leaf.trim().to_lowercase().as_str() {
        "sent" | "sent messages" | "sent items" | "sent mail" => Some("sent"),
        "drafts" | "draft" => Some("drafts"),
        "trash" | "deleted" | "deleted messages" | "deleted items" | "bin" => Some("trash"),
        "junk" | "spam" | "junk email" | "junk e-mail" => Some("junk"),
        "archive" | "archives" => Some("archive"),
        _ => None,
    }
}

/// Map each selected source folder to the right destination folder name:
/// translate the hierarchy delimiter between hosts and route special-use
/// folders (Sent/Drafts/Trash/Junk/Archive) to the destination's existing one.
/// A Microsoft 365 destination resolves folders itself (Graph), so names pass
/// through unchanged.
fn plan_targets(
    source: &Account,
    destination: &Account,
    folders: &[String],
) -> Result<Vec<(String, String)>, String> {
    if destination.is_microsoft() {
        return Ok(folders.iter().map(|f| (f.clone(), f.clone())).collect());
    }

    let dst_list = list_folders_sync(&imap_creds(destination)?)?;
    let dst_delim = dst_list.iter().find_map(|f| f.delimiter.clone());
    let src_delim = if source.is_microsoft() {
        None
    } else {
        list_folders_sync(&imap_creds(source)?)?.iter().find_map(|f| f.delimiter.clone())
    };

    // Index the destination's special-use folders by category.
    let mut dst_special: HashMap<&'static str, String> = HashMap::new();
    for f in &dst_list {
        let leaf = leaf_of(&f.name, dst_delim.as_deref());
        if let Some(c) = canonical_special(&leaf) {
            dst_special.entry(c).or_insert_with(|| f.name.clone());
        }
    }

    let pairs = folders
        .iter()
        .map(|f| {
            if is_inbox(f) {
                return (f.clone(), "INBOX".to_string());
            }
            let leaf = leaf_of(f, src_delim.as_deref());
            if let Some(c) = canonical_special(&leaf) {
                if let Some(dest_name) = dst_special.get(c) {
                    return (f.clone(), dest_name.clone());
                }
            }
            (f.clone(), translate_path(f, src_delim.as_deref(), dst_delim.as_deref()))
        })
        .collect();
    Ok(pairs)
}

// --- Commands --------------------------------------------------------------

/// Probe an account for everything Godwit can migrate.
#[tauri::command]
async fn probe(account: Account) -> Result<Probe, String> {
    tokio::task::spawn_blocking(move || probe_blocking(&account))
        .await
        .map_err(|e| format!("task failed: {e}"))
}

fn probe_blocking(acc: &Account) -> Probe {
    if acc.is_microsoft() {
        return probe_microsoft(acc);
    }
    Probe {
        imap: probe_imap(acc),
        calendars: probe_dav(acc, dav::DavKind::Calendar),
        contacts: probe_dav(acc, dav::DavKind::Contacts),
    }
}

fn probe_microsoft(acc: &Account) -> Probe {
    match microsoft::probe(&acc.email) {
        Ok(mp) => {
            let folders = mp
                .folders
                .into_iter()
                .map(|f| FolderInfo { name: f.name, delimiter: None, attributes: vec![] })
                .collect();
            let as_collections = |names: Vec<String>| {
                names
                    .into_iter()
                    .map(|n| dav::DavCollection { name: n.clone(), href: n, count: None })
                    .collect::<Vec<_>>()
            };
            Probe {
                imap: ImapProbe { status: "ok".into(), folders, ..Default::default() },
                calendars: DavProbe { status: "ok".into(), collections: as_collections(mp.calendars), error: None },
                contacts: DavProbe { status: "ok".into(), collections: as_collections(mp.contact_folders), error: None },
            }
        }
        Err(e) => Probe {
            imap: ImapProbe { status: "error".into(), error: Some(e.clone()), ..Default::default() },
            calendars: DavProbe { status: "unavailable".into(), error: Some(e.clone()), ..Default::default() },
            contacts: DavProbe { status: "unavailable".into(), error: Some(e), ..Default::default() },
        },
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

    let creds = mail::ImapCreds {
        host: host.clone(),
        port,
        username: acc.email.clone(),
        password: acc.password.clone(),
    };
    match list_folders_sync(&creds) {
        Ok(folders) => ImapProbe { status: "ok".into(), host: Some(host), port: Some(port), folders, error: None },
        Err(e) => ImapProbe { status: "error".into(), host: Some(host), port: Some(port), folders: vec![], error: Some(e) },
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
        let pairs = plan_targets(&source, &destination, &selection.folders)?;
        let mut reader = make_reader(&source)?;
        let mut writer = make_writer(&destination)?;
        report.folders = migrate_mail(app, reader.as_mut(), writer.as_mut(), &pairs, dry_run);
    }

    // Calendar/contacts run over CalDAV/CardDAV. M365 (Graph JSON) conversion
    // isn't built yet, so skip those pairs with a clear warning.
    let dav_ok = !source.is_microsoft() && !destination.is_microsoft();
    let src_dav = dav_account(&source);
    let dst_dav = dav_account(&destination);

    let run_dav = |app: &tauri::AppHandle, pairs: &[DavPair], kind: dav::DavKind, out: &mut Vec<NamedDavReport>| {
        for pair in pairs {
            if !dav_ok {
                emit(app, Progress::Warning {
                    message: format!("{}: Microsoft 365 calendar/contacts migration isn't supported yet — skipped.", pair.name),
                });
                out.push(NamedDavReport { name: pair.name.clone(), report: dav::DavReport::default() });
                continue;
            }
            emit(app, Progress::Phase { label: format!("{}", pair.name) });
            let r = dav::migrate(&src_dav, &pair.source, &dst_dav, &pair.dest, kind, dry_run)
                .unwrap_or_else(|e| {
                    emit(app, Progress::Warning { message: format!("{}: {e}", pair.name) });
                    dav::DavReport::default()
                });
            out.push(NamedDavReport { name: pair.name.clone(), report: r });
        }
    };

    run_dav(app, &selection.calendars, dav::DavKind::Calendar, &mut report.calendars);
    run_dav(app, &selection.contacts, dav::DavKind::Contacts, &mut report.contacts);

    Ok(report)
}

/// Sign in to a Microsoft 365 account via OAuth (opens the system browser).
#[tauri::command]
async fn ms_sign_in(client_id: String) -> Result<microsoft::MsAccount, String> {
    tokio::task::spawn_blocking(move || microsoft::sign_in(&client_id))
        .await
        .map_err(|e| format!("task failed: {e}"))?
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![probe, run_migration, ms_sign_in])
        .run(tauri::generate_context!())
        .expect("error while running Godwit");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_hierarchy_delimiter() {
        assert_eq!(translate_path("INBOX.Archive.2023", Some("."), Some("/")), "INBOX/Archive/2023");
        assert_eq!(translate_path("A/B", Some("/"), Some("/")), "A/B"); // same delim: unchanged
        assert_eq!(translate_path("Sent", Some("."), None), "Sent"); // unknown dest delim: unchanged
    }

    #[test]
    fn recognises_special_use_synonyms() {
        assert_eq!(canonical_special("Sent Items"), Some("sent"));
        assert_eq!(canonical_special("sent messages"), Some("sent"));
        assert_eq!(canonical_special("JUNK"), Some("junk"));
        assert_eq!(canonical_special("Deleted Items"), Some("trash"));
        assert_eq!(canonical_special("Projects"), None);
    }

    #[test]
    fn takes_leaf_by_delimiter() {
        assert_eq!(leaf_of("INBOX.Sent", Some(".")), "Sent");
        assert_eq!(leaf_of("INBOX/Archive/Old", Some("/")), "Old");
        assert_eq!(leaf_of("Sent Items", None), "Sent Items");
    }

    #[test]
    fn inbox_is_case_insensitive() {
        assert!(is_inbox("INBOX"));
        assert!(is_inbox("inbox"));
        assert!(!is_inbox("INBOX.Sent"));
    }
}
