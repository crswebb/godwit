# Microsoft 365 setup (one-time, by the developer)

Godwit talks to Microsoft 365 through the Microsoft Graph API using OAuth 2.0.
Microsoft requires every app to be registered once. This gives you a **client
ID** (a public identifier — not a secret) that the app uses. You do this once;
your users never see it.

## Register the app in Microsoft Entra

1. Go to <https://entra.microsoft.com> → **Applications → App registrations → New registration**.
2. **Name:** `Godwit` (anything).
3. **Supported account types:** choose based on who you migrate:
   - *Accounts in any organizational directory and personal Microsoft accounts* — broadest (work/school + personal). Recommended.
4. **Redirect URI:** platform **Mobile and desktop applications**, value:
   ```
   http://localhost
   ```
   (Microsoft matches `http://localhost` on any port for desktop PKCE apps, which is what Godwit uses.)
5. Click **Register**. Copy the **Application (client) ID** — you'll paste this into Godwit.

## Allow the public-client (PKCE) flow

6. In the app → **Authentication** → under **Advanced settings**, set **Allow
   public client flows** to **Yes**. (Desktop apps have no client secret.)

## Permissions (Microsoft Graph, delegated)

7. **API permissions → Add a permission → Microsoft Graph → Delegated permissions**, add:
   - `offline_access` (refresh tokens)
   - `User.Read` (read the signed-in user's address)
   - `Mail.ReadWrite`
   - `Calendars.ReadWrite`
   - `Contacts.ReadWrite`
8. These are user-consented at sign-in; no admin consent needed for personal
   accounts. Some organizations require an admin to consent once — if so, click
   **Grant admin consent** (or ask the tenant admin).

## Using it in Godwit

Paste the **client ID** into the Microsoft 365 panel and click **Sign in with
Microsoft**. A browser window opens for you to log in and consent; Godwit
captures the result on a local loopback address. Tokens are kept in memory for
the session (keychain persistence is planned).

## Notes

- **Basic Auth (password IMAP) is disabled by Microsoft** — this OAuth flow is
  the only way in. That's why a password won't work for an M365 mailbox.
- The same registration works for every user you migrate; it identifies the
  *app*, not the account.
