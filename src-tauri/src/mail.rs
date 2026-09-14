//! Protocol-agnostic mail copy: a source is read as raw MIME and a destination
//! is written as raw MIME, so email flows between any combination of IMAP and
//! Microsoft 365 (Graph). Message identity for dedup is the RFC Message-ID
//! (IMAP ENVELOPE / Graph internetMessageId), which is the same value on both.

use std::collections::{HashMap, HashSet};
use std::net::TcpStream;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{DateTime, FixedOffset};
use imap::types::Flag;
use native_tls::TlsStream;
use reqwest::blocking::Client;
use serde_json::{json, Value};

type ImapSession = imap::Session<TlsStream<TcpStream>>;
const GRAPH: &str = "https://graph.microsoft.com/v1.0";
const ENVELOPE_BATCH: usize = 500;

#[derive(Clone)]
pub struct ImapCreds {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

pub fn open_session(creds: &ImapCreds) -> Result<ImapSession, String> {
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| format!("TLS setup failed: {e}"))?;
    let client = imap::connect((creds.host.as_str(), creds.port), creds.host.as_str(), &tls)
        .map_err(|e| format!("Connection failed: {e}"))?;
    client
        .login(&creds.username, &creds.password)
        .map_err(|(e, _c)| format!("Login failed: {e}"))
}

/// One message as seen by a source: an opaque per-source id, its Message-ID
/// (for dedup), and the flags/date worth carrying where the protocols allow.
pub struct MsgRef {
    pub id: String,
    pub message_id: Option<String>,
    pub flags: Vec<Flag<'static>>,
    pub date: Option<DateTime<FixedOffset>>,
}

pub trait MailReader {
    fn list(&mut self, folder: &str) -> Result<Vec<MsgRef>, String>;
    fn fetch_mime(&mut self, folder: &str, msg: &MsgRef) -> Result<Vec<u8>, String>;
}

pub trait MailWriter {
    fn ensure_folder(&mut self, folder: &str) -> Result<(), String>;
    fn existing_message_ids(&mut self, folder: &str) -> Result<HashSet<String>, String>;
    fn append(&mut self, folder: &str, mime: &[u8], msg: &MsgRef) -> Result<(), String>;
}

// --- IMAP ------------------------------------------------------------------

