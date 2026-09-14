//! Provider-agnostic migration engine. A source is a `Reader`, a destination is
//! a `Writer`, and everything moves as a canonical `Item` (raw MIME for mail,
//! iCalendar / vCard text for calendar / contacts). One `copy` loop does dedup,
//! progress, and verification for every data kind — connectors never appear
//! here, and the engine never asks "is this Microsoft 365?".

use crate::domain::{FolderReport, Progress};
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

#[derive(Clone)]
pub enum Payload {
    Mime(Vec<u8>),
    Text(String),
}

/// A single item in canonical form, with the dedup key and (mail-only) metadata.
#[derive(Clone)]
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
/// stream items, verify by re-reading the destination. Progress is reported
/// through the `on` callback (keeps the engine free of any UI/Tauri coupling).
#[allow(clippy::too_many_arguments)]
pub fn copy_collection(
    reader: &mut dyn Reader,
    writer: &mut dyn Writer,
    src: &str,
    dst: &str,
    index: usize,
    total_collections: usize,
    dry_run: bool,
    on: &mut dyn FnMut(Progress),
) -> Result<FolderReport, String> {
    writer.ensure(dst)?;
    let dest_before = writer.existing_keys(dst)?;
    let refs = reader.list(src)?;
    let total = refs.len() as u32;
    on(Progress::FolderStart {
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
                    on(Progress::Warning { message: format!("{src}: {e}") });
                }
            },
            Err(e) => {
                fr.failed += 1;
                on(Progress::Warning { message: format!("{src}: {e}") });
            }
        }
        if done % 25 == 0 || done == target {
            on(Progress::Tick { folder: src.to_string(), done, total: target });
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

    // A faithful in-memory mailbox that actually stores and reflects writes, so
    // copy_collection's real behaviour (dedup, verify, counts) is exercised —
    // not a canned mock. `drop_keys` models a lossy sink (write returns Ok but
    // nothing lands); `fail_keys` models an outright rejection.
    struct MemStore {
        data: HashMap<String, Vec<Item>>,
        drop_keys: HashSet<String>,
        fail_keys: HashSet<String>,
    }

    impl MemStore {
        fn empty() -> Self {
            Self { data: HashMap::new(), drop_keys: HashSet::new(), fail_keys: HashSet::new() }
        }
        fn seeded(collection: &str, items: Vec<Item>) -> Self {
            let mut s = Self::empty();
            s.data.insert(collection.to_string(), items);
            s
        }
        fn count(&self, collection: &str) -> usize {
            self.data.get(collection).map(Vec::len).unwrap_or(0)
        }
    }

    impl Reader for MemStore {
        fn list(&mut self, collection: &str) -> Result<Vec<ItemRef>, String> {
            Ok(self
                .data
                .get(collection)
                .map(|v| v.iter().enumerate().map(|(i, it)| ItemRef { id: i.to_string(), key: it.key.clone() }).collect())
                .unwrap_or_default())
        }
        fn fetch(&mut self, collection: &str, r: &ItemRef) -> Result<Item, String> {
            let i: usize = r.id.parse().map_err(|_| "bad id".to_string())?;
            self.data.get(collection).and_then(|v| v.get(i)).cloned().ok_or_else(|| "gone".into())
        }
    }

    impl Writer for MemStore {
        fn ensure(&mut self, collection: &str) -> Result<(), String> {
            self.data.entry(collection.to_string()).or_default();
            Ok(())
        }
        fn existing_keys(&mut self, collection: &str) -> Result<HashSet<String>, String> {
            Ok(self
                .data
                .get(collection)
                .map(|v| v.iter().map(|it| it.key.clone()).filter(|k| !k.is_empty()).collect())
                .unwrap_or_default())
        }
        fn write(&mut self, collection: &str, item: &Item) -> Result<(), String> {
            if self.fail_keys.contains(&item.key) {
                return Err("rejected".into());
            }
            if self.drop_keys.contains(&item.key) {
                return Ok(()); // silently lost
            }
            self.data.entry(collection.to_string()).or_default().push(item.clone());
            Ok(())
        }
    }

    fn item(key: &str, body: &str) -> Item {
        Item::text(key.to_string(), body.to_string())
    }

    fn silent() -> impl FnMut(Progress) {
        |_p| {}
    }

    #[test]
    fn copies_all_items_to_empty_destination() {
        let mut src = MemStore::seeded("c", vec![item("a", "1"), item("b", "2"), item("c3", "3")]);
        let mut dst = MemStore::empty();
        let fr = copy_collection(&mut src, &mut dst, "c", "c", 0, 1, false, &mut silent()).unwrap();
        assert_eq!((fr.source_total, fr.copied, fr.skipped, fr.failed, fr.missing), (3, 3, 0, 0, 0));
        assert_eq!(dst.count("c"), 3);
    }

    #[test]
    fn dedup_copies_only_the_delta() {
        let mut src = MemStore::seeded("c", vec![item("a", "1"), item("b", "2"), item("c3", "3")]);
        let mut dst = MemStore::seeded("c", vec![item("a", "1")]); // "a" already present
        let fr = copy_collection(&mut src, &mut dst, "c", "c", 0, 1, false, &mut silent()).unwrap();
        assert_eq!((fr.copied, fr.skipped, fr.missing), (2, 1, 0));
        assert_eq!(dst.count("c"), 3);

        // Re-running copies nothing (fully deduped).
        let again = copy_collection(&mut src, &mut dst, "c", "c", 0, 1, false, &mut silent()).unwrap();
        assert_eq!((again.copied, again.skipped), (0, 3));
        assert_eq!(dst.count("c"), 3);
    }

    #[test]
    fn dry_run_writes_nothing_but_counts_would_copy() {
        let mut src = MemStore::seeded("c", vec![item("a", "1"), item("b", "2")]);
        let mut dst = MemStore::empty();
        let fr = copy_collection(&mut src, &mut dst, "c", "c", 0, 1, true, &mut silent()).unwrap();
        assert_eq!((fr.copied, fr.skipped), (2, 0));
        assert_eq!(dst.count("c"), 0); // nothing actually written
    }

    #[test]
    fn verify_detects_silently_lost_item() {
        let mut src = MemStore::seeded("c", vec![item("a", "1"), item("b", "2"), item("c3", "3")]);
        let mut dst = MemStore::empty();
        dst.drop_keys.insert("b".into()); // write reports Ok but "b" never lands
        let fr = copy_collection(&mut src, &mut dst, "c", "c", 0, 1, false, &mut silent()).unwrap();
        assert_eq!(fr.copied, 3); // write() said Ok for all three
        assert_eq!(fr.failed, 0);
        assert_eq!(fr.missing, 1); // ...but verification caught the loss
        assert_eq!(dst.count("c"), 2);
    }

    #[test]
    fn rejected_write_counts_failed_and_missing() {
        let mut src = MemStore::seeded("c", vec![item("a", "1"), item("b", "2")]);
        let mut dst = MemStore::empty();
        dst.fail_keys.insert("b".into());
        let fr = copy_collection(&mut src, &mut dst, "c", "c", 0, 1, false, &mut silent()).unwrap();
        assert_eq!((fr.copied, fr.failed, fr.missing), (1, 1, 1));
        assert_eq!(dst.count("c"), 1);
    }

    #[test]
    fn items_without_a_key_are_always_copied() {
        let mut src = MemStore::seeded("c", vec![item("", "no-message-id")]);
        let mut dst = MemStore::empty();
        let first = copy_collection(&mut src, &mut dst, "c", "c", 0, 1, false, &mut silent()).unwrap();
        assert_eq!(first.copied, 1);
        assert_eq!(first.missing, 0); // empty keys aren't counted as missing
        // No dedup key, so a second run copies it again (documented behaviour).
        let second = copy_collection(&mut src, &mut dst, "c", "c", 0, 1, false, &mut silent()).unwrap();
        assert_eq!(second.copied, 1);
        assert_eq!(dst.count("c"), 2);
    }

    #[test]
    fn progress_reflects_the_actual_work() {
        let mut src = MemStore::seeded("c", vec![item("a", "1"), item("b", "2")]);
        let mut dst = MemStore::empty();
        dst.fail_keys.insert("b".into());
        let mut events: Vec<Progress> = Vec::new();
        copy_collection(&mut src, &mut dst, "c", "c", 0, 1, false, &mut |p| events.push(p)).unwrap();

        let started = events.iter().find_map(|e| match e {
            Progress::FolderStart { source_total, .. } => Some(*source_total),
            _ => None,
        });
        assert_eq!(started, Some(2));
        let warnings = events.iter().filter(|e| matches!(e, Progress::Warning { .. })).count();
        assert_eq!(warnings, 1); // one per failed item
    }
}
