//! Google connector: OAuth sign-in (shared `oauth`), Gmail over IMAP + XOAUTH2,
//! and Google CalDAV/CardDAV (OAuth bearer). Reuses the IMAP and DAV engine
//! adapters in `imapdav`, so this file is mostly wiring.
//!
//! Gmail read/write is a Google "restricted scope": public distribution needs a
//! CASA security assessment, but OAuth "testing" mode covers up to 100 users
//! with no assessment. See docs/google-setup.md.

use super::imapdav;
use super::{Connector, Kind};
use crate::domain::{Account, DavProbe, FolderInfo, ImapProbe, Probe};
use crate::engine::{Reader, Writer};
use crate::{dav, oauth};

use serde::Serialize;
use serde_json::Value;

const CONFIG: oauth::Config = oauth::Config {
    auth_url: "https://accounts.google.com/o/oauth2/v2/auth",
    token_url: "https://oauth2.googleapis.com/token",
    scopes: "openid email https://mail.google.com/ https://www.googleapis.com/auth/calendar https://www.googleapis.com/auth/carddav",
    // access_type=offline + prompt=consent are how Google returns a refresh token.
    extra_auth: &[("access_type", "offline"), ("prompt", "consent")],
};

const GMAIL_HOST: &str = "imap.gmail.com";
const GMAIL_PORT: u16 = 993;
const CALDAV_BASE: &str = "https://apidata.googleusercontent.com/caldav/v2/";

fn carddav_base(email: &str) -> String {
    format!("https://www.googleapis.com/carddav/v1/principals/{email}/")
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleAccount {
    pub email: String,
}

pub fn sign_in(client_id: &str) -> Result<GoogleAccount, String> {
    let tokens = oauth::authorize(&CONFIG, client_id)?;
    let info: Value = reqwest::blocking::Client::new()
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(&tokens.access)
        .send()
        .map_err(|e| format!("userinfo request: {e}"))?
        .json()
        .map_err(|e| format!("userinfo parse: {e}"))?;
    let email = info
        .get("email")
        .and_then(Value::as_str)
        .ok_or("Couldn't read the Google account email.")?
        .to_string();

    oauth::remember(format!("google:{email}"), client_id, &CONFIG, tokens);
    Ok(GoogleAccount { email })
}

fn valid_access(email: &str) -> Result<String, String> {
    oauth::valid_access(&format!("google:{email}"))
}

pub struct GoogleConnector {
    account: Account,
}

impl GoogleConnector {
    pub fn new(account: Account) -> Self {
        Self { account }
    }

    fn token(&self) -> Result<String, String> {
        valid_access(&self.account.email)
    }

    fn dav_account(&self, url: Option<String>, token: String) -> dav::DavAccount {
        dav::DavAccount {
            url,
            username: self.account.email.clone(),
            password: String::new(),
            bearer: Some(token),
        }
    }
}

fn probe_dav(conn: &GoogleConnector, kind: dav::DavKind, token: &str) -> DavProbe {
    let url = match kind {
        dav::DavKind::Calendar => CALDAV_BASE.to_string(),
        dav::DavKind::Contacts => carddav_base(&conn.account.email),
    };
    let acc = conn.dav_account(Some(url), token.to_string());
    match dav::discover(&acc, kind) {
        Ok(collections) => DavProbe { status: "ok".into(), collections, error: None },
        Err(e) => DavProbe { status: "unavailable".into(), collections: vec![], error: Some(e) },
    }
}

impl Connector for GoogleConnector {
    fn probe(&self) -> Probe {
        let token = match self.token() {
            Ok(t) => t,
            Err(e) => {
                return Probe {
                    imap: ImapProbe { status: "error".into(), error: Some(e.clone()), ..Default::default() },
                    calendars: DavProbe { status: "unavailable".into(), error: Some(e.clone()), ..Default::default() },
                    contacts: DavProbe { status: "unavailable".into(), error: Some(e), ..Default::default() },
                }
            }
        };

        let imap = match imapdav::xoauth2_folders(GMAIL_HOST, GMAIL_PORT, &self.account.email, &token) {
            Ok(folders) => ImapProbe {
                status: "ok".into(),
                host: Some(GMAIL_HOST.into()),
                port: Some(GMAIL_PORT),
                folders,
                error: None,
            },
            Err(e) => ImapProbe { status: "error".into(), error: Some(e), ..Default::default() },
        };

        Probe {
            imap,
            calendars: probe_dav(self, dav::DavKind::Calendar, &token),
            contacts: probe_dav(self, dav::DavKind::Contacts, &token),
        }
    }

    fn folders(&self) -> Result<Vec<FolderInfo>, String> {
        let token = self.token()?;
        imapdav::xoauth2_folders(GMAIL_HOST, GMAIL_PORT, &self.account.email, &token)
    }

    fn remaps_folders(&self) -> bool {
        true
    }

    fn reader(&self, kind: Kind) -> Result<Box<dyn Reader>, String> {
        let token = self.token()?;
        Ok(match kind {
            Kind::Mail => imapdav::xoauth2_reader(GMAIL_HOST, GMAIL_PORT, &self.account.email, &token)?,
            Kind::Calendar => imapdav::dav_reader(self.dav_account(None, token), dav::DavKind::Calendar),
            Kind::Contacts => imapdav::dav_reader(self.dav_account(None, token), dav::DavKind::Contacts),
        })
    }

    fn writer(&self, kind: Kind) -> Result<Box<dyn Writer>, String> {
        let token = self.token()?;
        Ok(match kind {
            Kind::Mail => imapdav::xoauth2_writer(GMAIL_HOST, GMAIL_PORT, &self.account.email, &token)?,
            Kind::Calendar => imapdav::dav_writer(self.dav_account(None, token), dav::DavKind::Calendar),
            Kind::Contacts => imapdav::dav_writer(self.dav_account(None, token), dav::DavKind::Contacts),
        })
    }
}