fn imap_message_id(f: &imap::types::Fetch) -> Option<String> {
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

pub struct ImapReader {
    session: ImapSession,
    selected: Option<String>,
}

impl ImapReader {
    pub fn new(session: ImapSession) -> Self {
        Self { session, selected: None }
    }
    fn select_if_needed(&mut self, folder: &str) -> Result<(), String> {
        if self.selected.as_deref() != Some(folder) {
            self.session.select(folder).map_err(|e| format!("select {folder}: {e}"))?;
            self.selected = Some(folder.to_string());
        }
        Ok(())
    }
}

impl MailReader for ImapReader {
    fn list(&mut self, folder: &str) -> Result<Vec<MsgRef>, String> {
        self.select_if_needed(folder)?;
        let mut uids: Vec<u32> = self
            .session
            .uid_search("ALL")
            .map_err(|e| format!("UID SEARCH: {e}"))?
            .into_iter()
            .collect();
        uids.sort_unstable();

        let mut out = Vec::with_capacity(uids.len());
        for chunk in uids.chunks(ENVELOPE_BATCH) {
            let set = chunk.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
            let items = self
                .session
                .uid_fetch(&set, "(UID ENVELOPE FLAGS INTERNALDATE)")
                .map_err(|e| format!("UID FETCH: {e}"))?;
            for f in items.iter() {
                if let Some(uid) = f.uid {
                    out.push(MsgRef {
                        id: uid.to_string(),
                        message_id: imap_message_id(f),
                        flags: map_flags(f.flags()),
                        date: f.internal_date(),
                    });
                }
            }
        }
        Ok(out)
    }

    fn fetch_mime(&mut self, folder: &str, msg: &MsgRef) -> Result<Vec<u8>, String> {
        self.select_if_needed(folder)?;
        let items = self
            .session
            .uid_fetch(&msg.id, "BODY.PEEK[]")
            .map_err(|e| format!("UID FETCH body: {e}"))?;
        items
            .iter()
            .next()
            .and_then(|f| f.body())
            .map(<[u8]>::to_vec)
            .ok_or_else(|| "message body missing".into())
    }
}

pub struct ImapWriter {
    session: ImapSession,
    selected: Option<String>,
}

impl ImapWriter {
    pub fn new(session: ImapSession) -> Self {
        Self { session, selected: None }
    }
    fn select_if_needed(&mut self, folder: &str) -> Result<u32, String> {
        if self.selected.as_deref() != Some(folder) {
            let mb = self.session.select(folder).map_err(|e| format!("select {folder}: {e}"))?;
            self.selected = Some(folder.to_string());
            Ok(mb.exists)
        } else {
            // Re-select to get a current count; cheap.
            let mb = self.session.select(folder).map_err(|e| format!("select {folder}: {e}"))?;
            Ok(mb.exists)
        }
    }
}

impl MailWriter for ImapWriter {
    fn ensure_folder(&mut self, folder: &str) -> Result<(), String> {
        let _ = self.session.create(folder);
        Ok(())
    }

    fn existing_message_ids(&mut self, folder: &str) -> Result<HashSet<String>, String> {
        let exists = self.select_if_needed(folder)?;
        let mut ids = HashSet::new();
        if exists == 0 {
            return Ok(ids);
        }
        let mut start = 1u32;
        while start <= exists {
            let end = (start + ENVELOPE_BATCH as u32 - 1).min(exists);
            let seq = format!("{start}:{end}");
            let items = self
                .session
                .fetch(&seq, "(ENVELOPE)")
                .map_err(|e| format!("ENVELOPE fetch: {e}"))?;
            for f in items.iter() {
                if let Some(id) = imap_message_id(f) {
                    ids.insert(id);
                }
            }
            start = end + 1;
        }
        Ok(ids)
    }

    fn append(&mut self, folder: &str, mime: &[u8], msg: &MsgRef) -> Result<(), String> {
        self.session
            .append_with_flags_and_date(folder, mime, &msg.flags, msg.date)
            .map_err(|e| format!("APPEND: {e}"))
    }
}

// --- Microsoft Graph -------------------------------------------------------

/// Map an IMAP-style folder name to a Graph well-known folder id.
fn well_known(name: &str) -> Option<&'static str> {
    let last = name.rsplit(['.', '/']).next().unwrap_or(name).to_lowercase();
    match last.as_str() {
        "inbox" => Some("inbox"),
        "sent" | "sent messages" | "sent items" | "sentitems" => Some("sentitems"),
        "drafts" => Some("drafts"),
        "trash" | "deleted" | "deleted messages" | "deleted items" => Some("deleteditems"),
        "junk" | "spam" | "junk email" => Some("junkemail"),
        "archive" => Some("archive"),
        _ => None,
    }
}

fn last_segment(name: &str) -> &str {
    name.rsplit(['.', '/']).next().unwrap_or(name)
}

pub struct GraphMail {
    email: String,
    client: Client,
    folder_ids: HashMap<String, String>,
}

impl GraphMail {
    pub fn new(email: String) -> Self {
        Self { email, client: Client::new(), folder_ids: HashMap::new() }
    }

    fn token(&self) -> Result<String, String> {
        crate::microsoft::valid_access(&self.email)
    }

