//! Minimal CalDAV / CardDAV client: autodiscover + list collections and
//! (later) read/write items. Hand-rolled on reqwest + quick-xml because the
//! Rust DAV-client crates are immature. Namespace-prefix-agnostic (matches XML
//! by local name).
//!
//! Autodiscovery follows RFC 6764's well-known URIs: a PROPFIND to
//! `https://<domain>/.well-known/caldav` is redirected by the server to the
//! real context path. (SRV-record discovery — needed to fully auto-resolve
//! Google/iCloud — is a future addition; those use the manual URL override.)

use quick_xml::events::Event;
use quick_xml::Reader;
use reqwest::blocking::Client;
use reqwest::header::LOCATION;
use reqwest::redirect::Policy;
use reqwest::{Method, Url};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct DavAccount {
    /// Optional explicit DAV base URL. When empty, discovery uses the domain of
    /// `username` (an email address) via `.well-known`.
    #[serde(default)]
    pub url: Option<String>,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DavKind {
    Calendar,
    Contacts,
}

#[derive(Debug, Serialize)]
pub struct DavCollection {
    pub name: String,
    pub href: String,
    pub count: Option<u32>,
}

struct ResponseEntry {
    href: String,
    displayname: String,
    resourcetypes: Vec<String>,
    contenttype: String,
}

fn http_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        // Follow well-known redirects manually so PROPFIND method + body survive.
        .redirect(Policy::none())
        .build()
        .map_err(|e| format!("HTTP client init failed: {e}"))
}

/// PROPFIND with manual redirect handling. Returns the response body and the
/// final URL it was served from (so relative hrefs resolve correctly even after
/// a well-known redirect to another host).
fn propfind(
    client: &Client,
    acc: &DavAccount,
    url: &Url,
    depth: &str,
    body: &str,
) -> Result<(String, Url), String> {
    let mut current = url.clone();
    for _ in 0..6 {
        let method = Method::from_bytes(b"PROPFIND").expect("valid method");
        let resp = client
            .request(method, current.clone())
            .basic_auth(&acc.username, Some(&acc.password))
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body.to_string())
            .send()
            .map_err(|e| format!("PROPFIND {current} failed: {e}"))?;

        let status = resp.status();

        if status.is_redirection() {
            let loc = resp
                .headers()
                .get(LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("{current} redirected without a Location header"))?;
            let next = current.join(loc).map_err(|e| format!("bad redirect target: {e}"))?;
            if next.scheme() != "https" {
                return Err("refusing to follow a non-HTTPS redirect".into());
            }
            current = next;
            continue;
        }

        let text = resp.text().map_err(|e| format!("reading response from {current}: {e}"))?;
        if status.as_u16() == 207 || status.is_success() {
            return Ok((text, current));
        }
        if status.as_u16() == 401 {
            return Err("Authentication failed (check username/password).".into());
        }
        return Err(format!("{current} returned HTTP {}", status.as_u16()));
    }
    Err("too many redirects during discovery".into())
}

/// Work out where to begin discovery: an explicit URL if given, otherwise the
/// `.well-known` path on the domain of the (email) username.
fn start_url(acc: &DavAccount, kind: DavKind) -> Result<Url, String> {
    if let Some(u) = acc.url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        return Url::parse(u).map_err(|e| format!("Invalid URL: {e}"));
    }
    let domain = acc
        .username
        .split('@')
        .nth(1)
        .filter(|d| !d.is_empty())
        .ok_or("Enter your email address as the username, or add a server URL under Advanced.")?;
    let wk = match kind {
        DavKind::Calendar => "caldav",
        DavKind::Contacts => "carddav",
    };
    Url::parse(&format!("https://{domain}/.well-known/{wk}"))
        .map_err(|e| format!("Could not build discovery URL: {e}"))
}

