//! Generic host connector: IMAP for mail, CalDAV/CardDAV for calendar/contacts.
//! Contains the low-level IMAP client and the adapters that expose IMAP and DAV
//! as engine `Reader`/`Writer`s.

use super::{Connector, Kind};
use crate::domain::{Account, DavProbe, FolderInfo, ImapProbe, Probe};
use crate::engine::{Item, ItemRef, MailFlag, Reader, Writer};
use crate::{autoconfig, convert, dav};

use imap::types::Flag;
use native_tls::TlsStream;
use std::collections::HashSet;
use std::net::TcpStream;

type ImapSession = imap::Session<TlsStream<TcpStream>>;
const ENVELOPE_BATCH: usize = 500;

#[derive(Clone)]
pub struct ImapCreds {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

fn open_session(creds: &ImapCreds) -> Result<ImapSession, String> {
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| format!("TLS setup failed: {e}"))?;
    let client = imap::connect((creds.host.as_str(), creds.port), creds.host.as_str(), &tls)
        .map_err(|e| format!("Connection failed: {e}"))?;
    client
        .login(&creds.username, &creds.password)
        .map_err(|(e, _c)| format!("Login failed: {e}"))
}

/// XOAUTH2 SASL authenticator (Gmail and other OAuth-only IMAP servers).
struct XOAuth2 {
    user: String,
    token: String,
}

impl imap::Authenticator for XOAuth2 {
    type Response = String;
    fn process(&self, _challenge: &[u8]) -> Self::Response {
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
    }
}

fn open_xoauth2(host: &str, port: u16, user: &str, token: &str) -> Result<ImapSession, String> {
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| format!("TLS setup failed: {e}"))?;
    let client = imap::connect((host, port), host, &tls).map_err(|e| format!("Connection failed: {e}"))?;
    let auth = XOAuth2 { user: user.to_string(), token: token.to_string() };
    client.authenticate("XOAUTH2", &auth).map_err(|(e, _c)| format!("XOAUTH2 auth failed: {e}"))
}

/// Build an engine mail reader over an XOAUTH2 IMAP session (e.g. Gmail).
pub fn xoauth2_reader(host: &str, port: u16, user: &str, token: &str) -> Result<Box<dyn Reader>, String> {
    Ok(Box::new(ImapReader { session: open_xoauth2(host, port, user, token)?, selected: None }))
}

pub fn xoauth2_writer(host: &str, port: u16, user: &str, token: &str) -> Result<Box<dyn Writer>, String> {
    Ok(Box::new(ImapWriter { session: open_xoauth2(host, port, user, token)? }))
}

pub fn xoauth2_folders(host: &str, port: u16, user: &str, token: &str) -> Result<Vec<FolderInfo>, String> {
    list_folders_session(open_xoauth2(host, port, user, token)?)
}

/// Build engine calendar/contacts reader/writer over a DAV account (which may
/// carry a bearer token — used by the Google connector).
pub fn dav_reader(account: dav::DavAccount, kind: dav::DavKind) -> Box<dyn Reader> {
    Box::new(DavReader { account, kind, cache: Vec::new() })
}

pub fn dav_writer(account: dav::DavAccount, kind: dav::DavKind) -> Box<dyn Writer> {
    Box::new(DavWriter { account, kind })
}

fn imap_message_id(f: &imap::types::Fetch) -> String {
    f.envelope()
        .and_then(|e| e.message_id.as_deref())
        .map(|b| String::from_utf8_lossy(b).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default()
}

fn to_mail_flags(src: &[Flag]) -> Vec<MailFlag> {
    let mut out = Vec::new();
    for f in src {
        match f {
            Flag::Seen => out.push(MailFlag::Seen),
            Flag::Answered => out.push(MailFlag::Answered),
            Flag::Flagged => out.push(MailFlag::Flagged),
            Flag::Draft => out.push(MailFlag::Draft),
            _ => {}
        }
    }
    out
}

fn to_imap_flags(src: &[MailFlag]) -> Vec<Flag<'static>> {
    src.iter()
        .map(|f| match f {
            MailFlag::Seen => Flag::Seen,
            MailFlag::Answered => Flag::Answered,
            MailFlag::Flagged => Flag::Flagged,
            MailFlag::Draft => Flag::Draft,
        })
        .collect()
}

// --- IMAP mail reader / writer --------------------------------------------

pub struct ImapReader {
    session: ImapSession,
    selected: Option<String>,
}

