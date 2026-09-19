# Google (Gmail) setup — bring your own client ID

Godwit signs in to Google with OAuth 2.0 (PKCE, public desktop client), then uses
**Gmail over IMAP (XOAUTH2)** for mail and **Google CalDAV/CardDAV** for
calendar/contacts.

Godwit ships **no** Google client ID: Gmail's scope is *restricted*, so a shared
client would either cap at 100 users or require the maintainer to pay for a CASA
security assessment. Instead — like rclone and DAVx⁵ — **you create your own
Google OAuth client** (5 minutes, free). Because you're the only user of your own
app, the 100-user cap and CASA simply don't apply to you.

**Prefer no setup at all?** Use an **app-specific password** instead: enable
2-step verification on the Google account, generate an app password, and use
Godwit's plain email + password with `imap.gmail.com`. No client ID needed.

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
6. Put the Client ID into Godwit: open **Settings** in the app and paste it into
   **Google client ID**. It's stored locally on your device.

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
