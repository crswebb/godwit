# Key decisions

Short log of the decisions made during brainstorming, with rationale, so future-you remembers *why*.

## D1 — Desktop app, not a web service

**Decision:** Build a local desktop app; no hosted migration backend.

**Why:** Browsers cannot open raw TCP sockets, so IMAP/SMTP/CalDAV cannot run in a browser tab. A
hosted relay would (a) cost bandwidth per migration and (b) force us to hold users' credentials and
mailbox contents — the exact liability we want to avoid. A local app makes running cost ~zero,
scales infinitely, and keeps credentials on the device. The only real hosted piece is a static
site + optional license endpoint, neither of which sees user data.

## D2 — Tauri (Rust core + web UI), not Electron

**Decision:** Tauri v2.

**Why (for *this* product specifically):** the defining qualities are trust, data-integrity on long
jobs, and running heavy work on someone else's machine — all Rust strengths.
- **Security posture is the pitch** ("credentials never leave your machine") → Tauri's tiny dependency
  surface + memory-safe Rust beats Electron's bundled Chromium/Node + large npm tree.
- **Data integrity on hours-long jobs** → Rust's error handling makes robust, resumable processes.
- **Footprint** → lean native process keeps the client's machine usable during a large move.
- **Perception** → small, fast, native-feeling app for a trust product.
- UI is a wash (both use a web frontend), so nothing is lost on the interface.

**Accepted cost:** Rust's IMAP/DAV libraries are less battle-tested than Node's. Mitigated by
optionally bundling `imapsync` and by a strong verification pass. Running cost is zero either way,
so build velocity — not hosting — was the only real tradeoff, and fluency in both removes it.

## D3 — Hub-and-spoke connector architecture

**Decision:** One connector per provider (read + write) over canonical formats (MIME / iCalendar /
vCard); a provider-agnostic engine pipes source → destination.

**Why:** Migrations are **any provider → any provider**. Per-pair integrations are an N×N explosion;
connectors make it N. The interchange formats are already universal standards, so the canonical model
is essentially free. Adding a provider later works with every existing one, both directions.

## D4 — Start with the Generic IMAP/DAV connector

**Decision:** MVP targets standard IMAP/DAV hosts first, not Google/M365.

**Why:** It needs **no third-party app registration**, it's the real recurring use case, and it avoids
the OAuth-verification swamp. Google and M365 connectors come after the core works.

## D5 — Copy-only; verify everything

**Decision:** Copy-only, idempotent, checkpointed, with a post-migration verification report.

**Why:** Data integrity must be structural, not hopeful — copy-only, resumable, and verified so
nothing is silently dropped, and the source is never touched.