impl ImapReader {
    fn select_if_needed(&mut self, folder: &str) -> Result<(), String> {
        if self.selected.as_deref() != Some(folder) {
            self.session.select(folder).map_err(|e| format!("select {folder}: {e}"))?;
            self.selected = Some(folder.to_string());
        }
        Ok(())
    }
}

impl Reader for ImapReader {
    fn list(&mut self, folder: &str) -> Result<Vec<ItemRef>, String> {
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
                .uid_fetch(&set, "(UID ENVELOPE)")
                .map_err(|e| format!("UID FETCH: {e}"))?;
            for f in items.iter() {
                if let Some(uid) = f.uid {
                    out.push(ItemRef { id: uid.to_string(), key: imap_message_id(f) });
                }
            }
        }
        Ok(out)
    }

    fn fetch(&mut self, folder: &str, item: &ItemRef) -> Result<Item, String> {
        self.select_if_needed(folder)?;
        let items = self
            .session
            .uid_fetch(&item.id, "(FLAGS INTERNALDATE BODY.PEEK[])")
            .map_err(|e| format!("UID FETCH body: {e}"))?;
        let f = items.iter().next().ok_or("message body missing")?;
        let body = f.body().ok_or("message body missing")?.to_vec();
        Ok(Item::mime(item.key.clone(), body, to_mail_flags(f.flags()), f.internal_date()))
    }
}

pub struct ImapWriter {
    session: ImapSession,
}

impl Writer for ImapWriter {
    fn ensure(&mut self, folder: &str) -> Result<(), String> {
        let _ = self.session.create(folder);
        Ok(())
    }

    fn existing_keys(&mut self, folder: &str) -> Result<HashSet<String>, String> {
        let mb = self.session.select(folder).map_err(|e| format!("select {folder}: {e}"))?;
        let mut ids = HashSet::new();
        if mb.exists == 0 {
            return Ok(ids);
        }
        let mut start = 1u32;
        while start <= mb.exists {
            let end = (start + ENVELOPE_BATCH as u32 - 1).min(mb.exists);
            let items = self
                .session
                .fetch(format!("{start}:{end}"), "(ENVELOPE)")
                .map_err(|e| format!("ENVELOPE fetch: {e}"))?;
            for f in items.iter() {
                let id = imap_message_id(f);
                if !id.is_empty() {
                    ids.insert(id);
                }
            }
            start = end + 1;
        }
        Ok(ids)
    }

    fn write(&mut self, folder: &str, item: &Item) -> Result<(), String> {
        let flags = to_imap_flags(&item.flags);
        self.session
            .append_with_flags_and_date(folder, item.bytes(), &flags, item.date)
            .map_err(|e| format!("APPEND: {e}"))
    }
}

// --- DAV calendar / contacts reader / writer ------------------------------

fn key_of(kind: dav::DavKind, body: &str) -> String {
    match kind {
        dav::DavKind::Calendar => convert::event_key(body),
        dav::DavKind::Contacts => convert::contact_key(body),
    }
}

fn ext_and_type(kind: dav::DavKind) -> (&'static str, &'static str) {
    match kind {
        dav::DavKind::Calendar => (".ics", "text/calendar; charset=utf-8"),
        dav::DavKind::Contacts => (".vcf", "text/vcard; charset=utf-8"),
    }
}

