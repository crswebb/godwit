# MVP Spec

## Goal

Ship a desktop app that can perform a **complete, verified migration of email + calendar + contacts**
between two standard IMAP/DAV hosts — proven on a real host-to-host migration — then extend to Google
and Microsoft 365 via connectors.

## Non-goals (for the MVP)

- No hosted/server version. Local-only.
- No automatic DNS/MX changes — we *guide* the cutover, we don't touch registrars.
- No provider beyond the three connectors below (others come later, additively).
- No mobile app.

---

## Phased roadmap

| Phase | Deliverable | Why |
| --- | --- | --- |
| **0. Spike** | Verify CalDAV/CardDAV on the first source/destination pair (see checklist). Bare Tauri app that connects to one IMAP box and lists folders. | De-risk the biggest unknown (host DAV support) before building. |
| **1. Email MVP** | Generic IMAP ↔ IMAP: folders, messages, flags, resume, verify. Wizard UI + progress + report. | The core value; your real host-to-host use case. |
| **2. Calendar + Contacts** | CalDAV + CardDAV in the Generic connector, with **ICS/vCard export→import fallback** if a host lacks DAV. | Calendar and contacts need to transfer too before it's usable. |
| **3. Google connector** | OAuth (PKCE) + Gmail/Calendar/People (or app-password + IMAP/CalDAV path). | Covers Google ↔ anything. |
| **4. Microsoft 365 connector** | OAuth (PKCE) + Microsoft Graph (mail, calendar, contacts). Entra app registration. | Mandatory OAuth; unlocks M365 ↔ anything. |
| **5. Polish & ship** | Guided cutover checklist (MX/DNS, device reconfig, forwarding), code signing/notarization, installers. | Turns a tool into a product a nervous SMB owner trusts. |

---

## Provider auth matrix

| Provider(s) | Email | Calendar | Contacts | Auth | Dev registration |
| --- | --- | --- | --- | --- | --- |
| Any standard IMAP/DAV host | IMAP | CalDAV *(or ICS)* | CardDAV *(or vCard)* | User credentials / app password | **None** |
| Google | Gmail API *or* IMAP | Calendar API *or* CalDAV | People API *or* CardDAV | OAuth 2.0 PKCE, *or* app password | 1 Google Cloud app (100 test users free; CASA review only for public scale) |
| Microsoft 365 | **Graph (required)** | Graph | Graph | **OAuth 2.0 PKCE only** (Basic Auth disabled) | 1 Entra (Azure AD) app |

---

## First migration — test plan (standard host → standard host)

**Setup:** a real source mailbox on the origin host, an empty target on the destination host.

**Steps:**
1. Enter source IMAP host/port/user + app password. Connect, list folders + counts.
2. Enter destination IMAP. Connect. Create matching folder tree.
3. Run email migration. Watch progress + per-folder counts.
4. Run calendar (CalDAV or ICS fallback) and contacts (CardDAV or vCard fallback).
5. Run verification pass.

**Success criteria (definition of done for Phase 1–2):**
- [ ] Destination message count per folder == source (minus known duplicates).
- [ ] Folder hierarchy preserved (incl. nested + special-use: Sent, Drafts, Trash, Archive).
- [ ] Flags preserved: read/unread, flagged, answered.
- [ ] Attachments intact (byte-compare a sample).
- [ ] No duplicate messages on a second run (idempotency).
- [ ] Interrupting mid-run and resuming completes correctly.
- [ ] All calendar events present on the destination.
- [ ] Contacts present with names, emails, phones intact.
- [ ] Source mailbox untouched.

---

## Pre-code research: CalDAV / CardDAV verification checklist

*~20 minutes, before writing connector code. This is the biggest unknown.*

For **each** host in the migration (source and destination):
- [ ] Which webmail is it? (Roundcube / Horde / SOGo / other)
- [ ] Is there a **CalDAV** URL? (look in webmail settings, or try
      `https://<mailserver>/.well-known/caldav`)
- [ ] Is there a **CardDAV** URL? (`/.well-known/carddav`)
- [ ] Do CalDAV/CardDAV authenticate with the same mail credentials or a separate password?
- [ ] If no DAV: can the webmail **export ICS / vCard** (fallback path)?
- [ ] IMAP host/port, TLS, and whether an **app-specific password** is required.

Record findings in `docs/providers/` (one file per provider).

---

## Tech stack (proposed — see `DECISIONS.md`)

- **Shell:** Tauri v2.
- **Core (Rust):** `tokio` (async), `async-imap` + `mail-parser` / `mail-builder` (email),
  `reqwest` + `quick-xml` for CalDAV/CardDAV (DAV client crates are immature — likely hand-roll
  PROPFIND/REPORT), `rusqlite` (job store), `keyring` (secrets), `oauth2` (PKCE flows).
- **Frontend:** _open_ — SvelteKit (lean) or React + Vite. Recommend SvelteKit for footprint.
- **Mail-engine fallback option:** bundle **`imapsync`** for the mail heavy-lifting if native Rust
  IMAP edge-case coverage proves risky (see Risks).

---

## Risks & mitigations

| Risk | Mitigation |
| --- | --- |
| Rust IMAP/DAV libs less battle-tested than Node's → edge-case data-loss bugs | Bundle & orchestrate **`imapsync`** for mail; heavy test suite; verification pass catches gaps. |
| Some hosts have weak/no CalDAV → calendar/contacts can't sync via DAV | **ICS/vCard export→import fallback** so they still transfer, even if via one guided manual step. |
| Google restricted-scope verification (CASA) is slow/costly | Ship in OAuth "testing" mode (100 users) initially; offer app-password + IMAP/CalDAV path meanwhile. |
| M365 Basic Auth disabled | Graph-only connector; Entra app registration from day one of Phase 4. |
| Large mailboxes (10–50 GB) tie up the machine | Streaming (never load whole mailbox in memory), batching, resumable checkpoints, throttle controls. |

---

## Open questions

1. Frontend framework: SvelteKit vs. React?
2. Native Rust IMAP vs. bundled `imapsync` for Phase 1 — decide after the spike.
3. Licensing/distribution model (free / paid / per-migration) — deferred, not urgent given goals.
4. Product name (Flyttfågel is a codename).
