//! Provider-agnostic migration engine. A source is a `Reader`, a destination is
//! a `Writer`, and everything moves as a canonical `Item` (raw MIME for mail,
//! iCalendar / vCard text for calendar / contacts). One `copy` loop does dedup,
//! progress, and verification for every data kind — connectors never appear
//! here, and the engine never asks "is this Microsoft 365?".

use crate::domain::{emit, FolderReport, Progress};
use chrono::{DateTime, FixedOffset};
use std::collections::{HashMap, HashSet};

/// Standard mail flags worth carrying where the protocols allow (mail only).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MailFlag {
    Seen,
    Answered,
    Flagged,
    Draft,
}

pub enum Payload {
    Mime(Vec<u8>),
    Text(String),
}

/// A single item in canonical form, with the dedup key and (mail-only) metadata.
pub struct Item {
    pub key: String,
    pub payload: Payload,
    pub flags: Vec<MailFlag>,
    pub date: Option<DateTime<FixedOffset>>,
}

impl Item {
    pub fn text(key: String, body: String) -> Self {
        Item { key, payload: Payload::Text(body), flags: Vec::new(), date: None }
    }
    pub fn mime(key: String, bytes: Vec<u8>, flags: Vec<MailFlag>, date: Option<DateTime<FixedOffset>>) -> Self {
        Item { key, payload: Payload::Mime(bytes), flags, date }
    }
    pub fn bytes(&self) -> Vec<u8> {
        match &self.payload {
            Payload::Mime(b) => b.clone(),
            Payload::Text(s) => s.clone().into_bytes(),
        }
    }
    pub fn as_text(&self) -> String {
        match &self.payload {
            Payload::Text(s) => s.clone(),
            Payload::Mime(b) => String::from_utf8_lossy(b).into_owned(),
        }
    }
}

/// A listed item: an opaque per-source id and its dedup key (may be empty when
/// the source has no stable id, in which case it's always copied).
pub struct ItemRef {
    pub id: String,
    pub key: String,
}

pub trait Reader {
    fn list(&mut self, collection: &str) -> Result<Vec<ItemRef>, String>;
    fn fetch(&mut self, collection: &str, item: &ItemRef) -> Result<Item, String>;
}

pub trait Writer {
    fn ensure(&mut self, collection: &str) -> Result<(), String>;
    fn existing_keys(&mut self, collection: &str) -> Result<HashSet<String>, String>;
    fn write(&mut self, collection: &str, item: &Item) -> Result<(), String>;
}

/// Copy one source collection to one destination collection: dedup by key,
/// stream items, verify by re-reading the destination. Emits progress.
#[allow(clippy::too_many_arguments)]
pub fn copy_collection(
    app: &tauri::AppHandle,
    reader: &mut dyn Reader,
    writer: &mut dyn Writer,
    src: &str,
    dst: &str,
    index: usize,
    total_collections: usize,
    dry_run: bool,
) -> Result<FolderReport, String> {
    writer.ensure(dst)?;
    let dest_before = writer.existing_keys(dst)?;
    let refs = reader.list(src)?;
    let total = refs.len() as u32;
    emit(app, Progress::FolderStart {
        name: src.to_string(),
        index,
        folders: total_collections,
        source_total: total,
    });

    let to_copy: Vec<&ItemRef> = refs
        .iter()
        .filter(|r| r.key.is_empty() || !dest_before.contains(&r.key))
        .collect();

    let display = if src == dst { src.to_string() } else { format!("{src} → {dst}") };
    let mut fr = FolderReport {
        folder: display,
        source_total: total,
        skipped: (refs.len() - to_copy.len()) as u32,
        ..Default::default()
    };

    if dry_run {
        fr.copied = to_copy.len() as u32;
        return Ok(fr);
    }

    let target = to_copy.len() as u32;
    let mut done = 0u32;
    for r in to_copy {
        done += 1;
        match reader.fetch(src, r) {
            Ok(item) => match writer.write(dst, &item) {
                Ok(()) => fr.copied += 1,
                Err(e) => {
                    fr.failed += 1;
                    emit(app, Progress::Warning { message: format!("{src}: {e}") });
                }
            },
            Err(e) => {
                fr.failed += 1;
                emit(app, Progress::Warning { message: format!("{src}: {e}") });
            }
        }
        if done % 25 == 0 || done == target {
            emit(app, Progress::Tick { folder: src.to_string(), done, total: target });
        }
    }

    let dest_after = writer.existing_keys(dst)?;
    fr.missing = refs
        .iter()
        .filter(|r| !r.key.is_empty() && !dest_after.contains(&r.key))
        .count() as u32;

    Ok(fr)
}

// --- Folder mapping between hosts -----------------------------------------