    fn get(&self, url: &str) -> Result<Value, String> {
        let token = self.token()?;
        let resp = self
            .client
            .get(url)
            .bearer_auth(token)
            .send()
            .map_err(|e| format!("Graph GET: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("Graph GET {url} -> HTTP {}", resp.status().as_u16()));
        }
        resp.json().map_err(|e| format!("Graph parse: {e}"))
    }

    /// Resolve a folder name to a Graph folder id, creating it if `create`.
    fn folder_id(&mut self, name: &str, create: bool) -> Result<String, String> {
        if let Some(id) = self.folder_ids.get(name) {
            return Ok(id.clone());
        }
        let id = if let Some(wk) = well_known(name) {
            wk.to_string()
        } else {
            let display = last_segment(name);
            let token = self.token()?;
            let found: Value = self
                .client
                .get(format!("{GRAPH}/me/mailFolders"))
                .bearer_auth(&token)
                .query(&[("$filter", format!("displayName eq '{}'", display.replace('\'', "''")))])
                .send()
                .map_err(|e| format!("folder search: {e}"))?
                .json()
                .map_err(|e| format!("folder search parse: {e}"))?;
            if let Some(existing) =
                found["value"].as_array().and_then(|a| a.first()).and_then(|f| f["id"].as_str())
            {
                existing.to_string()
            } else if create {
                let created: Value = self
                    .client
                    .post(format!("{GRAPH}/me/mailFolders"))
                    .bearer_auth(&token)
                    .json(&json!({ "displayName": display }))
                    .send()
                    .map_err(|e| format!("folder create: {e}"))?
                    .json()
                    .map_err(|e| format!("folder create parse: {e}"))?;
                created["id"].as_str().ok_or("folder create: no id")?.to_string()
            } else {
                return Err(format!("folder '{name}' not found on Microsoft 365"));
            }
        };
        self.folder_ids.insert(name.to_string(), id.clone());
        Ok(id)
    }

    fn message_refs(&self, folder_id: &str, want_flags: bool) -> Result<Vec<MsgRef>, String> {
        let select = if want_flags { "internetMessageId,isRead" } else { "internetMessageId" };
        let mut url =
            format!("{GRAPH}/me/mailFolders/{folder_id}/messages?$select={select}&$top=100");
        let mut out = Vec::new();
        loop {
            let v = self.get(&url)?;
            if let Some(arr) = v["value"].as_array() {
                for m in arr {
                    let flags = if want_flags && m["isRead"].as_bool() == Some(true) {
                        vec![Flag::Seen]
                    } else {
                        Vec::new()
                    };
                    out.push(MsgRef {
                        id: m["id"].as_str().unwrap_or("").to_string(),
                        message_id: m["internetMessageId"].as_str().map(str::to_string),
                        flags,
                        date: None,
                    });
                }
            }
            match v["@odata.nextLink"].as_str() {
                Some(next) => url = next.to_string(),
                None => break,
            }
        }
        Ok(out)
    }
}

impl MailReader for GraphMail {
    fn list(&mut self, folder: &str) -> Result<Vec<MsgRef>, String> {
        let id = self.folder_id(folder, false)?;
        self.message_refs(&id, true)
    }

    fn fetch_mime(&mut self, _folder: &str, msg: &MsgRef) -> Result<Vec<u8>, String> {
        let token = self.token()?;
        let resp = self
            .client
            .get(format!("{GRAPH}/me/messages/{}/$value", msg.id))
            .bearer_auth(token)
            .send()
            .map_err(|e| format!("Graph MIME GET: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("Graph MIME GET -> HTTP {}", resp.status().as_u16()));
        }
        resp.bytes().map(|b| b.to_vec()).map_err(|e| format!("Graph MIME read: {e}"))
    }
}

impl MailWriter for GraphMail {
    fn ensure_folder(&mut self, folder: &str) -> Result<(), String> {
        self.folder_id(folder, true).map(|_| ())
    }

    fn existing_message_ids(&mut self, folder: &str) -> Result<HashSet<String>, String> {
        let id = self.folder_id(folder, true)?;
        Ok(self.message_refs(&id, false)?.into_iter().filter_map(|m| m.message_id).collect())
    }

    fn append(&mut self, folder: &str, mime: &[u8], _msg: &MsgRef) -> Result<(), String> {
        let id = self.folder_id(folder, true)?;
        let token = self.token()?;
        let body = STANDARD.encode(mime);
        let resp = self
            .client
            .post(format!("{GRAPH}/me/mailFolders/{id}/messages"))
            .bearer_auth(token)
            .header("Content-Type", "text/plain")
            .body(body)
            .send()
            .map_err(|e| format!("Graph import: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("Graph import -> HTTP {}", resp.status().as_u16()))
        }
    }
}