/// Discover the calendars or address books for an account.
pub fn discover(acc: &DavAccount, kind: DavKind) -> Result<Vec<DavCollection>, String> {
    let client = http_client()?;
    let start = start_url(acc, kind)?;

    // 1. current-user-principal (this also resolves any well-known redirect).
    let body = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:current-user-principal/></d:prop></d:propfind>"#;
    let (xml, base) = propfind(&client, acc, &start, "0", body)?;
    let principal_href = first_href_in(&xml, "current-user-principal")
        .ok_or("Couldn't find a calendar/contacts service for that account. Try adding the server URL under Advanced.")?;
    let principal = base.join(&principal_href).map_err(|e| format!("resolving principal: {e}"))?;

    // 2. home-set for the requested kind
    let (body, home_local, coll_local) = match kind {
        DavKind::Calendar => (
            r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><c:calendar-home-set/></d:prop></d:propfind>"#,
            "calendar-home-set",
            "calendar",
        ),
        DavKind::Contacts => (
            r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav"><d:prop><card:addressbook-home-set/></d:prop></d:propfind>"#,
            "addressbook-home-set",
            "addressbook",
        ),
    };
    let (xml, _) = propfind(&client, acc, &principal, "0", body)?;
    let home_href = first_href_in(&xml, home_local)
        .ok_or("Server did not return a home-set for this data type.")?;
    let home = base.join(&home_href).map_err(|e| format!("resolving home-set: {e}"))?;

    // 3. enumerate collections under the home-set
    let body = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:displayname/></d:prop></d:propfind>"#;
    let (xml, _) = propfind(&client, acc, &home, "1", body)?;

    let mut out = Vec::new();
    for r in parse_responses(&xml) {
        if r.resourcetypes.iter().any(|t| t == coll_local) {
            let coll_url = base.join(&r.href).map_err(|e| format!("resolving collection: {e}"))?;
            let count = count_items(&client, acc, &coll_url).ok();
            let name = if r.displayname.is_empty() { r.href.clone() } else { r.displayname };
            out.push(DavCollection { name, href: r.href, count });
        }
    }
    Ok(out)
}

/// Count items in a collection via a Depth:1 PROPFIND (items carry a content
/// type / .ics|.vcf href; the collection itself does not).
fn count_items(client: &Client, acc: &DavAccount, url: &Url) -> Result<u32, String> {
    let body = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:getcontenttype/></d:prop></d:propfind>"#;
    let (xml, _) = propfind(client, acc, url, "1", body)?;
    let n = parse_responses(&xml)
        .iter()
        .filter(|r| {
            r.contenttype.contains("calendar")
                || r.contenttype.contains("vcard")
                || r.href.ends_with(".ics")
                || r.href.ends_with(".vcf")
        })
        .count();
    Ok(n as u32)
}

// --- XML helpers (namespace-prefix-agnostic) -------------------------------

fn local_name(qname: &[u8]) -> String {
    let s = String::from_utf8_lossy(qname);
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.to_string(),
    }
}

/// Return the first `<href>` text found inside the first `<target>` element.
fn first_href_in(xml: &str, target: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut in_target = false;
    let mut capture = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let ln = local_name(e.name().as_ref());
                if !in_target && ln == target {
                    in_target = true;
                } else if in_target && ln == "href" {
                    capture = true;
                }
            }
            Ok(Event::Text(t)) if capture => {
                return Some(t.unescape().map(|c| c.trim().to_string()).unwrap_or_default());
            }
            Ok(Event::End(e)) => {
                let ln = local_name(e.name().as_ref());
                if ln == "href" {
                    capture = false;
                } else if ln == target && in_target {
                    return None;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    None
}

/// Parse a multistatus document into per-`<response>` entries. Assumes each
/// response's own `<href>` is a direct child of `<response>` (true for the
/// collection listings and item listings we run this on).
fn parse_responses(xml: &str) -> Vec<ResponseEntry> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut cur: Option<ResponseEntry> = None;
    let mut stack: Vec<String> = Vec::new();
    let mut text_target: Option<&'static str> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let ln = local_name(e.name().as_ref());
                let parent = stack.last().cloned().unwrap_or_default();
                match ln.as_str() {
                    "response" => {
                        cur = Some(ResponseEntry {
                            href: String::new(),
                            displayname: String::new(),
                            resourcetypes: Vec::new(),
                            contenttype: String::new(),
                        });
                    }
                    "href" if parent == "response" => text_target = Some("href"),
                    "displayname" => text_target = Some("displayname"),
                    "getcontenttype" => text_target = Some("contenttype"),
                    _ => {}
                }
                if parent == "resourcetype" && ln != "resourcetype" {
                    if let Some(c) = cur.as_mut() {
                        c.resourcetypes.push(ln.clone());
                    }
                }
                stack.push(ln);
            }
            Ok(Event::Empty(e)) => {
                let ln = local_name(e.name().as_ref());
                let parent = stack.last().cloned().unwrap_or_default();
                if parent == "resourcetype" {
                    if let Some(c) = cur.as_mut() {
                        c.resourcetypes.push(ln);
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(tt) = text_target {
                    let val = t.unescape().map(|c| c.trim().to_string()).unwrap_or_default();
                    if let Some(c) = cur.as_mut() {
                        match tt {
                            "href" if c.href.is_empty() => c.href = val,
                            "displayname" if c.displayname.is_empty() => c.displayname = val,
                            "contenttype" if c.contenttype.is_empty() => c.contenttype = val,
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                let ln = local_name(e.name().as_ref());
                text_target = None;
                if ln == "response" {
                    if let Some(c) = cur.take() {
                        out.push(c);
                    }
                }
                stack.pop();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}
