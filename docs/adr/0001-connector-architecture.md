# ADR-0001: Refactor to the connector architecture

**Status:** Accepted
**Date:** 2026-09-14
**Deciders:** Stefan

## Context

Godwit was built feature-first to validate the product quickly. That worked — we
learned the real requirements by shipping (same-address domain moves, Microsoft
365's Basic-Auth wall, folder-delimiter mismatches, Graph↔iCal/vCard conversion).
But the code drifted from the design in [`ARCHITECTURE.md`](../ARCHITECTURE.md),
which describes a **hub-and-spoke of per-provider connectors** over canonical
formats (MIME / iCalendar / vCard).

Current reality (2026-09-14):

- **`main.rs` is 736 lines / 35 functions** — a god-module mixing five concerns:
  Tauri command handlers, migration orchestration, per-protocol glue, the domain
  types, and unit tests.
- The connector abstraction exists **only for mail** (`MailReader`/`MailWriter`).
  **Calendar and contacts branch on `account.is_microsoft()`** in three separate
  orchestrators (`migrate_calendar`, `migrate_contacts`, and mail's
  `make_reader`/`make_writer`) — **9 such branches** scattered across the code.
  That triplication is the smell of a missing abstraction.
- Graph code is split inconsistently between `microsoft.rs` (folders, contacts,
  events) and `mail.rs` (`GraphMail`).

Forces at play:

- **Correctness must be preserved.** The only path exercised against real servers
  is IMAP↔IMAP mail (the live CloudAccess→Loopia use case). Everything else is
  compiler- and unit-test-verified but runtime-untested.
- **A test safety net exists:** 12 unit tests (folder mapping + all converters).
- **Feature set is now known and stable** — the uncertainty that justified the
  fast-and-loose phase is gone.
- Solo developer; no deadline pressure; this is maintenance investment, not a ship
  blocker.

## Decision

Refactor to the documented connector architecture **without changing behavior**:

1. Introduce a **`Connector` trait** per provider that yields a reader and a writer
   for each data kind (mail, calendar, contacts) over canonical formats.
2. Add a single provider-agnostic **`engine`** that copies items between any
   reader and writer (dedup + verify + progress in one place), replacing the three
   near-identical orchestrators and eliminating all `is_microsoft()` branching.
3. Split `main.rs` into `domain` (types), `engine`, `connectors/*`, and a thin
   `commands` layer; `main.rs` becomes ~20 lines.
4. **Keep every Tauri command name and argument shape identical**, so the Svelte
   frontend needs zero changes — a hard invariant that bounds the blast radius.

Target layout:

```
domain.rs          Account, Provider, Probe, Selection, reports, Progress — pure data
engine.rs          generic copy(reader, writer, pairs, dry_run): dedup + verify + progress
connectors/
  mod.rs           Connector trait + build_connector(account)
  imapdav.rs       generic IMAP + CalDAV + CardDAV host
  microsoft.rs     Microsoft Graph (OAuth + mail/calendar/contacts)
convert.rs         pure Graph↔iCal/vCard converters (unchanged)
autoconfig.rs      IMAP autodiscovery (unchanged; may move under connectors/)
commands.rs        thin Tauri handlers: probe / run_migration / ms_sign_in
main.rs            builder + handler registration only
```

The canonical **item** carries a dedup key, a payload (raw MIME bytes or
iCal/vCard text), and optional mail metadata (flags, internal date) used only by
the IMAP mail writer. A `Connector` exposes `reader(kind)` / `writer(kind)`; the
engine is the only place that loops, dedups, and verifies.

## Options Considered

### Option A: Leave it as is

| Dimension | Assessment |
|---|---|
| Complexity | Low (no work) |
| Cost | Zero now, compounding later |
| Extensibility | Poor — each new provider adds more scattered branches |
| Risk | Zero to current behavior |

**Pros:** No effort; no risk of regressing the working IMAP path.
**Cons:** Adding Google (next connector) triples the branching again; `main.rs`
keeps growing; the docs describe an architecture the code doesn't have; onboarding
and reasoning stay hard.

### Option B: Cosmetic split (move types/commands out, stop there)

| Dimension | Assessment |
|---|---|
| Complexity | Low |
| Cost | ~half a session |
| Extensibility | Still poor — the `is_microsoft()` triplication remains |
| Risk | Very low (pure moves) |

**Pros:** Shrinks `main.rs`, quick win.
**Cons:** Doesn't fix the actual problem (missing connector abstraction, duplicated
copy logic). Cost without the main benefit.

