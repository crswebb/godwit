# JMAP setup

Godwit can use **JMAP** as a source or destination for **email** (RFC 8620 +
RFC 8621) and **contacts** (RFC 9610). JMAP is the modern HTTP/JSON successor to
IMAP, spoken by [Fastmail](https://www.fastmail.com), [Stalwart](https://stalw.art),
recent Cyrus, and Apache James.

> **No calendars yet.** JMAP for Calendars is still an IETF draft, not a
> published standard, so Godwit doesn't migrate calendars over JMAP. Migrate
> calendars for the same account over CalDAV using the **email & password**
> option. Contacts over JMAP are converted to/from vCard via JSContact
> (RFC 9553 / RFC 9555).

## What you need

1. **Your email address** (e.g. `you@fastmail.com`).
2. **An API token** for your JMAP account, used as the password. Godwit sends it
   as an OAuth-style bearer token — the credential never leaves your machine.
3. Optionally, a **JMAP server URL**. Leave it blank and Godwit discovers the
   server from your email domain via `https://<domain>/.well-known/jmap`
   (RFC 8620 §2.2). Set it only if autodiscovery can't reach your server.

### Fastmail

Create a token at **Settings → Privacy & Security → Integrations →
New API token**, scoped to **Mail** and **Contacts** (read for a source,
read/write for a destination). Paste it into the **API token** field.
Autodiscovery works from any `@fastmail.com` address and from custom domains
hosted at Fastmail.

### Stalwart / self-hosted

Create an API access token (or use an app password that your server accepts as a
bearer token). If your mail domain doesn't serve `/.well-known/jmap`, enter the
session URL explicitly, e.g. `https://mail.example.com/.well-known/jmap` or your
server's JMAP session endpoint.

## How it maps

| JMAP | Godwit |
| --- | --- |
| Message (raw RFC 5322 blob) | canonical MIME item — bytes copied verbatim |
| `Message-ID` header | dedup key (same as IMAP, so dedup works across protocols) |
| Keywords `$seen` / `$answered` / `$flagged` / `$draft` | read / answered / flagged / draft flags |
| `Mailbox` name + role (inbox, sent, drafts, trash, junk, archive) | folder, mapped to the destination's matching folder |
| `receivedAt` | internal date |
| `AddressBook` | address book (collection), matched to the destination by name |
| `ContactCard` (JSContact) | vCard, converted per RFC 9555 |

Reading uses `Email/query` + `Email/get` + blob download (mail) and
`ContactCard/query` + `ContactCard/get` (contacts); writing uploads the MIME
blob and calls `Email/import` (mail) or `ContactCard/set` (contacts). The
source is only ever read.

## Limitations

- **No calendars** (see above) — use CalDAV for those.
- Contacts convert through JSContact↔vCard (RFC 9555). Core fields (name,
  emails, phones, organizations, titles, addresses, notes, nicknames, birthday)
  are mapped; rare or vendor-specific JSContact properties are passed over
  rather than guessed at.
- When the same message is copied into two folders, JMAP stores one copy and
  the second folder is added to its mailbox memberships (JMAP mail is
  deduplicated account-wide, unlike IMAP's per-folder copies).
- **Localized special-use source folders** may not map onto the destination's
  standard folders. Folder mapping matches on folder *name*, not JMAP role, so a
  source mailbox named e.g. `Posteingang` (role `inbox`) copies into a folder of
  that name rather than the destination's INBOX. (Shared with the IMAP
  connector; a role-aware planner is a planned follow-up.)
- **A contact in several selected address books** is created in the first; if
  the server enforces account-wide unique contact UIDs (RFC 9610), the duplicate
  into the second book is reported as failed. Select one address book per
  contact to avoid this.
- Autodiscovery expects the standard `.well-known/jmap` resource; otherwise set
  the server URL by hand.