fn sanitize_name(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

pub struct DavReader {
    account: dav::DavAccount,
    kind: dav::DavKind,
    cache: Vec<(String, String)>, // (key, body)
}

impl Reader for DavReader {
    fn list(&mut self, collection: &str) -> Result<Vec<ItemRef>, String> {
        let bodies = dav::read_items(&self.account, collection)?;
        self.cache = bodies.into_iter().map(|b| (key_of(self.kind, &b), b)).collect();
        Ok(self
            .cache
            .iter()
            .enumerate()
            .map(|(i, (k, _))| ItemRef { id: i.to_string(), key: k.clone() })
            .collect())
    }

    fn fetch(&mut self, _collection: &str, item: &ItemRef) -> Result<Item, String> {
        let i: usize = item.id.parse().map_err(|_| "bad item id".to_string())?;
        let (k, b) = self.cache.get(i).ok_or("item no longer available")?;
        Ok(Item::text(k.clone(), b.clone()))
    }
}

pub struct DavWriter {
    account: dav::DavAccount,
    kind: dav::DavKind,
}

impl Writer for DavWriter {
    fn ensure(&mut self, _collection: &str) -> Result<(), String> {
        Ok(())
    }

    fn existing_keys(&mut self, collection: &str) -> Result<HashSet<String>, String> {
        Ok(dav::read_items(&self.account, collection)?
            .iter()
            .map(|b| key_of(self.kind, b))
            .filter(|k| !k.is_empty())
            .collect())
    }

    fn write(&mut self, collection: &str, item: &Item) -> Result<(), String> {
        let text = item.as_text();
        let key = if item.key.is_empty() { key_of(self.kind, &text) } else { item.key.clone() };
        let base = if key.is_empty() { "item".to_string() } else { sanitize_name(&key) };
        let (ext, ct) = ext_and_type(self.kind);
        dav::write_item(&self.account, collection, &format!("{base}{ext}"), ct, &text)
    }
}

// --- Connector ------------------------------------------------------------

pub struct ImapDavConnector {
    account: Account,
}

impl ImapDavConnector {
    pub fn new(account: Account) -> Self {
        Self { account }
    }

    fn imap_creds(&self) -> Result<ImapCreds, String> {
        let host = self
            .account
            .imap_host
            .clone()
            .filter(|h| !h.is_empty())
            .ok_or("IMAP server is unknown for this account (set it under Advanced).")?;
        Ok(ImapCreds {
            host,
            port: self.account.imap_port.unwrap_or(993),
            username: self.account.email.clone(),
            password: self.account.password.clone(),
        })
    }

    fn dav_account(&self) -> dav::DavAccount {
        dav::DavAccount {
            url: self.account.dav_url.clone(),
            username: self.account.email.clone(),
            password: self.account.password.clone(),
            bearer: None,
        }
    }
}

fn list_folders(creds: &ImapCreds) -> Result<Vec<FolderInfo>, String> {
    list_folders_session(open_session(creds)?)
}

fn list_folders_session(mut session: ImapSession) -> Result<Vec<FolderInfo>, String> {
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

fn probe_imap(conn: &ImapDavConnector) -> ImapProbe {
    let acc = &conn.account;
    let (host, port) = if let Some(h) = acc.imap_host.clone().filter(|h| !h.is_empty()) {
        (h, acc.imap_port.unwrap_or(993))
    } else if let Some(s) = autoconfig::discover_imap(&acc.email) {
        (s.host, s.port)
    } else {
        return ImapProbe { status: "needsHost".into(), ..Default::default() };
    };
    let creds = ImapCreds { host: host.clone(), port, username: acc.email.clone(), password: acc.password.clone() };
    match list_folders(&creds) {
        Ok(folders) => ImapProbe { status: "ok".into(), host: Some(host), port: Some(port), folders, error: None },
        Err(e) => ImapProbe { status: "error".into(), host: Some(host), port: Some(port), folders: vec![], error: Some(e) },
    }
}

fn probe_dav(conn: &ImapDavConnector, kind: dav::DavKind) -> DavProbe {
    match dav::discover(&conn.dav_account(), kind) {
        Ok(collections) => DavProbe { status: "ok".into(), collections, error: None },
        Err(e) => DavProbe { status: "unavailable".into(), collections: vec![], error: Some(e) },
    }
}

impl Connector for ImapDavConnector {
    fn probe(&self) -> Probe {
        Probe {
            imap: probe_imap(self),
            calendars: probe_dav(self, dav::DavKind::Calendar),
            contacts: probe_dav(self, dav::DavKind::Contacts),
        }
    }

    fn folders(&self) -> Result<Vec<FolderInfo>, String> {
        list_folders(&self.imap_creds()?)
    }

    fn remaps_folders(&self) -> bool {
        true
    }

    fn reader(&self, kind: Kind) -> Result<Box<dyn Reader>, String> {
        match kind {
            Kind::Mail => Ok(Box::new(ImapReader {
                session: open_session(&self.imap_creds()?)?,
                selected: None,
            })),
            Kind::Calendar => Ok(Box::new(DavReader { account: self.dav_account(), kind: dav::DavKind::Calendar, cache: vec![] })),
            Kind::Contacts => Ok(Box::new(DavReader { account: self.dav_account(), kind: dav::DavKind::Contacts, cache: vec![] })),
        }
    }

    fn writer(&self, kind: Kind) -> Result<Box<dyn Writer>, String> {
        match kind {
            Kind::Mail => Ok(Box::new(ImapWriter { session: open_session(&self.imap_creds()?)? })),
            Kind::Calendar => Ok(Box::new(DavWriter { account: self.dav_account(), kind: dav::DavKind::Calendar })),
            Kind::Contacts => Ok(Box::new(DavWriter { account: self.dav_account(), kind: dav::DavKind::Contacts })),
        }
    }
}
