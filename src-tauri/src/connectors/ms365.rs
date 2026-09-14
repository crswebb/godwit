//! Microsoft 365 connector (Graph). Adapts Graph mail (raw MIME), calendar and
//! contacts (JSON, converted to/from iCalendar/vCard) to the engine traits.
//! Uses the OAuth/token client in `crate::microsoft`.

use super::{Connector, Kind};
use crate::domain::{Account, DavProbe, FolderInfo, ImapProbe, Probe};
use crate::engine::{Item, ItemRef, MailFlag, Reader, Writer};
use crate::{convert, dav, microsoft};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

fn graph_get(email: &str, url: &str) -> Result<Value, String> {
    let token = microsoft::valid_access(email)?;
    let resp = Client::new()
        .get(url)
        .bearer_auth(token)
        .send()
        .map_err(|e| format!("Graph GET: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Graph GET {url} -> HTTP {}", resp.status().as_u16()));
    }
    resp.json().map_err(|e| format!("Graph parse: {e}"))
}

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

fn resolve_folder(email: &str, cache: &mut HashMap<String, String>, name: &str, create: bool) -> Result<String, String> {
    if let Some(id) = cache.get(name) {
        return Ok(id.clone());
    }
    let id = if let Some(wk) = well_known(name) {
        wk.to_string()
    } else {
        let display = name.rsplit(['.', '/']).next().unwrap_or(name);
        let token = microsoft::valid_access(email)?;
        let found: Value = Client::new()
            .get(format!("{GRAPH}/me/mailFolders"))
            .bearer_auth(&token)
            .query(&[("$filter", format!("displayName eq '{}'", display.replace('\'', "''")))])
            .send()
            .map_err(|e| format!("folder search: {e}"))?
            .json()
            .map_err(|e| format!("folder search parse: {e}"))?;
        if let Some(existing) = found["value"].as_array().and_then(|a| a.first()).and_then(|f| f["id"].as_str()) {
            existing.to_string()
        } else if create {
            let created: Value = Client::new()
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
    cache.insert(name.to_string(), id.clone());
    Ok(id)
}

/// (message id, internetMessageId, isRead) for each message in a folder.
fn message_refs(email: &str, folder_id: &str, want_flags: bool) -> Result<Vec<(String, String, bool)>, String> {
    let select = if want_flags { "internetMessageId,isRead" } else { "internetMessageId" };
    let mut url = format!("{GRAPH}/me/mailFolders/{folder_id}/messages?$select={select}&$top=100");
    let mut out = Vec::new();
    loop {
        let v = graph_get(email, &url)?;
        if let Some(arr) = v["value"].as_array() {
            for m in arr {
                out.push((
                    m["id"].as_str().unwrap_or("").to_string(),
                    m["internetMessageId"].as_str().unwrap_or("").to_string(),
                    m["isRead"].as_bool().unwrap_or(false),
                ));
            }
        }
        match v["@odata.nextLink"].as_str() {
            Some(next) => url = next.to_string(),
            None => break,
        }
    }
    Ok(out)
}

// --- Mail -----------------------------------------------------------------

pub struct GraphMailReader {
    email: String,
    folders: HashMap<String, String>,
    is_read: HashMap<String, bool>,
}

impl Reader for GraphMailReader {
    fn list(&mut self, folder: &str) -> Result<Vec<ItemRef>, String> {
        let id = resolve_folder(&self.email, &mut self.folders, folder, false)?;
        let refs = message_refs(&self.email, &id, true)?;
        let mut out = Vec::with_capacity(refs.len());
        for (id, imid, is_read) in refs {
            self.is_read.insert(id.clone(), is_read);
            out.push(ItemRef { id, key: imid });
        }
        Ok(out)
    }

    fn fetch(&mut self, _folder: &str, item: &ItemRef) -> Result<Item, String> {
        let token = microsoft::valid_access(&self.email)?;
        let resp = Client::new()
            .get(format!("{GRAPH}/me/messages/{}/$value", item.id))
            .bearer_auth(token)
            .send()
            .map_err(|e| format!("Graph MIME GET: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("Graph MIME GET -> HTTP {}", resp.status().as_u16()));
        }
        let body = resp.bytes().map_err(|e| format!("Graph MIME read: {e}"))?.to_vec();
        let flags = if *self.is_read.get(&item.id).unwrap_or(&false) { vec![MailFlag::Seen] } else { vec![] };
        Ok(Item::mime(item.key.clone(), body, flags, None))
    }
}

pub struct GraphMailWriter {
    email: String,
    folders: HashMap<String, String>,
}

impl Writer for GraphMailWriter {
    fn ensure(&mut self, folder: &str) -> Result<(), String> {
        resolve_folder(&self.email, &mut self.folders, folder, true).map(|_| ())
    }

    fn existing_keys(&mut self, folder: &str) -> Result<HashSet<String>, String> {
        let id = resolve_folder(&self.email, &mut self.folders, folder, true)?;
        Ok(message_refs(&self.email, &id, false)?
            .into_iter()
            .map(|(_, imid, _)| imid)
            .filter(|k| !k.is_empty())
            .collect())
    }

    fn write(&mut self, folder: &str, item: &Item) -> Result<(), String> {
        let id = resolve_folder(&self.email, &mut self.folders, folder, true)?;
        let token = microsoft::valid_access(&self.email)?;
        let body = STANDARD.encode(item.bytes());
        let resp = Client::new()
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

// --- Calendar / contacts (JSON <-> iCal/vCard) ----------------------------

/// The two Graph JSON collections share one shape: list to text, create from text.
enum Pim {
    Calendar,
    Contacts,
}

impl Pim {
    fn list_text(&self, email: &str) -> Result<Vec<String>, String> {
        match self {
            Pim::Calendar => Ok(microsoft::list_events(email)?.iter().map(convert::graph_event_to_ics).collect()),
            Pim::Contacts => Ok(microsoft::list_contacts(email)?.iter().map(convert::graph_contact_to_vcard).collect()),
        }
    }
    fn key_of(&self, text: &str) -> String {
        match self {
            Pim::Calendar => convert::event_key(text),
            Pim::Contacts => convert::contact_key(text),
        }
    }
    fn create(&self, email: &str, text: &str) -> Result<(), String> {
        match self {
            Pim::Calendar => microsoft::create_event(email, &convert::ics_to_graph_event(text)),
            Pim::Contacts => microsoft::create_contact(email, &convert::vcard_to_graph_contact(text)),
        }
    }
}

pub struct GraphPimReader {
    email: String,
    pim: Pim,
    cache: Vec<(String, String)>,
}

impl Reader for GraphPimReader {
    fn list(&mut self, _collection: &str) -> Result<Vec<ItemRef>, String> {
        self.cache = self.pim.list_text(&self.email)?.into_iter().map(|t| (self.pim.key_of(&t), t)).collect();
        Ok(self.cache.iter().enumerate().map(|(i, (k, _))| ItemRef { id: i.to_string(), key: k.clone() }).collect())
    }
    fn fetch(&mut self, _collection: &str, item: &ItemRef) -> Result<Item, String> {
        let i: usize = item.id.parse().map_err(|_| "bad item id".to_string())?;
        let (k, t) = self.cache.get(i).ok_or("item no longer available")?;
        Ok(Item::text(k.clone(), t.clone()))
    }
}

pub struct GraphPimWriter {
    email: String,
    pim: Pim,
}

impl Writer for GraphPimWriter {
    fn ensure(&mut self, _collection: &str) -> Result<(), String> {
        Ok(())
    }
    fn existing_keys(&mut self, _collection: &str) -> Result<HashSet<String>, String> {
        Ok(self.pim.list_text(&self.email)?.iter().map(|t| self.pim.key_of(t)).filter(|k| !k.is_empty()).collect())
    }
    fn write(&mut self, _collection: &str, item: &Item) -> Result<(), String> {
        self.pim.create(&self.email, &item.as_text())
    }
}

// --- Connector ------------------------------------------------------------

pub struct Ms365Connector {
    account: Account,
}

impl Ms365Connector {
    pub fn new(account: Account) -> Self {
        Self { account }
    }
}

impl Connector for Ms365Connector {
    fn probe(&self) -> Probe {
        match microsoft::probe(&self.account.email) {
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

    fn folders(&self) -> Result<Vec<FolderInfo>, String> {
        // Graph resolves folders by name itself, so mapping isn't needed here.
        Ok(vec![])
    }

    fn remaps_folders(&self) -> bool {
        false
    }

    fn reader(&self, kind: Kind) -> Result<Box<dyn Reader>, String> {
        let email = self.account.email.clone();
        Ok(match kind {
            Kind::Mail => Box::new(GraphMailReader { email, folders: HashMap::new(), is_read: HashMap::new() }),
            Kind::Calendar => Box::new(GraphPimReader { email, pim: Pim::Calendar, cache: vec![] }),
            Kind::Contacts => Box::new(GraphPimReader { email, pim: Pim::Contacts, cache: vec![] }),
        })
    }

    fn writer(&self, kind: Kind) -> Result<Box<dyn Writer>, String> {
        let email = self.account.email.clone();
        Ok(match kind {
            Kind::Mail => Box::new(GraphMailWriter { email, folders: HashMap::new() }),
            Kind::Calendar => Box::new(GraphPimWriter { email, pim: Pim::Calendar }),
            Kind::Contacts => Box::new(GraphPimWriter { email, pim: Pim::Contacts }),
        })
    }
}
