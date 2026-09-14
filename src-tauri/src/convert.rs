//! Conversions between Microsoft Graph JSON resources and the canonical
//! interchange formats used everywhere else (vCard for contacts; iCalendar for
//! calendar comes later). Pure functions so they can be unit-tested without a
//! live Graph connection.

use serde_json::{json, Value};

// --- Contacts: Graph <-> vCard --------------------------------------------

fn escape_vcard(s: &str) -> String {
    s.replace('\\', "\\\\").replace(';', "\\;").replace(',', "\\,").replace('\n', "\\n")
}

fn unescape_vcard(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Convert a Graph `contact` resource to a vCard 3.0 string.
pub fn graph_contact_to_vcard(c: &Value) -> String {
    let mut lines = vec!["BEGIN:VCARD".to_string(), "VERSION:3.0".to_string()];

    let given = str_field(c, "givenName").unwrap_or_default();
    let surname = str_field(c, "surname").unwrap_or_default();
    let display = str_field(c, "displayName")
        .unwrap_or_else(|| format!("{given} {surname}").trim().to_string());

    lines.push(format!("FN:{}", escape_vcard(&display)));
    lines.push(format!("N:{};{};;;", escape_vcard(&surname), escape_vcard(&given)));

    if let Some(emails) = c.get("emailAddresses").and_then(Value::as_array) {
        for e in emails {
            if let Some(addr) = str_field(e, "address") {
                lines.push(format!("EMAIL;TYPE=INTERNET:{}", escape_vcard(&addr)));
            }
        }
    }
    if let Some(m) = str_field(c, "mobilePhone") {
        lines.push(format!("TEL;TYPE=CELL:{}", escape_vcard(&m)));
    }
    for (key, typ) in [("businessPhones", "WORK"), ("homePhones", "HOME")] {
        if let Some(arr) = c.get(key).and_then(Value::as_array) {
            for p in arr {
                if let Some(num) = p.as_str().filter(|s| !s.is_empty()) {
                    lines.push(format!("TEL;TYPE={typ}:{}", escape_vcard(num)));
                }
            }
        }
    }
    if let Some(org) = str_field(c, "companyName") {
        lines.push(format!("ORG:{}", escape_vcard(&org)));
    }
    if let Some(title) = str_field(c, "jobTitle") {
        lines.push(format!("TITLE:{}", escape_vcard(&title)));
    }
    if let Some(note) = str_field(c, "personalNotes") {
        lines.push(format!("NOTE:{}", escape_vcard(&note)));
    }
    if let Some(id) = str_field(c, "id") {
        lines.push(format!("UID:{}", escape_vcard(&id)));
    }

    lines.push("END:VCARD".to_string());
    lines.join("\r\n")
}

/// Parse a vCard into (property-name, params, value) tuples, unfolding
/// continuation lines.
fn parse_vcard_lines(vcard: &str) -> Vec<(String, String, String)> {
    // Unfold: a line beginning with space or tab continues the previous one.
    let mut logical: Vec<String> = Vec::new();
    for raw in vcard.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if (line.starts_with(' ') || line.starts_with('\t')) && !logical.is_empty() {
            logical.last_mut().unwrap().push_str(&line[1..]);
        } else {
            logical.push(line.to_string());
        }
    }
    let mut out = Vec::new();
    for line in logical {
        if let Some((head, value)) = line.split_once(':') {
            let (name, params) = match head.split_once(';') {
                Some((n, p)) => (n.to_string(), p.to_string()),
                None => (head.to_string(), String::new()),
            };
            out.push((name.to_uppercase(), params.to_uppercase(), value.to_string()));
        }
    }
    out
}

