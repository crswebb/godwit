//! JMAP connector (RFC 8620 core + RFC 8621 mail + RFC 9610 contacts).
//!
//! **Email and contacts.** Calendars over JMAP are still an IETF draft, so this
//! connector covers mail (RFC 8621) and contacts (RFC 9610); migrate calendars
//! via CalDAV (the password connector) until the calendar spec is final. Auth
//! is a bearer **API token**, entered as the account password — what Fastmail
//! requires and what modern JMAP servers expect, so there is no separate OAuth
//! sign-in.
//!
//! Everything maps onto Godwit's canonical items with no new interchange
//! format: mail rides as raw RFC 5322 (blob download / `Email/import`), and
//! contacts ride as vCard, converting to/from JSContact (RFC 9553) in `convert`
//! per RFC 9555. Dedup keys match the other connectors (Message-ID for mail,
//! email/FN for contacts), so dedup works across protocols.

use super::{Connector, Kind};
use crate::domain::{Account, DavProbe, FolderInfo, ImapProbe, Probe};
use crate::engine::{self, Item, ItemRef, MailFlag, Reader, Writer};
use crate::{convert, dav};

use chrono::{SecondsFormat, Utc};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const CORE_CAP: &str = "urn:ietf:params:jmap:core";
const MAIL_CAP: &str = "urn:ietf:params:jmap:mail";
const CONTACTS_CAP: &str = "urn:ietf:params:jmap:contacts";
const MAIL_USING: [&str; 2] = [CORE_CAP, MAIL_CAP];
const CONTACTS_USING: [&str; 2] = [CORE_CAP, CONTACTS_CAP];
const DEFAULT_GET_BATCH: usize = 500;
const QUERY_PAGE: usize = 1000;
const CALENDAR_NOTE: &str = "JMAP calendars are still an IETF draft — migrate calendars over CalDAV (use email & password).";
const CONTACTS_UNADVERTISED: &str = "This JMAP server doesn't advertise contacts support (urn:ietf:params:jmap:contacts).";

// --- Session resource ------------------------------------------------------

#[derive(Clone)]
struct Session {
    api_url: String,
    download_url: String,
    upload_url: String,
    account_id: String,
    /// Primary account for contacts (RFC 9610); may differ from the mail
    /// account, and is absent when the server doesn't support contacts.
    contacts_account: Option<String>,
    max_get: usize,
}

fn origin_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .map(|u| format!("{}://{}", u.scheme(), u.authority()))
        .unwrap_or_default()
}

/// Resolve a possibly-relative session URL against the request base.
fn abs(base: &str, u: &str) -> String {
    if u.starts_with("http://") || u.starts_with("https://") {
        u.to_string()
    } else {
        format!("{}{}", origin_of(base), u)
    }
}

fn parse_session(v: &Value, base: &str) -> Result<Session, String> {
    let get = |k: &str| v.get(k).and_then(Value::as_str);
    let api_url = get("apiUrl").ok_or("JMAP session response had no apiUrl.")?;
    let download_url = get("downloadUrl").ok_or("JMAP session response had no downloadUrl.")?;
    let upload_url = get("uploadUrl").ok_or("JMAP session response had no uploadUrl.")?;
    let primary = |cap: &str| v.get("primaryAccounts").and_then(|p| p.get(cap)).and_then(Value::as_str).map(str::to_string);
    let account_id = primary(MAIL_CAP).ok_or("This JMAP account has no mail capability (urn:ietf:params:jmap:mail).")?;
    let max_get = v
        .get("capabilities")
        .and_then(|c| c.get(CORE_CAP))
        .and_then(|c| c.get("maxObjectsInGet"))
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_GET_BATCH);
    Ok(Session {
        api_url: abs(base, api_url),
        download_url: abs(base, download_url),
        upload_url: abs(base, upload_url),
        account_id,
        contacts_account: primary(CONTACTS_CAP),
        max_get,
    })
}

