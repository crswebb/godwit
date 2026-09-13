# Provider notes

One file per mail host we migrate to/from, capturing exactly how to reach its
email, calendar, and contacts. Fill these in **before** writing connector code —
the CalDAV/CardDAV support of cheap hosts is the biggest unknown in the project.

- Copy [`TEMPLATE.md`](TEMPLATE.md) to `<provider>.md` (e.g. `example-host.md`).
- Do the checks with a real test account in that host's webmail.
- **Never commit real credentials** — record endpoints, ports, and auth *style* only.

See the checklist in [`../MVP-SPEC.md`](../MVP-SPEC.md#pre-code-research-caldav--carddav-verification-checklist).