### Option C: Full connector refactor (recommended)

| Dimension | Assessment |
|---|---|
| Complexity | Medium — reorganization + one trait, no new behavior |
| Cost | ~1–2 focused sessions, incremental |
| Extensibility | Strong — a new provider = one `Connector` impl, zero engine changes |
| Risk | Low–medium, mitigated by tests + green-at-every-step + a final live re-test |

**Pros:** Realizes the documented design; one copy path instead of three; removes
9 branches; makes Google/keychain/etc. drop-in; each module gets one job.
**Cons:** Touches the working IMAP path, so it must be done carefully and re-tested
live at the end.

## Trade-off Analysis

The core trade is **effort + a bounded regression risk now** vs. **compounding
drift and duplicated logic forever**. Two facts tilt it decisively toward Option C:
the feature set is settled (so we're not refactoring a moving target), and the
converters/mapping already have unit tests (so the riskiest pure logic is pinned).
Option B spends most of C's cost for little of its benefit. Option A is only
correct if the project is being frozen — it isn't (Google, keychain, packaging
remain). **Recommend Option C.**

The single real risk — regressing IMAP↔IMAP mail — is contained by three rules:
behavior-preserving moves only (no feature changes mixed in), the app stays
compiling with all tests green after every step, and Stefan re-runs a real
Loopia→Loopia (and CloudAccess→Loopia) migration at the end before the refactor is
called done.

## Consequences

**Easier:** adding a provider (implement `Connector`, done); reasoning about one
copy engine; testing (engine and connectors mockable in isolation); keeping code
and `ARCHITECTURE.md` in agreement.

**Harder / cost:** a period of churn; the mail-metadata (flags/date) nuance must be
carried through the generic item cleanly; more files to navigate (offset by each
being small and single-purpose).

**Revisit later:** whether calendar/contacts need per-collection (not just default
M365 folder) support; whether the engine should stream rather than collect item
lists for very large mailboxes.

## Action Items — behavior-preserving migration order

Each step ends green: `cargo check` clean, `cargo test` = 12 passing, `npm run
build` + `svelte-check` clean. Commit per step so any regression is bisectable.

1. [ ] **Baseline.** Confirm 12 tests + build green; tag the commit as the
       pre-refactor reference.
2. [ ] **`domain.rs`.** Move pure types out of `main.rs` (Account, Provider,
       FolderInfo, FolderReport, NamedDavReport, UnifiedReport, Probe, Selection,
       Progress). Pure moves, no logic. Green.
3. [ ] **`commands.rs`.** Move the three `#[tauri::command]` fns into it as thin
       wrappers still calling the existing functions; `main.rs` → builder only.
       Command names/args unchanged (frontend untouched). Green.
4. [ ] **Canonical item + Reader/Writer traits in `engine.rs`.** Generalize the
       existing `MailReader`/`MailWriter` into kind-agnostic traits over a canonical
       `Item { key, payload, mail_meta? }`. No call sites changed yet. Green.
5. [ ] **`connectors/imapdav.rs`.** Implement `Connector` for the generic host by
       wrapping today's `ImapReader`/`ImapWriter` and `dav::read_items`/`write_item`.
       Behavior identical. Green.
6. [ ] **`connectors/microsoft.rs`.** Implement `Connector` for Graph; move
       `GraphMail` here from `mail.rs` and reuse `microsoft`'s contacts/events calls.
       Green.
7. [ ] **Generic engine copy.** Replace `copy_folder` + `migrate_contacts` +
       `migrate_calendar` with one `engine::copy(reader, writer, pairs, dry_run)`.
       `run_migration` now builds a source and a destination `Connector` and drives
       the engine per data kind. **Delete all 9 `is_microsoft()` branches.** Green.
8. [ ] **Fold in folder mapping.** Move `plan_targets` to where it belongs (engine
       or the imapdav connector). Green.
9. [ ] **Sweep.** Ensure each module has one responsibility; delete dead code;
       confirm `main.rs` is ~20 lines. Green.
10. [ ] **Live re-test.** Stefan runs a real Loopia→Loopia and CloudAccess→Loopia
        migration (dry-run then real) to confirm no behavior regressed. Only then is
        the refactor "done".

## Non-goals

- No new features during the refactor (no keychain, Google, packaging — those come
  after, and become easier).
- No change to the Tauri command surface or the Svelte UI.
- No change to on-the-wire behavior (same IMAP/DAV/Graph calls).
