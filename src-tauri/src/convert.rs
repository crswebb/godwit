//! Conversions between Microsoft Graph JSON resources and the canonical
//! interchange formats used everywhere else (vCard for contacts; iCalendar for
//! calendar comes later). Pure functions so they can be unit-tested without a
//! live Graph connection.

use serde_json::{json, Value};
use std::collections::HashMap;

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
            out.push((name.to_uppercase(), params, value.to_string()));
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
                let p = params.to_uppercase();
                if p.contains("CELL") || p.contains("MOBILE") {
                    mobile = Some(value);
                } else if p.contains("HOME") {
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

// --- Calendar: Graph <-> iCalendar ----------------------------------------
//
// Times are normalised to UTC on the way out of Graph (request with
// Prefer: outlook.timezone="UTC"), which avoids Windows<->IANA timezone
// mapping. v1 covers non-recurring and all-day events, and the common
// recurrence patterns (daily/weekly/monthly/yearly + interval + count/until +
// weekly BYDAY). UID is preserved to a DAV destination; Microsoft assigns its
// own iCalUId on create, so dedup into M365 is best-effort.

const GRAPH_DAYS: [(&str, &str); 7] = [
    ("sunday", "SU"), ("monday", "MO"), ("tuesday", "TU"), ("wednesday", "WE"),
    ("thursday", "TH"), ("friday", "FR"), ("saturday", "SA"),
];

fn graph_day_to_ics(d: &str) -> Option<&'static str> {
    GRAPH_DAYS.iter().find(|(g, _)| d.eq_ignore_ascii_case(g)).map(|(_, i)| *i)
}
fn ics_day_to_graph(d: &str) -> Option<&'static str> {
    let d = d.trim_start_matches(|c: char| c == '+' || c == '-' || c.is_ascii_digit());
    GRAPH_DAYS.iter().find(|(_, i)| d.eq_ignore_ascii_case(i)).map(|(g, _)| *g)
}

fn graph_dt_to_ics_utc(s: &str) -> String {
    if s.len() >= 19 {
        format!("{}{}{}T{}{}{}Z", &s[0..4], &s[5..7], &s[8..10], &s[11..13], &s[14..16], &s[17..19])
    } else {
        s.to_string()
    }
}
fn graph_date_to_ics(s: &str) -> String {
    if s.len() >= 10 { format!("{}{}{}", &s[0..4], &s[5..7], &s[8..10]) } else { s.to_string() }
}
fn ics_utc_to_graph(s: &str) -> String {
    let s = s.trim_end_matches('Z');
    if s.len() >= 15 {
        format!("{}-{}-{}T{}:{}:{}", &s[0..4], &s[4..6], &s[6..8], &s[9..11], &s[11..13], &s[13..15])
    } else {
        s.to_string()
    }
}
fn ics_local_to_graph(s: &str) -> String {
    // "YYYYMMDDTHHMMSS" (no zone)
    if s.len() >= 15 {
        format!("{}-{}-{}T{}:{}:{}", &s[0..4], &s[4..6], &s[6..8], &s[9..11], &s[11..13], &s[13..15])
    } else {
        s.to_string()
    }
}
fn ics_date_to_graph(s: &str) -> String {
    if s.len() >= 8 { format!("{}-{}-{}T00:00:00", &s[0..4], &s[4..6], &s[6..8]) } else { s.to_string() }
}

fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").trim().to_string()
}

fn relative_byday(pat: &Value) -> Option<String> {
    let idx = pat.get("index").and_then(Value::as_str).unwrap_or("first");
    let n = match idx {
        "first" => "1", "second" => "2", "third" => "3", "fourth" => "4", "last" => "-1", _ => "1",
    };
    let day = pat.get("daysOfWeek")?.as_array()?.iter()
        .filter_map(|x| x.as_str()).filter_map(graph_day_to_ics).next()?;
    Some(format!("{n}{day}"))
}

