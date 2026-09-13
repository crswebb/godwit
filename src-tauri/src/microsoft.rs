//! Microsoft 365 connector: OAuth 2.0 Authorization Code + PKCE sign-in (public
//! desktop client, loopback redirect) and Microsoft Graph discovery.
//!
//! Basic Auth is disabled by Microsoft, so this is the only way into an M365
//! mailbox. Tokens are kept in memory for the session (keychain persistence is
//! a planned follow-up). See docs/microsoft-setup.md for app registration.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const AUTH_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const GRAPH: &str = "https://graph.microsoft.com/v1.0";
const SCOPES: &str = "offline_access openid profile email User.Read Mail.ReadWrite Calendars.ReadWrite Contacts.ReadWrite";

struct SessionToken {
    client_id: String,
    access: String,
    refresh: Option<String>,
    expires_at: Instant,
}

fn store() -> &'static Mutex<HashMap<String, SessionToken>> {
    static STORE: OnceLock<Mutex<HashMap<String, SessionToken>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

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

// --- PKCE ------------------------------------------------------------------

fn pkce() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn random_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

// --- Loopback capture ------------------------------------------------------

/// Wait for the OAuth redirect on the loopback listener and return the `code`,
/// after checking `state`.
fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String, String> {
    let (mut stream, _) = listener.accept().map_err(|e| format!("loopback accept: {e}"))?;
    let request_line = {
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| format!("reading redirect: {e}"))?;
        line
    };

    // request_line looks like: GET /?code=...&state=... HTTP/1.1
    let path = request_line.split_whitespace().nth(1).unwrap_or("");
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            match k {
                "code" => code = Some(urldecode(v)),
                "state" => state = Some(urldecode(v)),
                _ => {}
            }
        }
    }

    let body = "<!doctype html><html><body style=\"font-family:sans-serif;padding:40px\">\
                <h2>Signed in to Godwit</h2><p>You can close this tab and return to the app.</p></body></html>";
    let _ = stream.write_all(
        format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}", body.len(), body).as_bytes(),
    );

    if state.as_deref() != Some(expected_state) {
        return Err("OAuth state mismatch (possible tampering).".into());
    }
    code.ok_or_else(|| "No authorization code returned.".into())
}

fn urldecode(s: &str) -> String {
    let s = s.replace('+', " ");
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// --- Token exchange / refresh ----------------------------------------------

struct TokenResponse {
    access: String,
    refresh: Option<String>,
    expires_in: u64,
}

fn parse_token_response(v: &Value) -> Result<TokenResponse, String> {
    if let Some(err) = v.get("error_description").and_then(Value::as_str) {
        return Err(err.to_string());
    }
    let access = v.get("access_token").and_then(Value::as_str).ok_or("no access_token in response")?.to_string();
    let refresh = v.get("refresh_token").and_then(Value::as_str).map(str::to_string);
    let expires_in = v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600);
    Ok(TokenResponse { access, refresh, expires_in })
}

fn exchange_code(client_id: &str, code: &str, verifier: &str, redirect: &str) -> Result<TokenResponse, String> {
    let client = reqwest::blocking::Client::new();
    let params = [
        ("client_id", client_id),
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect),
        ("code_verifier", verifier),
        ("scope", SCOPES),
    ];
    let resp = client.post(TOKEN_URL).form(&params).send().map_err(|e| format!("token request: {e}"))?;
    let v: Value = resp.json().map_err(|e| format!("token response parse: {e}"))?;
    parse_token_response(&v)
}

fn refresh_token(client_id: &str, refresh: &str) -> Result<TokenResponse, String> {
    let client = reqwest::blocking::Client::new();
    let params = [
        ("client_id", client_id),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("scope", SCOPES),
    ];
    let resp = client.post(TOKEN_URL).form(&params).send().map_err(|e| format!("refresh request: {e}"))?;
    let v: Value = resp.json().map_err(|e| format!("refresh response parse: {e}"))?;
    parse_token_response(&v)
}

/// Return a currently-valid access token for the account, refreshing if needed.
fn valid_access(email: &str) -> Result<String, String> {
    let guard = store().lock().unwrap();
    let session = guard.get(email).ok_or("Not signed in to this Microsoft account.")?;
    if session.expires_at > Instant::now() {
        return Ok(session.access.clone());
    }
    let refresh = session.refresh.clone().ok_or("Session expired and no refresh token.")?;
    let client_id = session.client_id.clone();
    drop(guard);

    let tr = refresh_token(&client_id, &refresh)?;
    let mut guard = store().lock().unwrap();
    let session = guard.get_mut(email).ok_or("Session vanished during refresh.")?;
    session.access = tr.access.clone();
    if tr.refresh.is_some() {
        session.refresh = tr.refresh;
    }
    session.expires_at = Instant::now() + Duration::from_secs(tr.expires_in.saturating_sub(60));
    Ok(tr.access)
}

// --- Public: sign-in + probe ----------------------------------------------

pub fn sign_in(client_id: &str) -> Result<MsAccount, String> {
    if client_id.trim().is_empty() {
        return Err("Enter the Microsoft app client ID (see docs/microsoft-setup.md).".into());
    }
    let (verifier, challenge) = pkce();
    let state = random_state();

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("loopback bind: {e}"))?;
    let port = listener.local_addr().map_err(|e| format!("loopback addr: {e}"))?.port();
    let redirect = format!("http://localhost:{port}");

    let mut url = reqwest::Url::parse(AUTH_URL).map_err(|e| format!("auth url: {e}"))?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect)
        .append_pair("response_mode", "query")
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state);

    open::that(url.as_str()).map_err(|e| format!("couldn't open browser: {e}"))?;

    let code = wait_for_code(&listener, &state)?;
    let tr = exchange_code(client_id, &code, &verifier, &redirect)?;

    // Identify the account.
    let client = reqwest::blocking::Client::new();
    let me: Value = client
        .get(format!("{GRAPH}/me"))
        .bearer_auth(&tr.access)
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

    store().lock().unwrap().insert(
        email.clone(),
        SessionToken {
            client_id: client_id.to_string(),
            access: tr.access,
            refresh: tr.refresh,
            expires_at: Instant::now() + Duration::from_secs(tr.expires_in.saturating_sub(60)),
        },
    );

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
    // The default Contacts folder isn't always listed under contactFolders.
    contact_folders.insert(0, "Contacts".to_string());

    Ok(MsProbe { folders, calendars, contact_folders })
}
