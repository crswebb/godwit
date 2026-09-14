//! Per-provider connectors and the migration runner. A `Connector` represents
//! one account and yields engine `Reader`/`Writer`s per data kind; `run` wires a
//! source and destination connector to the engine, the same way for every
//! provider combination.

pub mod imapdav;
pub mod ms365;

use crate::domain::{
    emit, Account, DavPair, FolderReport, NamedDavReport, Probe, Progress, Selection, UnifiedReport,
};
use crate::{dav, engine};

#[derive(Clone, Copy)]
pub enum Kind {
    Mail,
    Calendar,
    Contacts,
}

pub trait Connector {
    fn probe(&self) -> Probe;
    /// Mail folders (for delimiter/special-use mapping). Empty when the provider
    /// resolves folders itself (see `remaps_folders`).
    fn folders(&self) -> Result<Vec<crate::domain::FolderInfo>, String>;
    /// Whether folder names should be remapped for this destination. False for
    /// providers (M365) that resolve folder names on their own.
    fn remaps_folders(&self) -> bool;
    fn reader(&self, kind: Kind) -> Result<Box<dyn engine::Reader>, String>;
    fn writer(&self, kind: Kind) -> Result<Box<dyn engine::Writer>, String>;
}

pub fn build(account: &Account) -> Box<dyn Connector> {
    if account.is_microsoft() {
        Box::new(ms365::Ms365Connector::new(account.clone()))
    } else {
        Box::new(imapdav::ImapDavConnector::new(account.clone()))
    }
}

pub fn probe(account: &Account) -> Probe {
    build(account).probe()
}

fn to_dav(fr: &FolderReport) -> dav::DavReport {
    dav::DavReport {
        total: fr.source_total,
        copied: fr.copied,
        skipped: fr.skipped,
        failed: fr.failed,
        missing: fr.missing,
    }
}

/// Run a migration for the selected folders / calendars / address books.
pub fn run(
    app: &tauri::AppHandle,
    source: Account,
    destination: Account,
    selection: Selection,
    dry_run: bool,
) -> Result<UnifiedReport, String> {
    let src = build(&source);
    let dst = build(&destination);
    let mut report = UnifiedReport::default();

    let total = selection.folders.len() + selection.calendars.len() + selection.contacts.len();
    emit(app, Progress::Started { folders: total });
    let mut index = 0usize;

    // Mail
    if !selection.folders.is_empty() {
        let src_delim = src.folders().ok().and_then(|fs| fs.into_iter().find_map(|f| f.delimiter));
        let dst_folders = if dst.remaps_folders() { dst.folders()? } else { Vec::new() };
        let dst_delim = dst_folders.iter().find_map(|f| f.delimiter.clone());
        let pairs = engine::plan_targets(
            &selection.folders,
            src_delim.as_deref(),
            &dst_folders,
            dst_delim.as_deref(),
            dst.remaps_folders(),
        );
        let mut reader = src.reader(Kind::Mail)?;
        let mut writer = dst.writer(Kind::Mail)?;
        for (s, d) in pairs {
            let fr = copy_one(app, reader.as_mut(), writer.as_mut(), &s, &d, index, total, dry_run, &s);
            report.folders.push(fr);
            index += 1;
        }
    }

    report.calendars = run_pim(app, src.as_ref(), dst.as_ref(), Kind::Calendar, &selection.calendars, &mut index, total, dry_run);
    report.contacts = run_pim(app, src.as_ref(), dst.as_ref(), Kind::Contacts, &selection.contacts, &mut index, total, dry_run);

    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn copy_one(
    app: &tauri::AppHandle,
    reader: &mut dyn engine::Reader,
    writer: &mut dyn engine::Writer,
    src: &str,
    dst: &str,
    index: usize,
    total: usize,
    dry_run: bool,
    label: &str,
) -> FolderReport {
    let fr = match engine::copy_collection(app, reader, writer, src, dst, index, total, dry_run) {
        Ok(fr) => fr,
        Err(e) => {
            emit(app, Progress::Warning { message: format!("{label}: {e}") });
            FolderReport { folder: label.to_string(), ..Default::default() }
        }
    };
    emit(app, Progress::FolderDone { report: fr.clone() });
    fr
}

#[allow(clippy::too_many_arguments)]
fn run_pim(
    app: &tauri::AppHandle,
    src: &dyn Connector,
    dst: &dyn Connector,
    kind: Kind,
    pairs: &[DavPair],
    index: &mut usize,
    total: usize,
    dry_run: bool,
) -> Vec<NamedDavReport> {
    if pairs.is_empty() {
        return Vec::new();
    }
    let mut reader = match src.reader(kind) {
        Ok(r) => r,
        Err(e) => return fail_all(app, pairs, index, &e),
    };
    let mut writer = match dst.writer(kind) {
        Ok(w) => w,
        Err(e) => return fail_all(app, pairs, index, &e),
    };

    let mut out = Vec::new();
    for pair in pairs {
        let fr = copy_one(app, reader.as_mut(), writer.as_mut(), &pair.source, &pair.dest, *index, total, dry_run, &pair.name);
        out.push(NamedDavReport { name: pair.name.clone(), report: to_dav(&fr) });
        *index += 1;
    }
    out
}

fn fail_all(app: &tauri::AppHandle, pairs: &[DavPair], index: &mut usize, err: &str) -> Vec<NamedDavReport> {
    pairs
        .iter()
        .map(|p| {
            emit(app, Progress::Warning { message: format!("{}: {err}", p.name) });
            *index += 1;
            NamedDavReport { name: p.name.clone(), ..Default::default() }
        })
        .collect()
}
