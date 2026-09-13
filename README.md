# Godwit

> **Why "Godwit"?** The bar-tailed godwit holds the record for the **longest non-stop flight of any
> animal** — crossing oceans without landing, carrying everything the whole way without dropping a
> thing. That's the promise: get all of it across, intact.
>
> A privacy-first **desktop app** that migrates **email, calendar, and contacts** between *any*
> mail providers, in *any* direction — without your data or credentials ever touching a server.

## The problem

Switching email providers is genuinely painful. Moving mailboxes, calendars, and contacts between
providers is fiddly and risky, and the existing tools are either enterprise-priced, ugly, or aimed at
IT admins. For small businesses the *fear of losing data* during the switch is the real blocker.

## The approach

A cross-platform **local desktop app** (Windows / macOS / Linux) that:

- Runs the entire migration **on the user's own machine** — nothing is relayed through our servers.
- **Never sees credentials.** They stay on the device (OS keychain); we can't leak what we never hold.
- Works **any provider → any provider** via a **hub-and-spoke connector architecture**
  (see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)).
- Is **copy-only, resumable, and verified** — designed to move everything without silently dropping it.

### Why this shape

| Goal | How it's met |
| --- | --- |
| Cheap to run | No server, no bandwidth cost — the client's machine does the work. |
| Scales infinitely | Zero marginal cost per user. |
| Trustworthy | Credentials & mailbox contents never leave the device. GDPR-clean (EU). |
| Any-to-any | One connector per provider, not one tool per pair. |

## Tech decisions (see `docs/` for rationale)

- **Tauri v2** (Rust core + web UI) — chosen for security posture, data-integrity on long jobs,
  small footprint, and native feel. See [`docs/DECISIONS.md`](docs/DECISIONS.md).
- **Connector architecture** with standard interchange formats (MIME / iCalendar / vCard).

## Status

**Pre-MVP — spec phase (2026-09-13).**
Next: verify CalDAV/CardDAV support on the first source/destination pair, then build the Generic
IMAP/DAV connector.

## Docs

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — system design, connector model, data flow, security.
- [`docs/MVP-SPEC.md`](docs/MVP-SPEC.md) — phased scope, auth matrix, first-migration test plan.
- [`docs/DECISIONS.md`](docs/DECISIONS.md) — key decisions and their rationale.
- [`docs/architecture-diagram.html`](docs/architecture-diagram.html) — rendered diagrams for quick viewing.