fn graph_recurrence_to_rrule(rec: &Value) -> Option<String> {
    let pat = rec.get("pattern")?;
    let typ = pat.get("type")?.as_str()?;
    let interval = pat.get("interval").and_then(Value::as_u64).unwrap_or(1);
    let mut parts = Vec::new();
    match typ {
        "daily" => parts.push("FREQ=DAILY".to_string()),
        "weekly" => {
            parts.push("FREQ=WEEKLY".to_string());
            if let Some(days) = pat.get("daysOfWeek").and_then(Value::as_array) {
                let d: Vec<&str> = days.iter().filter_map(|x| x.as_str()).filter_map(graph_day_to_ics).collect();
                if !d.is_empty() { parts.push(format!("BYDAY={}", d.join(","))); }
            }
        }
        "absoluteMonthly" => {
            parts.push("FREQ=MONTHLY".to_string());
            if let Some(d) = pat.get("dayOfMonth").and_then(Value::as_u64) { parts.push(format!("BYMONTHDAY={d}")); }
        }
        "relativeMonthly" => {
            parts.push("FREQ=MONTHLY".to_string());
            if let Some(bd) = relative_byday(pat) { parts.push(format!("BYDAY={bd}")); }
        }
        "absoluteYearly" => {
            parts.push("FREQ=YEARLY".to_string());
            if let Some(m) = pat.get("month").and_then(Value::as_u64) { parts.push(format!("BYMONTH={m}")); }
            if let Some(d) = pat.get("dayOfMonth").and_then(Value::as_u64) { parts.push(format!("BYMONTHDAY={d}")); }
        }
        "relativeYearly" => {
            parts.push("FREQ=YEARLY".to_string());
            if let Some(m) = pat.get("month").and_then(Value::as_u64) { parts.push(format!("BYMONTH={m}")); }
            if let Some(bd) = relative_byday(pat) { parts.push(format!("BYDAY={bd}")); }
        }
        _ => return None,
    }
    if interval > 1 { parts.push(format!("INTERVAL={interval}")); }
    if let Some(range) = rec.get("range") {
        match range.get("type").and_then(Value::as_str) {
            Some("numbered") => {
                if let Some(n) = range.get("numberOfOccurrences").and_then(Value::as_u64) {
                    if n > 0 { parts.push(format!("COUNT={n}")); }
                }
            }
            Some("endDate") => {
                if let Some(ed) = range.get("endDate").and_then(Value::as_str) {
                    parts.push(format!("UNTIL={}", graph_date_to_ics(ed)));
                }
            }
            _ => {}
        }
    }
    Some(parts.join(";"))
}

fn rrule_to_graph_recurrence(rrule: &str, start_date: &str) -> Option<Value> {
    let mut map: HashMap<String, String> = HashMap::new();
    for kv in rrule.split(';') {
        if let Some((k, v)) = kv.split_once('=') {
            map.insert(k.trim().to_uppercase(), v.trim().to_string());
        }
    }
    let freq = map.get("FREQ")?.to_uppercase();
    let interval = map.get("INTERVAL").and_then(|s| s.parse::<u64>().ok()).unwrap_or(1);
    let mut pattern = serde_json::Map::new();
    pattern.insert("interval".into(), json!(interval));
    match freq.as_str() {
        "DAILY" => { pattern.insert("type".into(), json!("daily")); }
        "WEEKLY" => {
            pattern.insert("type".into(), json!("weekly"));
            if let Some(bd) = map.get("BYDAY") {
                let days: Vec<Value> = bd.split(',').filter_map(ics_day_to_graph).map(|g| json!(g)).collect();
                if !days.is_empty() { pattern.insert("daysOfWeek".into(), json!(days)); }
            }
        }
        "MONTHLY" => {
            pattern.insert("type".into(), json!("absoluteMonthly"));
            let dom = map.get("BYMONTHDAY").and_then(|s| s.parse::<u64>().ok()).unwrap_or(1);
            pattern.insert("dayOfMonth".into(), json!(dom));
        }
        "YEARLY" => {
            pattern.insert("type".into(), json!("absoluteYearly"));
            if let Some(m) = map.get("BYMONTH").and_then(|s| s.parse::<u64>().ok()) { pattern.insert("month".into(), json!(m)); }
            let dom = map.get("BYMONTHDAY").and_then(|s| s.parse::<u64>().ok()).unwrap_or(1);
            pattern.insert("dayOfMonth".into(), json!(dom));
        }
        _ => return None,
    }
    let mut range = serde_json::Map::new();
    range.insert("startDate".into(), json!(start_date));
    if let Some(c) = map.get("COUNT").and_then(|s| s.parse::<u64>().ok()) {
        range.insert("type".into(), json!("numbered"));
        range.insert("numberOfOccurrences".into(), json!(c));
    } else if let Some(u) = map.get("UNTIL") {
        range.insert("type".into(), json!("endDate"));
        let d = &u[..u.len().min(8)];
        range.insert("endDate".into(), json!(format!("{}-{}-{}", &d[0..4], &d[4..6], &d[6..8])));
    } else {
        range.insert("type".into(), json!("noEnd"));
    }
    Some(json!({ "pattern": Value::Object(pattern), "range": Value::Object(range) }))
}