/// Where to fetch the session resource: the explicit override, else
/// `.well-known/jmap` on the email's domain (RFC 8620 §2.2).
fn session_url(account: &Account) -> Result<String, String> {
    if let Some(u) = account.jmap_url.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(u.to_string());
    }
    let domain = account
        .email
        .split('@')
        .nth(1)
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .ok_or("Enter a full email address (you@domain) or a JMAP server URL.")?;
    Ok(format!("https://{domain}/.well-known/jmap"))
}

// --- JMAP client -----------------------------------------------------------

struct Jmap {
    client: Client,
    token: String,
    session: Session,
}

impl Jmap {
    fn connect(account: &Account) -> Result<Self, String> {
        let token = account.password.trim().to_string();
        if token.is_empty() {
            return Err("Enter your JMAP API token as the password.".into());
        }
        let client = Client::new();
        let url = session_url(account)?;
        let resp = client
            .get(&url)
            .bearer_auth(&token)
            .header(ACCEPT, "application/json")
            .send()
            .map_err(|e| format!("JMAP session request: {e}"))?;
        if !resp.status().is_success() {
            let code = resp.status();
            let hint = if code.as_u16() == 401 || code.as_u16() == 403 { " — check the API token." } else { "" };
            return Err(format!("JMAP session {code} from {url}{hint}"));
        }
        let v: Value = resp.json().map_err(|e| format!("JMAP session parse: {e}"))?;
        let session = parse_session(&v, &url)?;
        Ok(Self { client, token, session })
    }

    fn acct(&self) -> &str {
        &self.session.account_id
    }

    fn contacts_account(&self) -> Result<String, String> {
        self.session.contacts_account.clone().ok_or_else(|| CONTACTS_UNADVERTISED.into())
    }