/// Convert a vCard string to a Graph `contact` resource.
pub fn vcard_to_graph_contact(vcard: &str) -> Value {
    let mut given = String::new();
    let mut surname = String::new();
    let mut display = String::new();
    let mut emails: Vec<Value> = Vec::new();
    let mut business: Vec<String> = Vec::new();
    let mut home: Vec<String> = Vec::new();
    let mut mobile: Option<String> = None;
    let mut company: Option<String> = None;
    let mut title: Option<String> = None;
    let mut note: Option<String> = None;

    for (name, params, value) in parse_vcard_lines(vcard) {
        let value = unescape_vcard(&value);
        match name.as_str() {
            "FN" => display = value,
            "N" => {
                let parts: Vec<&str> = value.split(';').collect();
                surname = parts.first().map(|s| unescape_vcard(s)).unwrap_or_default();
                given = parts.get(1).map(|s| unescape_vcard(s)).unwrap_or_default();
            }
            "EMAIL" => {
                if !value.is_empty() {
                    emails.push(json!({ "address": value, "name": value }));
                }
            }
            "TEL" => {
                if params.contains("CELL") || params.contains("MOBILE") {
                    mobile = Some(value);
                } else if params.contains("HOME") {
                    home.push(value);
                } else {
                    business.push(value);
                }
            }
            "ORG" => company = Some(value.split(';').next().unwrap_or(&value).to_string()),
            "TITLE" => title = Some(value),
            "NOTE" => note = Some(value),
            _ => {}
        }
    }

    if display.is_empty() {
        display = format!("{given} {surname}").trim().to_string();
    }

    let mut obj = json!({ "displayName": display });
    let map = obj.as_object_mut().unwrap();
    if !given.is_empty() { map.insert("givenName".into(), json!(given)); }
    if !surname.is_empty() { map.insert("surname".into(), json!(surname)); }
    if !emails.is_empty() { map.insert("emailAddresses".into(), json!(emails)); }
    if !business.is_empty() { map.insert("businessPhones".into(), json!(business)); }
    if !home.is_empty() { map.insert("homePhones".into(), json!(home)); }
    if let Some(m) = mobile { map.insert("mobilePhone".into(), json!(m)); }
    if let Some(c) = company { map.insert("companyName".into(), json!(c)); }
    if let Some(t) = title { map.insert("jobTitle".into(), json!(t)); }
    if let Some(n) = note { map.insert("personalNotes".into(), json!(n)); }
    obj
}

/// A stable dedup key for a contact vCard: primary email (lowercased), else FN.
pub fn contact_key(vcard: &str) -> String {
    let mut fn_val = String::new();
    for (name, _params, value) in parse_vcard_lines(vcard) {
        if name == "EMAIL" && !value.is_empty() {
            return unescape_vcard(&value).trim().to_lowercase();
        }
        if name == "FN" && fn_val.is_empty() {
            fn_val = unescape_vcard(&value).trim().to_lowercase();
        }
    }
    fn_val
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn graph_to_vcard_has_core_fields() {
        let c = json!({
            "id": "AAA123",
            "displayName": "Ada Lovelace",
            "givenName": "Ada",
            "surname": "Lovelace",
            "emailAddresses": [{ "address": "ada@example.com" }],
            "mobilePhone": "+4670",
            "businessPhones": ["+468"],
            "companyName": "Analytical",
            "jobTitle": "Mathematician"
        });
        let v = graph_contact_to_vcard(&c);
        assert!(v.contains("FN:Ada Lovelace"));
        assert!(v.contains("N:Lovelace;Ada;;;"));
        assert!(v.contains("EMAIL;TYPE=INTERNET:ada@example.com"));
        assert!(v.contains("TEL;TYPE=CELL:+4670"));
        assert!(v.contains("TEL;TYPE=WORK:+468"));
        assert!(v.contains("ORG:Analytical"));
        assert!(v.contains("UID:AAA123"));
    }

    #[test]
    fn vcard_to_graph_maps_fields() {
        let v = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Grace Hopper\r\nN:Hopper;Grace;;;\r\nEMAIL;TYPE=INTERNET:grace@example.com\r\nTEL;TYPE=CELL:+1555\r\nORG:Navy\r\nTITLE:Rear Admiral\r\nEND:VCARD";
        let g = vcard_to_graph_contact(v);
        assert_eq!(g["displayName"], "Grace Hopper");
        assert_eq!(g["givenName"], "Grace");
        assert_eq!(g["surname"], "Hopper");
        assert_eq!(g["emailAddresses"][0]["address"], "grace@example.com");
        assert_eq!(g["mobilePhone"], "+1555");
        assert_eq!(g["companyName"], "Navy");
        assert_eq!(g["jobTitle"], "Rear Admiral");
    }

    #[test]
    fn round_trip_preserves_key_fields() {
        let c = json!({
            "displayName": "Alan Turing",
            "givenName": "Alan",
            "surname": "Turing",
            "emailAddresses": [{ "address": "alan@example.com" }]
        });
        let g2 = vcard_to_graph_contact(&graph_contact_to_vcard(&c));
        assert_eq!(g2["displayName"], "Alan Turing");
        assert_eq!(g2["emailAddresses"][0]["address"], "alan@example.com");
    }

    #[test]
    fn contact_key_prefers_email() {
        let v = "BEGIN:VCARD\nFN:Bob\nEMAIL:BOB@Example.com\nEND:VCARD";
        assert_eq!(contact_key(v), "bob@example.com");
        let v2 = "BEGIN:VCARD\nFN:Just A Name\nEND:VCARD";
        assert_eq!(contact_key(v2), "just a name");
    }
}