/// Convert a Graph `event` resource to an iCalendar (VCALENDAR/VEVENT) string.
pub fn graph_event_to_ics(e: &Value) -> String {
    let mut v = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Godwit//EN".to_string(),
        "BEGIN:VEVENT".to_string(),
    ];
    let uid = str_field(e, "iCalUId").or_else(|| str_field(e, "id")).unwrap_or_default();
    v.push(format!("UID:{}", escape_vcard(&uid)));

    let stamp = str_field(e, "lastModifiedDateTime")
        .or_else(|| e.get("start").and_then(|s| str_field(s, "dateTime")))
        .map(|s| graph_dt_to_ics_utc(&s))
        .unwrap_or_default();
    if !stamp.is_empty() { v.push(format!("DTSTAMP:{stamp}")); }

    if let Some(s) = str_field(e, "subject") { v.push(format!("SUMMARY:{}", escape_vcard(&s))); }

    let all_day = e.get("isAllDay").and_then(Value::as_bool).unwrap_or(false);
    let start_dt = e.get("start").and_then(|s| str_field(s, "dateTime"));
    let end_dt = e.get("end").and_then(|s| str_field(s, "dateTime"));
    if all_day {
        if let Some(s) = &start_dt { v.push(format!("DTSTART;VALUE=DATE:{}", graph_date_to_ics(s))); }
        if let Some(s) = &end_dt { v.push(format!("DTEND;VALUE=DATE:{}", graph_date_to_ics(s))); }
    } else {
        if let Some(s) = &start_dt { v.push(format!("DTSTART:{}", graph_dt_to_ics_utc(s))); }
        if let Some(s) = &end_dt { v.push(format!("DTEND:{}", graph_dt_to_ics_utc(s))); }
    }

    if let Some(loc) = e.get("location").and_then(|l| str_field(l, "displayName")) {
        v.push(format!("LOCATION:{}", escape_vcard(&loc)));
    }
    if let Some(body) = e.get("body") {
        if let Some(content) = str_field(body, "content") {
            let text = if body.get("contentType").and_then(Value::as_str) == Some("html") {
                strip_html(&content)
            } else {
                content
            };
            if !text.is_empty() { v.push(format!("DESCRIPTION:{}", escape_vcard(&text))); }
        }
    }
    if let Some(rec) = e.get("recurrence") {
        if !rec.is_null() {
            if let Some(rrule) = graph_recurrence_to_rrule(rec) {
                v.push(format!("RRULE:{rrule}"));
            }
        }
    }

    v.push("END:VEVENT".to_string());
    v.push("END:VCALENDAR".to_string());
    v.join("\r\n")
}

/// Convert an iCalendar VEVENT string to a Graph `event` resource.
pub fn ics_to_graph_event(ics: &str) -> Value {
    let mut subject = String::new();
    let mut location = String::new();
    let mut description = String::new();
    let mut start = json!(null);
    let mut end = json!(null);
    let mut all_day = false;
    let mut rrule: Option<String> = None;
    let mut start_date = String::new();

    for (name, params, value) in parse_vcard_lines(ics) {
        let up = params.to_uppercase();
        match name.as_str() {
            "SUMMARY" => subject = unescape_vcard(&value),
            "LOCATION" => location = unescape_vcard(&value),
            "DESCRIPTION" => description = unescape_vcard(&value),
            "RRULE" => rrule = Some(value),
            "DTSTART" | "DTEND" => {
                let (dt, tz, day) = if up.contains("VALUE=DATE") {
                    all_day = true;
                    (ics_date_to_graph(&value), "UTC".to_string(), true)
                } else if value.ends_with('Z') {
                    (ics_utc_to_graph(&value), "UTC".to_string(), false)
                } else if let Some(tzid) = params.split(';').find_map(|p| p.strip_prefix("TZID=")) {
                    (ics_local_to_graph(&value), tzid.to_string(), false)
                } else {
                    (ics_local_to_graph(&value), "UTC".to_string(), false)
                };
                let obj = json!({ "dateTime": dt, "timeZone": tz });
                if name == "DTSTART" {
                    start_date = if day { value[..value.len().min(8)].to_string() } else { dt_date(&dt) };
                    start = obj;
                } else {
                    end = obj;
                }
            }
            _ => {}
        }
    }

    let mut obj = serde_json::Map::new();
    obj.insert("subject".into(), json!(subject));
    if !start.is_null() { obj.insert("start".into(), start); }
    if !end.is_null() { obj.insert("end".into(), end); }
    obj.insert("isAllDay".into(), json!(all_day));
    if !location.is_empty() { obj.insert("location".into(), json!({ "displayName": location })); }
    if !description.is_empty() {
        obj.insert("body".into(), json!({ "contentType": "text", "content": description }));
    }
    if let Some(r) = rrule {
        let sd = if start_date.len() >= 10 { start_date } else { String::new() };
        if let Some(rec) = rrule_to_graph_recurrence(&r, &sd) {
            obj.insert("recurrence".into(), rec);
        }
    }
    Value::Object(obj)
}