    /// POST one or more method calls under the given capability set.
    fn request(&self, using: &[&str], calls: Value) -> Result<Value, String> {
        let body = json!({ "using": using, "methodCalls": calls });
        let resp = self
            .client
            .post(&self.session.api_url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .map_err(|e| format!("JMAP request: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("JMAP API {}: {}", resp.status(), resp.text().unwrap_or_default()));
        }
        resp.json().map_err(|e| format!("JMAP response parse: {e}"))
    }

    /// Run a single method call and return its arguments (an `error` response
    /// becomes an `Err`).
    fn call(&self, using: &[&str], method: &str, args: Value) -> Result<Value, String> {
        let v = self.request(using, json!([[method, args, "0"]]))?;
        first_response(&v, method)
    }

    fn download(&self, blob_id: &str) -> Result<Vec<u8>, String> {
        let url = fill_download(&self.session.download_url, self.acct(), blob_id);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .map_err(|e| format!("JMAP blob download: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("JMAP blob download {}", resp.status()));
        }
        Ok(resp.bytes().map_err(|e| format!("JMAP blob read: {e}"))?.to_vec())
    }

    fn upload(&self, bytes: Vec<u8>) -> Result<String, String> {
        let url = self.session.upload_url.replace("{accountId}", self.acct());
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.token)
            .header(CONTENT_TYPE, "message/rfc822")
            .body(bytes)
            .send()
            .map_err(|e| format!("JMAP blob upload: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("JMAP blob upload {}: {}", resp.status(), resp.text().unwrap_or_default()));
        }
        let v: Value = resp.json().map_err(|e| format!("JMAP upload parse: {e}"))?;
        v.get("blobId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "JMAP upload response had no blobId.".into())
    }

    /// All object ids matching a `*/query` filter, paged so large collections
    /// aren't truncated by a server-clamped query window.
    fn query(&self, using: &[&str], method: &str, account_id: &str, filter: Value, sort: Value) -> Result<Vec<String>, String> {
        let mut all: Vec<String> = Vec::new();
        let mut position = 0usize;
        let mut total: Option<usize> = None;
        loop {
            let mut args = serde_json::Map::new();
            args.insert("accountId".into(), json!(account_id));
            args.insert("filter".into(), filter.clone());
            if !sort.is_null() {
                args.insert("sort".into(), sort.clone());
            }
            args.insert("position".into(), json!(position));
            args.insert("limit".into(), json!(QUERY_PAGE));
            args.insert("calculateTotal".into(), json!(total.is_none()));
            let resp = self.call(using, method, Value::Object(args))?;
            if total.is_none() {
                total = resp.get("total").and_then(Value::as_u64).map(|n| n as usize);
            }
            let ids = resp.get("ids").and_then(Value::as_array).cloned().unwrap_or_default();
            let n = ids.len();
            if n == 0 {
                break;
            }
            all.extend(ids.into_iter().filter_map(|v| v.as_str().map(str::to_string)));
            position += n;
            if matches!(total, Some(t) if all.len() >= t) {
                break;
            }
        }
        Ok(all)
    }
}

// --- Pure helpers (unit-tested) --------------------------------------------

fn fill_download(template: &str, account_id: &str, blob_id: &str) -> String {
    template
        .replace("{accountId}", account_id)
        .replace("{blobId}", blob_id)
        .replace("{type}", "application%2Foctet-stream")
        .replace("{name}", "message")
}

/// JMAP keyword object → the mail flags Godwit carries.
fn keywords_to_flags(v: &Value) -> Vec<MailFlag> {
    let is_set = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    let mut out = Vec::new();
    if is_set("$seen") {
        out.push(MailFlag::Seen);
    }
    if is_set("$answered") {
        out.push(MailFlag::Answered);
    }
    if is_set("$flagged") {
        out.push(MailFlag::Flagged);
    }
    if is_set("$draft") {
        out.push(MailFlag::Draft);
    }
    out
}

/// Mail flags → a JMAP keyword object for `Email/import`.
fn flags_to_keywords(flags: &[MailFlag]) -> Value {
    let mut map = serde_json::Map::new();
    for f in flags {
        let k = match f {
            MailFlag::Seen => "$seen",
            MailFlag::Answered => "$answered",
            MailFlag::Flagged => "$flagged",
            MailFlag::Draft => "$draft",
        };
        map.insert(k.to_string(), Value::Bool(true));
    }
    Value::Object(map)
}

/// First `Message-ID` from an `Email/get` object (the dedup key); empty when
/// the message has none, which the engine treats as "always copy".
fn message_id_of(email: &Value) -> String {
    email
        .get("messageId")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn first_response(v: &Value, expect: &str) -> Result<Value, String> {
    let responses = v.get("methodResponses").and_then(Value::as_array).ok_or("JMAP: no methodResponses")?;
    let first = responses.first().and_then(Value::as_array).ok_or("JMAP: empty methodResponses")?;
    let name = first.first().and_then(Value::as_str).unwrap_or("");
    let args = first.get(1).cloned().unwrap_or(Value::Null);
    if name == "error" {
        let t = args.get("type").and_then(Value::as_str).unwrap_or("unknown");
        return Err(format!("JMAP {expect} error: {t}"));
    }
    Ok(args)
}

// --- Mailbox tree ----------------------------------------------------------

#[derive(Clone)]
struct MailboxNode {
    id: String,
    name: String,
    parent: Option<String>,
    role: Option<String>,
}

struct MailboxSet {
    nodes: Vec<MailboxNode>,
}

impl MailboxSet {
    fn from_list(list: &[Value]) -> Self {
        let nodes = list
            .iter()
            .filter_map(|m| {
                let id = m.get("id").and_then(Value::as_str)?.to_string();
                let name = m.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                let parent = m.get("parentId").and_then(Value::as_str).map(str::to_string);
                let role = m.get("role").and_then(Value::as_str).map(str::to_string);
                Some(MailboxNode { id, name, parent, role })
            })
            .collect();
        MailboxSet { nodes }
    }

    fn get(&self, id: &str) -> Option<&MailboxNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Full "/"-joined path of a mailbox, walking the parent chain.
    fn path_of(&self, id: &str) -> String {
        let mut parts = Vec::new();
        let mut cur = Some(id.to_string());
        let mut guard = 0;
        while let Some(c) = cur {
            guard += 1;
            if guard > 64 {
                break; // cycle guard
            }
            match self.get(&c) {
                Some(node) => {
                    parts.push(node.name.clone());
                    cur = node.parent.clone();
                }
                None => break,
            }
        }
        parts.reverse();
        parts.join("/")
    }

    fn role_id(&self, role: &str) -> Option<String> {
        self.nodes.iter().find(|n| n.role.as_deref() == Some(role)).map(|n| n.id.clone())
    }

    /// Resolve a folder name (as produced by `folders()` or mapped by
    /// `plan_targets`) to a mailbox id. INBOX and special-use names fall back to
    /// the matching JMAP role so cross-provider folder names land correctly.
    fn resolve(&self, name: &str) -> Option<String> {
        let want = name.trim();
        if want.eq_ignore_ascii_case("INBOX") {
            if let Some(id) = self.role_id("inbox") {
                return Some(id);
            }
        }
        if let Some(id) = self.nodes.iter().find(|n| self.path_of(&n.id).eq_ignore_ascii_case(want)).map(|n| n.id.clone()) {
            return Some(id);
        }
        let leaf = want.rsplit('/').next().unwrap_or(want);
        if let Some(canon) = engine::canonical_special(leaf) {
            if let Some(id) = self.role_id(canon) {
                return Some(id);
            }
        }
        None
    }

    fn folder_infos(&self) -> Vec<FolderInfo> {
        let mut out: Vec<FolderInfo> = self
            .nodes
            .iter()
            .map(|n| FolderInfo {
                name: self.path_of(&n.id),
                delimiter: Some("/".to_string()),
                attributes: n.role.iter().map(|r| format!("role:{r}")).collect(),
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }
}

fn load_mailboxes(j: &Jmap) -> Result<MailboxSet, String> {
    let args = j.call(
        &MAIL_USING,
        "Mailbox/get",
        json!({ "accountId": j.acct(), "ids": Value::Null, "properties": ["id", "name", "parentId", "role"] }),
    )?;
    let list = args.get("list").and_then(Value::as_array).ok_or("Mailbox/get returned no list.")?;
    Ok(MailboxSet::from_list(list))
}

// --- Mail reader -----------------------------------------------------------

struct EmailMeta {
    blob_id: String,
    keywords: Vec<MailFlag>,
    date: Option<chrono::DateTime<chrono::FixedOffset>>,
}

fn mail_sort() -> Value {
    json!([{ "property": "receivedAt", "isAscending": true }])
}

struct JmapReader {
    j: Jmap,
    set: MailboxSet,
    cache: HashMap<String, EmailMeta>,
}

impl Reader for JmapReader {
    fn list(&mut self, collection: &str) -> Result<Vec<ItemRef>, String> {
        let mailbox = self.set.resolve(collection).ok_or_else(|| format!("No JMAP mailbox matches \"{collection}\"."))?;
        let ids = self.j.query(&MAIL_USING, "Email/query", self.j.acct(), json!({ "inMailbox": mailbox }), mail_sort())?;
        let mut out = Vec::with_capacity(ids.len());
        for chunk in ids.chunks(self.j.session.max_get) {
            let args = self.j.call(
                &MAIL_USING,
                "Email/get",
                json!({ "accountId": self.j.acct(), "ids": chunk, "properties": ["id", "blobId", "messageId", "keywords", "receivedAt"] }),
            )?;
            let list = args.get("list").and_then(Value::as_array).cloned().unwrap_or_default();
            for e in &list {
                let Some(id) = e.get("id").and_then(Value::as_str) else { continue };
                let Some(blob_id) = e.get("blobId").and_then(Value::as_str) else { continue };
                let key = message_id_of(e);
                let date = e.get("receivedAt").and_then(Value::as_str).and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
                self.cache.insert(
                    id.to_string(),
                    EmailMeta { blob_id: blob_id.to_string(), keywords: keywords_to_flags(e.get("keywords").unwrap_or(&Value::Null)), date },
                );
                out.push(ItemRef { id: id.to_string(), key });
            }
        }
        Ok(out)
    }

    fn fetch(&mut self, _collection: &str, item: &ItemRef) -> Result<Item, String> {
        let meta = self.cache.get(&item.id).ok_or("email is no longer available")?;
        let bytes = self.j.download(&meta.blob_id)?;
        Ok(Item::mime(item.key.clone(), bytes, meta.keywords.clone(), meta.date))
    }
}

// --- Mail writer -----------------------------------------------------------

struct JmapWriter {
    j: Jmap,
    set: MailboxSet,
}

impl JmapWriter {
    fn create_mailbox(&self, name: &str, parent: Option<&str>) -> Result<String, String> {
        let mut props = json!({ "name": name });
        if let Some(p) = parent {
            props["parentId"] = json!(p);
        }
        let args = self.j.call(&MAIL_USING, "Mailbox/set", json!({ "accountId": self.j.acct(), "create": { "m": props } }))?;
        if let Some(id) = args.get("created").and_then(|c| c.get("m")).and_then(|m| m.get("id")).and_then(Value::as_str) {
            return Ok(id.to_string());
        }
        let why = args
            .get("notCreated")
            .and_then(|n| n.get("m"))
            .and_then(|e| e.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        Err(format!("couldn't create mailbox \"{name}\": {why}"))
    }

    /// Resolve `path`, creating any missing "/"-separated levels. The local set
    /// is updated so later calls reuse the new ids.
    fn ensure_path(&mut self, path: &str) -> Result<String, String> {
        if let Some(id) = self.set.resolve(path) {
            return Ok(id);
        }
        let mut parent: Option<String> = None;
        let mut acc = String::new();
        for seg in path.split('/').filter(|s| !s.is_empty()) {
            acc = if acc.is_empty() { seg.to_string() } else { format!("{acc}/{seg}") };
            if let Some(id) = self.set.resolve(&acc) {
                parent = Some(id);
                continue;
            }
            let id = self.create_mailbox(seg, parent.as_deref())?;
            self.set.nodes.push(MailboxNode { id: id.clone(), name: seg.to_string(), parent: parent.clone(), role: None });
            parent = Some(id);
        }
        parent.ok_or_else(|| format!("empty mailbox name: \"{path}\""))
    }
}

impl Writer for JmapWriter {
    fn ensure(&mut self, collection: &str) -> Result<(), String> {
        self.ensure_path(collection).map(|_| ())
    }

    fn existing_keys(&mut self, collection: &str) -> Result<HashSet<String>, String> {
        let mailbox = self.set.resolve(collection).ok_or_else(|| format!("No JMAP mailbox matches \"{collection}\"."))?;
        let ids = self.j.query(&MAIL_USING, "Email/query", self.j.acct(), json!({ "inMailbox": mailbox }), mail_sort())?;
        let mut keys = HashSet::new();
        for chunk in ids.chunks(self.j.session.max_get) {
            let args = self.j.call(&MAIL_USING, "Email/get", json!({ "accountId": self.j.acct(), "ids": chunk, "properties": ["messageId"] }))?;
            if let Some(list) = args.get("list").and_then(Value::as_array) {
                for e in list {
                    let id = message_id_of(e);
                    if !id.is_empty() {
                        keys.insert(id);
                    }
                }
            }
        }
        Ok(keys)
    }

    fn write(&mut self, collection: &str, item: &Item) -> Result<(), String> {
        let mailbox = self.ensure_path(collection)?;
        let blob_id = self.j.upload(item.bytes())?;
        let mut email = json!({ "blobId": blob_id, "mailboxIds": { mailbox: true }, "keywords": flags_to_keywords(&item.flags) });
        if let Some(d) = item.date {
            // JMAP UTCDate must be UTC with a trailing "Z".
            email["receivedAt"] = json!(d.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Secs, true));
        }
        let args = self.j.call(&MAIL_USING, "Email/import", json!({ "accountId": self.j.acct(), "emails": { "x": email } }))?;
        if args.get("created").and_then(|c| c.get("x")).is_some() {
            return Ok(());
        }
        let why = args.get("notCreated").and_then(|n| n.get("x")).and_then(|e| e.get("type")).and_then(Value::as_str).unwrap_or("unknown");
        // A message already present in the account is a successful dedup, not a
        // failure — the engine's verify pass confirms it landed.
        if why == "alreadyExists" {
            return Ok(());
        }
        Err(format!("Email/import rejected: {why}"))
    }
}

// --- Contacts (RFC 9610) ---------------------------------------------------

fn list_address_books(j: &Jmap, account: &str) -> Result<Vec<dav::DavCollection>, String> {
    let args = j.call(&CONTACTS_USING, "AddressBook/get", json!({ "accountId": account, "ids": Value::Null, "properties": ["id", "name"] }))?;
    let list = args.get("list").and_then(Value::as_array).ok_or("AddressBook/get returned no list.")?;
    Ok(list
        .iter()
        .filter_map(|b| {
            let id = b.get("id").and_then(Value::as_str)?.to_string();
            let name = b.get("name").and_then(Value::as_str).unwrap_or("Contacts").to_string();
            Some(dav::DavCollection { name, href: id, count: None })
        })
        .collect())
}

/// Fetch full JSContact Cards for the given ids.
fn get_cards(j: &Jmap, account: &str, ids: &[String]) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for chunk in ids.chunks(j.session.max_get) {
        let args = j.call(&CONTACTS_USING, "ContactCard/get", json!({ "accountId": account, "ids": chunk }))?;
        if let Some(list) = args.get("list").and_then(Value::as_array) {
            out.extend(list.iter().cloned());
        }
    }
    Ok(out)
}

fn card_ids(j: &Jmap, account: &str, book: &str) -> Result<Vec<String>, String> {
    j.query(&CONTACTS_USING, "ContactCard/query", account, json!({ "inAddressBook": book }), Value::Null)
}

struct JmapContactsReader {
    j: Jmap,
    account: String,
    cache: HashMap<String, String>, // card id -> vCard body
}

impl Reader for JmapContactsReader {
    fn list(&mut self, collection: &str) -> Result<Vec<ItemRef>, String> {
        let ids = card_ids(&self.j, &self.account, collection)?;
        let cards = get_cards(&self.j, &self.account, &ids)?;
        let mut out = Vec::with_capacity(cards.len());
        for card in &cards {
            let Some(id) = card.get("id").and_then(Value::as_str) else { continue };
            let body = convert::jscontact_to_vcard(card);
            let key = convert::contact_key(&body);
            self.cache.insert(id.to_string(), body);
            out.push(ItemRef { id: id.to_string(), key });
        }
        Ok(out)
    }

    fn fetch(&mut self, _collection: &str, item: &ItemRef) -> Result<Item, String> {
        let body = self.cache.get(&item.id).ok_or("contact is no longer available")?;
        Ok(Item::text(item.key.clone(), body.clone()))
    }
}

struct JmapContactsWriter {
    j: Jmap,
    account: String,
}

impl Writer for JmapContactsWriter {
    fn ensure(&mut self, _collection: &str) -> Result<(), String> {
        // The destination address book is chosen from the probe, so it exists.
        Ok(())
    }

    fn existing_keys(&mut self, collection: &str) -> Result<HashSet<String>, String> {
        let ids = card_ids(&self.j, &self.account, collection)?;
        let cards = get_cards(&self.j, &self.account, &ids)?;
        Ok(cards
            .iter()
            .map(|c| convert::contact_key(&convert::jscontact_to_vcard(c)))
            .filter(|k| !k.is_empty())
            .collect())
    }

    fn write(&mut self, collection: &str, item: &Item) -> Result<(), String> {
        let mut card = convert::vcard_to_jscontact(&item.as_text());
        if let Some(obj) = card.as_object_mut() {
            obj.insert("addressBookIds".into(), json!({ collection: true }));
        }
        let args = self.j.call(&CONTACTS_USING, "ContactCard/set", json!({ "accountId": self.account, "create": { "x": card } }))?;
        if args.get("created").and_then(|c| c.get("x")).is_some() {
            return Ok(());
        }
        let why = args.get("notCreated").and_then(|n| n.get("x")).and_then(|e| e.get("type")).and_then(Value::as_str).unwrap_or("unknown");
        Err(format!("ContactCard/set rejected: {why}"))
    }
}

// --- Connector -------------------------------------------------------------

pub struct JmapConnector {
    account: Account,
}

impl JmapConnector {
    pub fn new(account: Account) -> Self {
        Self { account }
    }

    fn load(&self) -> Result<(Jmap, MailboxSet), String> {
        let j = Jmap::connect(&self.account)?;
        let set = load_mailboxes(&j)?;
        Ok((j, set))
    }
}

fn dav_unavailable(note: &str) -> DavProbe {
    DavProbe { status: "unavailable".into(), collections: vec![], error: Some(note.to_string()) }
}

impl Connector for JmapConnector {
    fn probe(&self) -> Probe {
        let (j, set) = match self.load() {
            Ok(pair) => pair,
            Err(e) => {
                return Probe {
                    imap: ImapProbe { status: "error".into(), error: Some(e.clone()), ..Default::default() },
                    calendars: dav_unavailable(CALENDAR_NOTE),
                    contacts: dav_unavailable(&e),
                }
            }
        };
        let imap = ImapProbe {
            status: "ok".into(),
            host: Some(origin_of(&j.session.api_url)),
            port: None,
            folders: set.folder_infos(),
            error: None,
        };
        let contacts = match &j.session.contacts_account {
            Some(account) => match list_address_books(&j, account) {
                Ok(collections) => DavProbe { status: "ok".into(), collections, error: None },
                Err(e) => dav_unavailable(&e),
            },
            None => dav_unavailable(CONTACTS_UNADVERTISED),
        };
        Probe { imap, calendars: dav_unavailable(CALENDAR_NOTE), contacts }
    }

    fn folders(&self) -> Result<Vec<FolderInfo>, String> {
        let (_, set) = self.load()?;
        Ok(set.folder_infos())
    }

    fn remaps_folders(&self) -> bool {
        true
    }

    fn reader(&self, kind: Kind) -> Result<Box<dyn Reader>, String> {
        match kind {
            Kind::Mail => {
                let (j, set) = self.load()?;
                Ok(Box::new(JmapReader { j, set, cache: HashMap::new() }))
            }
            Kind::Contacts => {
                let j = Jmap::connect(&self.account)?;
                let account = j.contacts_account()?;
                Ok(Box::new(JmapContactsReader { j, account, cache: HashMap::new() }))
            }
            Kind::Calendar => Err(CALENDAR_NOTE.into()),
        }
    }

    fn writer(&self, kind: Kind) -> Result<Box<dyn Writer>, String> {
        match kind {
            Kind::Mail => {
                let (j, set) = self.load()?;
                Ok(Box::new(JmapWriter { j, set }))
            }
            Kind::Contacts => {
                let j = Jmap::connect(&self.account)?;
                let account = j.contacts_account()?;
                Ok(Box::new(JmapContactsWriter { j, account }))
            }
            Kind::Calendar => Err(CALENDAR_NOTE.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SESSION: &str = r#"{
        "apiUrl": "https://api.example.com/jmap/api/",
        "downloadUrl": "https://api.example.com/jmap/download/{accountId}/{blobId}/{name}?type={type}",
        "uploadUrl": "https://api.example.com/jmap/upload/{accountId}/",
        "capabilities": { "urn:ietf:params:jmap:core": { "maxObjectsInGet": 128 } },
        "primaryAccounts": {
            "urn:ietf:params:jmap:mail": "acct-42",
            "urn:ietf:params:jmap:contacts": "acct-42"
        }
    }"#;

    #[test]
    fn parses_session_identity_and_limits() {
        let v: Value = serde_json::from_str(SAMPLE_SESSION).unwrap();
        let s = parse_session(&v, "https://example.com/.well-known/jmap").unwrap();
        assert_eq!(s.account_id, "acct-42");
        assert_eq!(s.contacts_account.as_deref(), Some("acct-42"));
        assert_eq!(s.api_url, "https://api.example.com/jmap/api/");
        assert_eq!(s.max_get, 128);
    }

    #[test]
    fn session_without_contacts_capability_has_no_contacts_account() {
        let v: Value = serde_json::json!({
            "apiUrl": "https://x/api", "downloadUrl": "https://x/d/{blobId}", "uploadUrl": "https://x/u",
            "primaryAccounts": { "urn:ietf:params:jmap:mail": "m1" }
        });
        let s = parse_session(&v, "https://x").unwrap();
        assert_eq!(s.account_id, "m1");
        assert!(s.contacts_account.is_none());
    }

    #[test]
    fn session_without_mail_capability_is_an_error() {
        let v: Value = serde_json::json!({
            "apiUrl": "https://x/api", "downloadUrl": "https://x/d/{blobId}", "uploadUrl": "https://x/u",
            "primaryAccounts": { "urn:ietf:params:jmap:submission": "s1" }
        });
        assert!(parse_session(&v, "https://x").is_err());
    }

    #[test]
    fn relative_session_urls_are_absolutized() {
        let v: Value = serde_json::json!({
            "apiUrl": "/jmap/api", "downloadUrl": "/d/{blobId}", "uploadUrl": "/u/{accountId}",
            "primaryAccounts": { "urn:ietf:params:jmap:mail": "a" }
        });
        let s = parse_session(&v, "https://host.example:8443/.well-known/jmap").unwrap();
        assert_eq!(s.api_url, "https://host.example:8443/jmap/api");
        assert_eq!(s.max_get, DEFAULT_GET_BATCH); // falls back when unspecified
    }

    fn sample_set() -> MailboxSet {
        MailboxSet::from_list(&[
            serde_json::json!({ "id": "mb-in", "name": "Inbox", "parentId": null, "role": "inbox" }),
            serde_json::json!({ "id": "mb-sent", "name": "Sent", "parentId": null, "role": "sent" }),
            serde_json::json!({ "id": "mb-bin", "name": "Bin", "parentId": null, "role": "trash" }),
            serde_json::json!({ "id": "mb-arch", "name": "Archive", "parentId": null, "role": "archive" }),
            serde_json::json!({ "id": "mb-2024", "name": "2024", "parentId": "mb-arch", "role": null }),
        ])
    }

    #[test]
    fn builds_nested_mailbox_paths() {
        let set = sample_set();
        assert_eq!(set.path_of("mb-2024"), "Archive/2024");
        assert_eq!(set.path_of("mb-in"), "Inbox");
    }

    #[test]
    fn resolve_uses_inbox_role_exact_path_and_special_use() {
        let set = sample_set();
        assert_eq!(set.resolve("INBOX").as_deref(), Some("mb-in"));
        assert_eq!(set.resolve("archive/2024").as_deref(), Some("mb-2024"));
        assert_eq!(set.resolve("Trash").as_deref(), Some("mb-bin"));
        assert_eq!(set.resolve("Deleted Items").as_deref(), Some("mb-bin"));
        assert_eq!(set.resolve("Projects"), None);
    }

    #[test]
    fn keywords_round_trip_to_flags() {
        let kw = flags_to_keywords(&[MailFlag::Seen, MailFlag::Flagged]);
        assert_eq!(kw.get("$seen").and_then(Value::as_bool), Some(true));
        assert_eq!(kw.get("$flagged").and_then(Value::as_bool), Some(true));
        assert!(kw.get("$draft").is_none());

        let back = keywords_to_flags(&kw);
        assert!(back.contains(&MailFlag::Seen) && back.contains(&MailFlag::Flagged));
        assert_eq!(back.len(), 2);
        let other = serde_json::json!({ "$seen": true, "$phishing": true, "custom": true });
        assert_eq!(keywords_to_flags(&other), vec![MailFlag::Seen]);
    }

    #[test]
    fn extracts_first_message_id() {
        let e = serde_json::json!({ "messageId": ["  <abc@host>  "] });
        assert_eq!(message_id_of(&e), "<abc@host>");
        assert_eq!(message_id_of(&serde_json::json!({ "messageId": null })), "");
        assert_eq!(message_id_of(&serde_json::json!({})), "");
    }

    #[test]
    fn fills_download_template() {
        let url = fill_download("https://h/d/{accountId}/{blobId}/{name}?type={type}", "a1", "b2");
        assert_eq!(url, "https://h/d/a1/b2/message?type=application%2Foctet-stream");
    }

    #[test]
    fn first_response_surfaces_method_errors() {
        let ok = serde_json::json!({ "methodResponses": [["Email/get", { "list": [] }, "0"]] });
        assert!(first_response(&ok, "Email/get").is_ok());
        let err = serde_json::json!({ "methodResponses": [["error", { "type": "unknownMethod" }, "0"]] });
        assert!(first_response(&err, "Email/get").unwrap_err().contains("unknownMethod"));
    }
}
