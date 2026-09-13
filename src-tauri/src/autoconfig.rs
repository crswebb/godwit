//! IMAP server autodiscovery via Mozilla's ISPDB (the same database
//! Thunderbird uses). Given an email address, look up the provider's IMAP
//! hostname + port so the user doesn't have to know it.
//!
//! Best-effort: returns None if the domain isn't in the database, in which case
//! the UI asks the user for the server manually.

use quick_xml::events::Event;
use quick_xml::Reader;
use std::time::Duration;

pub struct ImapServer {
    pub host: String,
    pub port: u16,
}

fn local_name(qname: &[u8]) -> String {
    let s = String::from_utf8_lossy(qname);
    match s.rsplit_once(':') {
        Some((_, l)) => l.to_string(),
        None => s.to_string(),
    }
}

pub fn discover_imap(email: &str) -> Option<ImapServer> {
    let domain = email.split('@').nth(1).filter(|d| !d.is_empty())?;
    let url = format!("https://autoconfig.thunderbird.net/v1.1/{domain}");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;
    let resp = client.get(&url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let xml = resp.text().ok()?;
    parse_imap(&xml)
}

/// Extract the first `<incomingServer type="imap">` host + port.
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