fn is_inbox(name: &str) -> bool {
    name.eq_ignore_ascii_case("INBOX")
}

fn leaf_of(name: &str, delim: Option<&str>) -> String {
    match delim {
        Some(d) if !d.is_empty() => name.rsplit(d).next().unwrap_or(name).to_string(),
        _ => name.to_string(),
    }
}

/// Re-express a folder path using the destination's hierarchy delimiter.
fn translate_path(name: &str, src: Option<&str>, dst: Option<&str>) -> String {
    match (src, dst) {
        (Some(s), Some(d)) if s != d && !s.is_empty() && !d.is_empty() => name.replace(s, d),
        _ => name.to_string(),
    }
}

/// Normalise a folder leaf name to a special-use category, if any.
fn canonical_special(leaf: &str) -> Option<&'static str> {
    match leaf.trim().to_lowercase().as_str() {
        "sent" | "sent messages" | "sent items" | "sent mail" => Some("sent"),
        "drafts" | "draft" => Some("drafts"),
        "trash" | "deleted" | "deleted messages" | "deleted items" | "bin" => Some("trash"),
        "junk" | "spam" | "junk email" | "junk e-mail" => Some("junk"),
        "archive" | "archives" => Some("archive"),
        _ => None,
    }
}

/// Map each selected source folder to the right destination folder name.
/// `dest_remaps` is false when the destination resolves folders itself (M365),
/// in which case names pass through unchanged.
pub fn plan_targets(
    folders: &[String],
    src_delim: Option<&str>,
    dst_folders: &[crate::domain::FolderInfo],
    dst_delim: Option<&str>,
    dest_remaps: bool,
) -> Vec<(String, String)> {
    if !dest_remaps {
        return folders.iter().map(|f| (f.clone(), f.clone())).collect();
    }

    let mut dst_special: HashMap<&'static str, String> = HashMap::new();
    for f in dst_folders {
        let leaf = leaf_of(&f.name, dst_delim);
        if let Some(c) = canonical_special(&leaf) {
            dst_special.entry(c).or_insert_with(|| f.name.clone());
        }
    }

    folders
        .iter()
        .map(|f| {
            if is_inbox(f) {
                return (f.clone(), "INBOX".to_string());
            }
            let leaf = leaf_of(f, src_delim);
            if let Some(c) = canonical_special(&leaf) {
                if let Some(dest_name) = dst_special.get(c) {
                    return (f.clone(), dest_name.clone());
                }
            }
            (f.clone(), translate_path(f, src_delim, dst_delim))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_hierarchy_delimiter() {
        assert_eq!(translate_path("INBOX.Archive.2023", Some("."), Some("/")), "INBOX/Archive/2023");
        assert_eq!(translate_path("A/B", Some("/"), Some("/")), "A/B");
        assert_eq!(translate_path("Sent", Some("."), None), "Sent");
    }

    #[test]
    fn recognises_special_use_synonyms() {
        assert_eq!(canonical_special("Sent Items"), Some("sent"));
        assert_eq!(canonical_special("sent messages"), Some("sent"));
        assert_eq!(canonical_special("JUNK"), Some("junk"));
        assert_eq!(canonical_special("Deleted Items"), Some("trash"));
        assert_eq!(canonical_special("Projects"), None);
    }

    #[test]
    fn takes_leaf_by_delimiter() {
        assert_eq!(leaf_of("INBOX.Sent", Some(".")), "Sent");
        assert_eq!(leaf_of("INBOX/Archive/Old", Some("/")), "Old");
        assert_eq!(leaf_of("Sent Items", None), "Sent Items");
    }

    #[test]
    fn inbox_is_case_insensitive() {
        assert!(is_inbox("INBOX"));
        assert!(is_inbox("inbox"));
        assert!(!is_inbox("INBOX.Sent"));
    }

    #[test]
    fn special_use_routes_to_destination_folder() {
        let dst = vec![
            crate::domain::FolderInfo { name: "Sent Items".into(), delimiter: Some("/".into()), attributes: vec![] },
            crate::domain::FolderInfo { name: "INBOX".into(), delimiter: Some("/".into()), attributes: vec![] },
        ];
        let pairs = plan_targets(&["INBOX.Sent".to_string()], Some("."), &dst, Some("/"), true);
        assert_eq!(pairs[0], ("INBOX.Sent".to_string(), "Sent Items".to_string()));
    }

    #[test]
    fn m365_destination_passes_names_through() {
        let pairs = plan_targets(&["INBOX.Sent".to_string()], Some("."), &[], None, false);
        assert_eq!(pairs[0], ("INBOX.Sent".to_string(), "INBOX.Sent".to_string()));
    }
}
