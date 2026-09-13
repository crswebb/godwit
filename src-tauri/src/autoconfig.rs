//! IMAP server autodiscovery. Tries, in order:
//!   1. Mozilla autoconfig served by the domain itself
//!      (`autoconfig.<domain>` and `<domain>/.well-known/autoconfig`)
//!   2. DNS SRV records (RFC 6186: `_imaps._tcp` / `_imap._tcp`)
//!   3. Mozilla's central ISPDB (covers the big public providers)
//!
//! Best-effort: returns None if nothing resolves, in which case the UI asks the
//! user for the server.

use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::Resolver;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::time::Duration;

pub struct ImapServer {
    pub host: String,
    pub port: u16,
}

pub fn discover_imap(email: &str) -> Option<ImapServer> {
    let domain = email.split('@').nth(1).filter(|d| !d.is_empty())?;
    let encoded = email.replace('@', "%40");

    // 1. Autoconfig hosted by the domain (authoritative for custom domains).
    let hosted = [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={encoded}"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml?emailaddress={encoded}"),
    ];
    for url in hosted {
        if let Some(s) = fetch_autoconfig(&url) {
            return Some(s);
        }
    }

    // 2. DNS SRV.
    if let Some(s) = srv_lookup(domain) {
        return Some(s);
    }

    // 3. Central ISPDB.
    fetch_autoconfig(&format!("https://autoconfig.thunderbird.net/v1.1/{domain}"))
}

fn fetch_autoconfig(url: &str) -> Option<ImapServer> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?;
    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    parse_imap(&resp.text().ok()?)
}

fn srv_lookup(domain: &str) -> Option<ImapServer> {
    let resolver = Resolver::new(ResolverConfig::default(), ResolverOpts::default()).ok()?;
    // Prefer implicit TLS (_imaps) over STARTTLS (_imap).
    for service in ["_imaps._tcp", "_imap._tcp"] {
        let query = format!("{service}.{domain}.");
        if let Ok(lookup) = resolver.srv_lookup(query) {
            // Lowest priority wins; a target of "." means "not provided".
            let best = lookup
                .iter()
                .filter(|r| {
                    let t = r.target().to_utf8();
                    t != "." && !t.is_empty()
                })
                .min_by_key(|r| r.priority());
            if let Some(r) = best {
                let mut host = r.target().to_utf8();
                if host.ends_with('.') {
                    host.pop();
                }
                return Some(ImapServer { host, port: r.port() });
            }
        }
    }
    None
}

fn local_name(qname: &[u8]) -> String {
    let s = String::from_utf8_lossy(qname);
    match s.rsplit_once(':') {
        Some((_, l)) => l.to_string(),
        None => s.to_string(),
    }
}

/// Extract the first `<incomingServer type="imap">` host + port from a Mozilla
/// autoconfig / ISPDB document.
fn parse_imap(xml: &str) -> Option<ImapServer> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut in_imap = false;
    let mut field: Option<&'static str> = None;
    let mut host = String::new();
    let mut port: u16 = 993;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let ln = local_name(e.name().as_ref());
                if ln == "incomingServer" {
                    in_imap = e
                        .attributes()
                        .flatten()
                        .any(|a| a.key.as_ref() == b"type" && a.value.as_ref() == b"imap");
                } else if in_imap && ln == "hostname" {
                    field = Some("host");
                } else if in_imap && ln == "port" {
                    field = Some("port");
                }
            }
            Ok(Event::Text(t)) if field.is_some() => {
                let v = t.unescape().map(|c| c.trim().to_string()).unwrap_or_default();
                match field {
                    Some("host") => host = v,
                    Some("port") => {
                        if let Ok(p) = v.parse::<u16>() {
                            port = p;
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                field = None;
                if local_name(e.name().as_ref()) == "incomingServer" && in_imap {
                    if !host.is_empty() {
                        return Some(ImapServer { host, port });
                    }
                    in_imap = false;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    if host.is_empty() {
        None
    } else {
        Some(ImapServer { host, port })
    }
}
