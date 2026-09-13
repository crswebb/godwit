# Provider: <name>

_Checked: <date> · Webmail software: <Roundcube / Horde / SOGo / other>_

> ⚠️ Endpoints and auth *style* only — never commit real credentials.

## Email (IMAP)

| Field | Value |
| --- | --- |
| IMAP host | |
| Port | 993 (implicit TLS) / 143 (STARTTLS) |
| TLS | implicit / STARTTLS |
| App-specific password required? | yes / no |
| Notes | |

## Calendar (CalDAV)

| Field | Value |
| --- | --- |
| CalDAV supported? | yes / no |
| CalDAV URL | (try `https://<host>/.well-known/caldav`) |
| Auth | same mail password / separate |
| Fallback if no CalDAV | webmail can export **.ics**? yes / no |
| Notes | |

## Contacts (CardDAV)

| Field | Value |
| --- | --- |
| CardDAV supported? | yes / no |
| CardDAV URL | (try `https://<host>/.well-known/carddav`) |
| Auth | same mail password / separate |
| Fallback if no CardDAV | webmail can export **.vcf**? yes / no |
| Notes | |

## Verdict

- [ ] Email path confirmed
- [ ] Calendar path confirmed (DAV or export fallback)
- [ ] Contacts path confirmed (DAV or export fallback)

_Summary: <one line — is this provider fully covered, or does calendar/contacts need the manual fallback?>_
