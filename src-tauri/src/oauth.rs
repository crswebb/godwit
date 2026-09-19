//! Shared OAuth 2.0 Authorization Code + PKCE flow for public desktop clients:
//! opens the system browser, captures the redirect on a loopback listener,
//! exchanges the code, and keeps tokens in memory with refresh. Every OAuth
//! provider connector (Microsoft, Google, …) uses this.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Per-provider endpoints + scopes.
pub struct Config {
    pub auth_url: &'static str,
    pub token_url: &'static str,
    pub scopes: &'static str,
    /// Extra authorize-URL params (e.g. Google's access_type=offline, prompt=consent).
    pub extra_auth: &'static [(&'static str, &'static str)],
}

pub struct Tokens {
    pub access: String,
    pub refresh: Option<String>,
    pub expires_in: u64,
}

struct Session {
    client_id: String,
    token_url: &'static str,
    scopes: &'static str,
    access: String,
    refresh: Option<String>,
    expires_at: Instant,
}

fn store() -> &'static Mutex<HashMap<String, Session>> {
    static STORE: OnceLock<Mutex<HashMap<String, Session>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

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

fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String, String> {
    let (mut stream, _) = listener.accept().map_err(|e| format!("loopback accept: {e}"))?;
    let request_line = {
        let mut reader = BufReader::new(&stream);
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| format!("reading redirect: {e}"))?;
        line
    };

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

fn parse_tokens(v: &Value) -> Result<Tokens, String> {
    if let Some(err) = v.get("error_description").and_then(Value::as_str) {
        return Err(err.to_string());
    }
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        return Err(err.to_string());
    }
    let access = v.get("access_token").and_then(Value::as_str).ok_or("no access_token in response")?.to_string();
    let refresh = v.get("refresh_token").and_then(Value::as_str).map(str::to_string);
    let expires_in = v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600);
    Ok(Tokens { access, refresh, expires_in })
}

fn post_token(token_url: &str, params: &[(&str, &str)]) -> Result<Tokens, String> {
    let resp = reqwest::blocking::Client::new()
        .post(token_url)
        .form(params)
        .send()
        .map_err(|e| format!("token request: {e}"))?;
    let v: Value = resp.json().map_err(|e| format!("token response parse: {e}"))?;
    parse_tokens(&v)
}

/// Run the interactive PKCE flow and return the tokens. The caller then fetches
/// the account identity and calls `remember`.
pub fn authorize(cfg: &Config, client_id: &str) -> Result<Tokens, String> {
    if client_id.trim().is_empty() {
        return Err("Missing OAuth client ID.".into());
    }
    let (verifier, challenge) = pkce();
    let state = random_state();

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("loopback bind: {e}"))?;
    let port = listener.local_addr().map_err(|e| format!("loopback addr: {e}"))?.port();
    let redirect = format!("http://localhost:{port}");

    let mut url = reqwest::Url::parse(cfg.auth_url).map_err(|e| format!("auth url: {e}"))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect)
            .append_pair("scope", cfg.scopes)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
        for (k, v) in cfg.extra_auth {
            q.append_pair(k, v);
        }
    }

    open::that(url.as_str()).map_err(|e| format!("couldn't open browser: {e}"))?;

    let code = wait_for_code(&listener, &state)?;
    let params = [
        ("client_id", client_id),
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("code_verifier", verifier.as_str()),
        ("scope", cfg.scopes),
    ];
    post_token(cfg.token_url, &params)
}

/// Store the tokens for later use, keyed by an app-chosen id (e.g. "google:me@x").
pub fn remember(key: String, client_id: &str, cfg: &Config, t: Tokens) {
    store().lock().unwrap().insert(
        key,
        Session {
            client_id: client_id.to_string(),
            token_url: cfg.token_url,
            scopes: cfg.scopes,
            access: t.access,
            refresh: t.refresh,
            expires_at: Instant::now() + Duration::from_secs(t.expires_in.saturating_sub(60)),
        },
    );
}

/// A currently-valid access token for `key`, refreshing if it has expired.
pub fn valid_access(key: &str) -> Result<String, String> {
    let guard = store().lock().unwrap();
    let s = guard.get(key).ok_or("Not signed in — sign in again.")?;
    if s.expires_at > Instant::now() {
        return Ok(s.access.clone());
    }
    let refresh = s.refresh.clone().ok_or("Session expired; sign in again.")?;
    let (client_id, token_url, scopes) = (s.client_id.clone(), s.token_url, s.scopes);
    drop(guard);

    let params = [
        ("client_id", client_id.as_str()),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh.as_str()),
        ("scope", scopes),
    ];
    let t = post_token(token_url, &params)?;

    let mut guard = store().lock().unwrap();
    let s = guard.get_mut(key).ok_or("Session vanished during refresh.")?;
    s.access = t.access.clone();
    if t.refresh.is_some() {
        s.refresh = t.refresh;
    }
    s.expires_at = Instant::now() + Duration::from_secs(t.expires_in.saturating_sub(60));
    Ok(t.access)
}
