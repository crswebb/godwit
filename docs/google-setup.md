# Google (Gmail) setup — one-time, by the developer

Godwit signs in to Google with OAuth 2.0 (PKCE, public desktop client) and then
uses **Gmail over IMAP (XOAUTH2)** for mail and **Google CalDAV/CardDAV** for
calendar/contacts. You register an OAuth client once; users never see it.

## Heads-up on `gcloud`

Unlike Microsoft's Entra app (which `az` can create end-to-end), **Google does
not expose full OAuth-client creation through `gcloud`** — the consent screen and
the OAuth *client ID* are Console tasks. `gcloud` can enable APIs and manage the
project, but plan on doing the OAuth bits in the Cloud Console.

## Steps (Google Cloud Console)

1. **Project:** create or pick one at <https://console.cloud.google.com>.
2. **OAuth consent screen** → **External**. Fill the basics.
3. **Scopes** — add:
   - `https://mail.google.com/`  *(Gmail via IMAP — a **restricted** scope)*
   - `https://www.googleapis.com/auth/calendar`  *(CalDAV)*
   - `https://www.googleapis.com/auth/carddav`  *(CardDAV)*
   - `openid`, `email`
4. **Test users** — while the app is in **Testing**, add the accounts you'll
   migrate (up to **100**, no assessment needed).
5. **Credentials → Create credentials → OAuth client ID → Application type:
   Desktop app.** Copy the **Client ID** (the secret is unused — PKCE is a public
   client).
6. Put the Client ID into Godwit: set `GOOGLE_CLIENT_ID` in
   `src/lib/AccountEntry.svelte` (or hand it to Claude to fill in).

## The restricted-scope catch

`https://mail.google.com/` is a **restricted scope**. To ship to the public,
Google requires a **CASA security assessment** (slow, can cost money). But
**Testing mode covers up to 100 users with no assessment** — plenty to serve
client requests and validate demand first. Verification only gates broad public
distribution.

## Notes

- Gmail's **All Mail** contains every message (Gmail uses labels, not folders),
  so migrating it *and* the label-folders duplicates messages. Leave
  `[Gmail]/All Mail` **unchecked** in the plan when migrating out of Gmail.
- 2-step verification and the loopback redirect (`http://localhost:<port>`) are
  handled automatically by the desktop OAuth flow.
