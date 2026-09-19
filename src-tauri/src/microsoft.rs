//! Microsoft 365 Graph client + OAuth sign-in (via the shared `oauth` module).
//! Basic Auth is disabled by Microsoft, so OAuth is the only way into an M365
//! mailbox. See docs/microsoft-setup.md for app registration.

use crate::oauth;
use serde::Serialize;
use serde_json::Value;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";

const CONFIG: oauth::Config = oauth::Config {
    auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
    token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
    scopes: "offline_access openid profile email User.Read Mail.ReadWrite Calendars.ReadWrite Contacts.ReadWrite",
    extra_auth: &[],
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MsAccount {
    pub email: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MsFolder {
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MsProbe {
    pub folders: Vec<MsFolder>,
    pub calendars: Vec<String>,
    pub contact_folders: Vec<String>,
}

pub fn valid_access(email: &str) -> Result<String, String> {
    oauth::valid_access(&format!("microsoft:{email}"))
}

pub fn sign_in(client_id: &str) -> Result<MsAccount, String> {
    let tokens = oauth::authorize(&CONFIG, client_id)?;
    let me: Value = reqwest::blocking::Client::new()
        .get(format!("{GRAPH}/me"))
        .bearer_auth(&tokens.access)
        .send()
        .map_err(|e| format!("/me request: {e}"))?
        .json()
        .map_err(|e| format!("/me parse: {e}"))?;
    let email = me
        .get("mail")
        .and_then(Value::as_str)
        .or_else(|| me.get("userPrincipalName").and_then(Value::as_str))
        .ok_or("Couldn't read the account address from Microsoft Graph.")?
        .to_string();

    oauth::remember(format!("microsoft:{email}"), client_id, &CONFIG, tokens);
    Ok(MsAccount { email })
}

fn graph_get(access: &str, url: &str) -> Result<Value, String> {
    let client = reqwest::blocking::Client::new();
    let resp = client.get(url).bearer_auth(access).send().map_err(|e| format!("Graph GET {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Graph {url} -> HTTP {}", resp.status().as_u16()));
    }
    resp.json().map_err(|e| format!("Graph parse {url}: {e}"))
}

pub fn probe(email: &str) -> Result<MsProbe, String> {
    let access = valid_access(email)?;

    let mail = graph_get(&access, &format!("{GRAPH}/me/mailFolders?$top=100"))?;
    let folders = mail
        .get("value")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|f| MsFolder {
                    name: f.get("displayName").and_then(Value::as_str).unwrap_or("").to_string(),
                    count: f.get("totalItemCount").and_then(Value::as_u64).unwrap_or(0) as u32,
                })
                .collect()
        })
        .unwrap_or_default();

    let cal = graph_get(&access, &format!("{GRAPH}/me/calendars?$top=100"))?;
    let calendars = cal
        .get("value")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(|c| c.get("name").and_then(Value::as_str)).map(str::to_string).collect())
        .unwrap_or_default();

    let cf = graph_get(&access, &format!("{GRAPH}/me/contactFolders?$top=100"))?;
    let mut contact_folders: Vec<String> = cf
        .get("value")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(|c| c.get("displayName").and_then(Value::as_str)).map(str::to_string).collect())
        .unwrap_or_default();
    contact_folders.insert(0, "Contacts".to_string());

    Ok(MsProbe { folders, calendars, contact_folders })
}

/// List all contacts in the account's default contact folder (Graph JSON).
pub fn list_contacts(email: &str) -> Result<Vec<Value>, String> {
    let access = valid_access(email)?;
    let client = reqwest::blocking::Client::new();
    let mut url = format!("{GRAPH}/me/contacts?$top=100");
    let mut out = Vec::new();
    loop {
        let resp = client
            .get(&url)
            .bearer_auth(&access)
            .send()
            .map_err(|e| format!("contacts GET: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("contacts -> HTTP {}", resp.status().as_u16()));
        }
        let v: Value = resp.json().map_err(|e| format!("contacts parse: {e}"))?;
        if let Some(arr) = v["value"].as_array() {
            out.extend(arr.iter().cloned());
        }
        match v["@odata.nextLink"].as_str() {
            Some(next) => url = next.to_string(),
            None => break,
        }
    }
    Ok(out)
}

/// Create a contact in the account's default contact folder.
pub fn create_contact(email: &str, contact: &Value) -> Result<(), String> {
    let access = valid_access(email)?;
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(format!("{GRAPH}/me/contacts"))
        .bearer_auth(&access)
        .json(contact)
        .send()
        .map_err(|e| format!("contact create: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("contact create -> HTTP {}", resp.status().as_u16()))
    }
}

/// List events in the account's default calendar, times normalised to UTC.
pub fn list_events(email: &str) -> Result<Vec<Value>, String> {
    let access = valid_access(email)?;
    let client = reqwest::blocking::Client::new();
    let select = "id,iCalUId,subject,body,start,end,isAllDay,location,recurrence,lastModifiedDateTime";
    let mut url = format!("{GRAPH}/me/events?$select={select}&$top=50");
    let mut out = Vec::new();
    loop {
        let resp = client
            .get(&url)
            .bearer_auth(&access)
            .header("Prefer", "outlook.timezone=\"UTC\"")
            .send()
            .map_err(|e| format!("events GET: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("events -> HTTP {}", resp.status().as_u16()));
        }
        let v: Value = resp.json().map_err(|e| format!("events parse: {e}"))?;
        if let Some(arr) = v["value"].as_array() {
            out.extend(arr.iter().cloned());
        }
        match v["@odata.nextLink"].as_str() {
            Some(next) => url = next.to_string(),
            None => break,
        }
    }
    Ok(out)
}

/// Create an event in the account's default calendar.
pub fn create_event(email: &str, event: &Value) -> Result<(), String> {
    let access = valid_access(email)?;
    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(format!("{GRAPH}/me/events"))
        .bearer_auth(&access)
        .json(event)
        .send()
        .map_err(|e| format!("event create: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("event create -> HTTP {}", resp.status().as_u16()))
    }
}