fn dt_date(graph_dt: &str) -> String {
    if graph_dt.len() >= 10 { graph_dt[..10].to_string() } else { graph_dt.to_string() }
}

/// A dedup key for a calendar item: its UID.
pub fn event_key(ics: &str) -> String {
    for (name, _p, value) in parse_vcard_lines(ics) {
        if name == "UID" {
            return unescape_vcard(&value).trim().to_lowercase();
        }
    }
    String::new()
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

    #[test]
    fn graph_event_to_ics_timed() {
        let e = json!({
            "iCalUId": "EVT-1",
            "subject": "Standup",
            "isAllDay": false,
            "start": { "dateTime": "2026-09-14T09:00:00.0000000", "timeZone": "UTC" },
            "end": { "dateTime": "2026-09-14T09:15:00.0000000", "timeZone": "UTC" },
            "location": { "displayName": "Room 1" }
        });
        let ics = graph_event_to_ics(&e);
        assert!(ics.contains("UID:EVT-1"));
        assert!(ics.contains("SUMMARY:Standup"));
        assert!(ics.contains("DTSTART:20260914T090000Z"));
        assert!(ics.contains("DTEND:20260914T091500Z"));
        assert!(ics.contains("LOCATION:Room 1"));
    }

    #[test]
    fn graph_event_all_day() {
        let e = json!({
            "iCalUId": "EVT-2", "subject": "Holiday", "isAllDay": true,
            "start": { "dateTime": "2026-12-24T00:00:00.0000000", "timeZone": "UTC" },
            "end": { "dateTime": "2026-12-25T00:00:00.0000000", "timeZone": "UTC" }
        });
        let ics = graph_event_to_ics(&e);
        assert!(ics.contains("DTSTART;VALUE=DATE:20261224"));
        assert!(ics.contains("DTEND;VALUE=DATE:20261225"));
    }

    #[test]
    fn weekly_recurrence_round_trips() {
        let rec = json!({
            "pattern": { "type": "weekly", "interval": 1, "daysOfWeek": ["monday", "wednesday"] },
            "range": { "type": "numbered", "startDate": "2026-09-14", "numberOfOccurrences": 10 }
        });
        let rrule = graph_recurrence_to_rrule(&rec).unwrap();
        assert!(rrule.contains("FREQ=WEEKLY"));
        assert!(rrule.contains("BYDAY=MO,WE"));
        assert!(rrule.contains("COUNT=10"));

        let back = rrule_to_graph_recurrence(&rrule, "2026-09-14").unwrap();
        assert_eq!(back["pattern"]["type"], "weekly");
        assert_eq!(back["range"]["numberOfOccurrences"], 10);
        let days = back["pattern"]["daysOfWeek"].as_array().unwrap();
        assert!(days.contains(&json!("monday")) && days.contains(&json!("wednesday")));
    }

    #[test]
    fn ics_to_graph_event_timed() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:X1\r\nSUMMARY:Review\r\nDTSTART:20260914T130000Z\r\nDTEND:20260914T140000Z\r\nLOCATION:HQ\r\nEND:VEVENT\r\nEND:VCALENDAR";
        let e = ics_to_graph_event(ics);
        assert_eq!(e["subject"], "Review");
        assert_eq!(e["start"]["dateTime"], "2026-09-14T13:00:00");
        assert_eq!(e["start"]["timeZone"], "UTC");
        assert_eq!(e["isAllDay"], false);
        assert_eq!(e["location"]["displayName"], "HQ");
        assert_eq!(event_key(ics), "x1");
    }
}
