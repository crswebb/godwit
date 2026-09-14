<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import AccountEntry, { type Account, blankAccount } from "./lib/AccountEntry.svelte";

  type FolderInfo = { name: string; delimiter: string | null; attributes: string[] };
  type DavCollection = { name: string; href: string; count: number | null };
  type ImapProbe = { status: string; host: string | null; port: number | null; folders: FolderInfo[]; error: string | null };
  type DavProbe = { status: string; collections: DavCollection[]; error: string | null };
  type Probe = { imap: ImapProbe; calendars: DavProbe; contacts: DavProbe };

  type FolderReport = { folder: string; sourceTotal: number; copied: number; skipped: number; failed: number; missing: number };
  type NamedDavReport = { name: string; total: number; copied: number; skipped: number; failed: number; missing: number };
  type UnifiedReport = { folders: FolderReport[]; calendars: NamedDavReport[]; contacts: NamedDavReport[] };

  type Progress =
    | { type: "started"; folders: number }
    | { type: "folderStart"; name: string; index: number; folders: number; sourceTotal: number }
    | { type: "tick"; folder: string; done: number; total: number }
    | { type: "folderDone"; report: FolderReport }
    | { type: "phase"; label: string }
    | { type: "warning"; message: string };

  let source = $state<Account>(blankAccount());
  let destination = $state<Account>(blankAccount());

  let probing = $state<"source" | "dest" | "both" | null>(null);
  let sourceProbe = $state<Probe | null>(null);
  let destProbe = $state<Probe | null>(null);

  let folderChecked = $state<Record<string, boolean>>({});
  let calChecked = $state<Record<string, boolean>>({});
  let abChecked = $state<Record<string, boolean>>({});

  let dryRun = $state(true);
  let running = $state(false);
  let error = $state<string | null>(null);
  let report = $state<UnifiedReport | null>(null);

  let current = $state<{ name: string; done: number; total: number } | null>(null);
  let phase = $state<string | null>(null);
  let warnings = $state<string[]>([]);

  const canConnect = $derived(
    source.email.trim() !== "" && source.password !== "" &&
      destination.email.trim() !== "" && destination.password !== "" && !probing && !running,
  );

  // Same address on both sides = a domain move between providers. Autodiscovery
  // keys off the domain, so it can't tell the two servers apart — the user must
  // set them explicitly.
  const sameEmail = $derived(
    source.provider === "password" && destination.provider === "password" &&
      source.email.trim() !== "" &&
      source.email.trim().toLowerCase() === destination.email.trim().toLowerCase(),
  );

  // CalDAV/CardDAV migration only works when neither side is Microsoft 365
  // (Graph JSON <-> ICS/vCard conversion isn't built yet).
  const davSupported = $derived(source.provider !== "microsoft" && destination.provider !== "microsoft");

  function normalize(a: Account): Account {
    const clean = (s: string | null) => (s && s.trim() !== "" ? s.trim() : null);
    return { ...a, imapHost: clean(a.imapHost), davUrl: clean(a.davUrl), imapPort: a.imapPort || null };
  }

  const destMatch = (list: DavCollection[] | undefined, name: string) =>
    list?.find((c) => c.name === name)?.href ?? null;

  // Is there anything discovered on the source worth showing a plan for?
  function anyDiscovered(): boolean {
    return !!(
      sourceProbe &&
      (sourceProbe.imap.folders.length ||
        sourceProbe.calendars.collections.length ||
        sourceProbe.contacts.collections.length)
    );
  }

  function syncHost(account: Account, probe: Probe) {
    if (probe.imap.status === "ok") { account.imapHost = probe.imap.host; account.imapPort = probe.imap.port; }
  }

  function initSelection() {
    if (!sourceProbe || !destProbe) return;
    folderChecked = {};
    for (const f of sourceProbe.imap.folders) folderChecked[f.name] = true;
    calChecked = {};
    for (const c of sourceProbe.calendars.collections) calChecked[c.href] = davSupported && destMatch(destProbe.calendars.collections, c.name) !== null;
    abChecked = {};
    for (const c of sourceProbe.contacts.collections) abChecked[c.href] = davSupported && destMatch(destProbe.contacts.collections, c.name) !== null;
  }

  async function connect() {
    probing = "both";
    error = null;
    report = null;
    try {
      const [sp, dp] = await Promise.all([
        invoke<Probe>("probe", { account: normalize(source) }),
        invoke<Probe>("probe", { account: normalize(destination) }),
      ]);
      sourceProbe = sp; syncHost(source, sp);
      destProbe = dp; syncHost(destination, dp);
      initSelection();
    } catch (err) {
      error = typeof err === "string" ? err : String(err);
    } finally {
      probing = null;
    }
  }

  async function retry(side: "source" | "dest") {
    probing = side;
    error = null;
    try {
      const acc = side === "source" ? source : destination;
      const p = await invoke<Probe>("probe", { account: normalize(acc) });
      if (side === "source") { sourceProbe = p; syncHost(source, p); }
      else { destProbe = p; syncHost(destination, p); }
      initSelection();
    } catch (err) {
      error = typeof err === "string" ? err : String(err);
    } finally {
      probing = null;
    }
  }

  const selectionCount = $derived.by(() => {
    if (!sourceProbe || !destProbe) return 0;
    let n = sourceProbe.imap.folders.filter((f) => folderChecked[f.name]).length;
    n += sourceProbe.calendars.collections.filter((c) => calChecked[c.href] && destMatch(destProbe!.calendars.collections, c.name)).length;
    n += sourceProbe.contacts.collections.filter((c) => abChecked[c.href] && destMatch(destProbe!.contacts.collections, c.name)).length;
    return n;
  });

  $effect(() => {
    const unlisten = listen<Progress>("migration://progress", (event) => {
      const p = event.payload;
      switch (p.type) {
        case "folderStart": current = { name: p.name, done: 0, total: p.sourceTotal }; phase = `Email: ${p.name}`; break;
        case "tick": if (current && current.name === p.folder) current = { ...current, done: p.done, total: p.total }; break;
        case "folderDone": current = null; break;
        case "phase": phase = p.label; current = null; break;
        case "warning": warnings = [...warnings, p.message]; break;
      }
    });
    return () => { unlisten.then((f) => f()); };
  });

  async function migrate() {
    if (!sourceProbe || !destProbe) return;
    running = true; error = null; report = null; warnings = []; current = null; phase = null;

    const folders = sourceProbe.imap.folders.filter((f) => folderChecked[f.name]).map((f) => f.name);
    const pairs = (src: DavCollection[], dst: DavCollection[], checked: Record<string, boolean>) =>
      src.filter((c) => checked[c.href])
        .map((c) => ({ name: c.name, source: c.href, dest: destMatch(dst, c.name) }))
        .filter((p): p is { name: string; source: string; dest: string } => p.dest !== null);

    const selection = {
      folders,
      calendars: pairs(sourceProbe.calendars.collections, destProbe.calendars.collections, calChecked),
      contacts: pairs(sourceProbe.contacts.collections, destProbe.contacts.collections, abChecked),
    };

    try {
      report = await invoke<UnifiedReport>("run_migration", {
        source: normalize(source), destination: normalize(destination), selection, dryRun,
      });
    } catch (err) {
      error = typeof err === "string" ? err : String(err);
    } finally {
      running = false; current = null; phase = null;
    }
  }
</script>

<main>
  <header>
    <h1>Godwit</h1>
    <p class="tagline">
      Move email, calendars, and contacts between two accounts. Enter both, connect, and pick what to
      bring across. The source is only ever read.
    </p>
  </header>

  <div class="accounts">
    <AccountEntry label="Source" bind:account={source} disabled={!!probing || running} />
    <div class="arrow" aria-hidden="true">→</div>
    <AccountEntry label="Destination" bind:account={destination} disabled={!!probing || running} />
  </div>

  <div class="connect-row">
    <button onclick={connect} disabled={!canConnect}>
      {probing === "both" ? "Connecting…" : sourceProbe ? "Reconnect" : "Connect"}
    </button>
  </div>

  {#if error}
    <div class="error" role="alert"><strong>Error.</strong> <span>{error}</span></div>
  {/if}

  {#if sourceProbe && destProbe}
    <section class="found">
      <h2>What we found</h2>
      {#if sameEmail}
        <p class="callout">
          Both accounts use the same address — this looks like a domain move between providers.
          Autodiscovery can't tell the two servers apart, so set each mail server below and press
          <em>Try again</em>.
        </p>
      {/if}
      <div class="cards">
        {@render statusCard("source", source, sourceProbe)}
        {@render statusCard("dest", destination, destProbe)}
      </div>
    </section>

    {#if selectionCount > 0 || anyDiscovered()}
      <section class="plan">
        <h2>Choose what to migrate</h2>

        <div class="group">
          <h3>Email folders</h3>
          {#if sourceProbe.imap.status === "ok" && sourceProbe.imap.folders.length}
            <ul class="checklist">
              {#each sourceProbe.imap.folders as f (f.name)}
                <li><label><input type="checkbox" bind:checked={folderChecked[f.name]} disabled={running} /> {f.name}</label></li>
              {/each}
            </ul>
            {#if destProbe.imap.status !== "ok"}
              <p class="muted note">Destination email isn't ready — add its mail server above to include email.</p>
            {/if}
          {:else}
            <p class="muted">Nothing to migrate here.</p>
          {/if}
        </div>

        {@render davGroup("Calendars", sourceProbe.calendars, destProbe.calendars, calChecked)}
        {@render davGroup("Address books", sourceProbe.contacts, destProbe.contacts, abChecked)}
      </section>

      <div class="controls">
        <label class="dry">
          <input type="checkbox" bind:checked={dryRun} disabled={running} />
          <span><strong>Dry run</strong> — count what would copy, write nothing.</span>
        </label>
        <button onclick={migrate} disabled={selectionCount === 0 || running}>
          {running ? "Migrating…" : dryRun ? `Preview (${selectionCount})` : `Migrate (${selectionCount})`}
        </button>
      </div>
    {/if}
  {/if}

  {#if running || report}
    <section class="status">
      {#if current}
        <div class="current">
          <div class="current-head"><span class="folder">{current.name}</span><span class="muted">{current.done} / {current.total}</span></div>
          <div class="bar"><div class="fill" style="width: {current.total ? (current.done / current.total) * 100 : 0}%"></div></div>
        </div>
      {:else if phase && running}
        <p class="muted">{phase}…</p>
      {/if}

      {#if report}
        {#if report.folders.length}
          <table>
            <thead><tr><th>Folder</th><th>Source</th><th>{dryRun ? "Would" : "Copied"}</th><th>Skipped</th><th>Failed</th></tr></thead>
            <tbody>
              {#each report.folders as f (f.folder)}
                <tr><td class="folder">{f.folder}</td><td>{f.sourceTotal}</td><td>{f.copied}</td><td>{f.skipped}</td><td class:bad={f.failed > 0 || f.missing > 0}>{f.failed + f.missing}</td></tr>
              {/each}
            </tbody>
          </table>
        {/if}
        {#each [...report.calendars, ...report.contacts] as r (r.name)}
          <div class="dav-line">
            <span class="name">{r.name}</span>
            <span class="muted">
              {dryRun ? `${r.copied} would copy` : `${r.copied} copied`} · {r.skipped} already there
              {#if r.failed || r.missing}· <span class="bad">{r.failed + r.missing} issue(s)</span>{/if}
            </span>
          </div>
        {/each}
        <div class="summary"><strong>{dryRun ? "Preview complete" : "Migration complete"}</strong></div>
      {/if}

      {#if warnings.length}
        <details class="warnings">
          <summary>{warnings.length} warning{warnings.length === 1 ? "" : "s"}</summary>
          <ul>{#each warnings as w, i (i)}<li>{w}</li>{/each}</ul>
        </details>
      {/if}
    </section>
  {/if}

</main>

{#snippet statusCard(side: "source" | "dest", account: Account, probe: Probe)}
  {@const busy = probing === side}
  <div class="card">
    <div class="card-head">{account.email}</div>
    <ul class="status-list">
      <li>
        <span class="lbl">Email</span>
        {#if probe.imap.status === "ok"}
          <span class="ok">✓ {probe.imap.folders.length} folder{probe.imap.folders.length === 1 ? "" : "s"}</span>
        {:else if probe.imap.status === "needsHost"}
          <span class="miss">couldn't find the mail server</span>
        {:else}
          <span class="miss">error: {probe.imap.error}</span>
        {/if}
      </li>
      <li>
        <span class="lbl">Calendars</span>
        {#if probe.calendars.status === "ok"}
          <span class="ok">✓ {probe.calendars.collections.length}</span>
        {:else}
          <span class="miss">not available</span>
        {/if}
      </li>
      <li>
        <span class="lbl">Contacts</span>
        {#if probe.contacts.status === "ok"}
          <span class="ok">✓ {probe.contacts.collections.length}</span>
        {:else}
          <span class="miss">not available</span>
        {/if}
      </li>
    </ul>

    {#if account.provider === "password"}
    <details class="servers" open={probe.imap.status !== "ok" || sameEmail}>
      <summary>Adjust servers</summary>
      <div class="fixers">
        <p class="fix-hint">
          Detected automatically where possible. Edit and press <em>Try again</em> to point at a
          specific server — leave a line blank to skip that part.
        </p>
        <label class="f">
          <span>Mail server (IMAP)</span>
          <input type="text" bind:value={account.imapHost} placeholder="imap.example.com" spellcheck="false" autocomplete="off" />
        </label>
        <label class="f">
          <span>Calendar / contacts URL</span>
          <input type="text" bind:value={account.davUrl} placeholder="https://caldav.example.com/" spellcheck="false" autocomplete="off" />
        </label>
        <button class="try" onclick={() => retry(side)} disabled={busy || running}>{busy ? "Trying…" : "Try again"}</button>
      </div>
    </details>
    {/if}
  </div>
{/snippet}

{#snippet davGroup(title: string, srcProbe: DavProbe, dstProbe: DavProbe, checked: Record<string, boolean>)}
  <div class="group">
    <h3>{title}</h3>
    {#if !davSupported}
      <p class="muted">Not available yet when a Microsoft 365 account is involved.</p>
    {:else if srcProbe.status === "ok" && srcProbe.collections.length}
      <ul class="checklist">
        {#each srcProbe.collections as c (c.href)}
          {@const dest = dstProbe.collections.find((d) => d.name === c.name)?.href ?? null}
          <li>
            <label class:disabled={!dest}>
              <input type="checkbox" bind:checked={checked[c.href]} disabled={running || !dest} />
              {c.name}{c.count === null ? "" : ` (${c.count})`}
              {#if dest}<span class="arrow-to">→ {c.name}</span>{:else}<span class="muted small">no match on destination</span>{/if}
            </label>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted">Nothing to migrate here.</p>
    {/if}
  </div>
{/snippet}

<style>
  main { max-width: 820px; margin: 0 auto; padding: 28px 20px 56px; }
  header h1 { margin: 0; font-size: 1.9rem; letter-spacing: -0.02em; }
  .tagline { margin: 4px 0 0; color: var(--muted); font-size: 0.9rem; max-width: 64ch; }

  .accounts { margin-top: 22px; display: flex; align-items: flex-start; gap: 12px; }
  .arrow { color: var(--muted); font-size: 1.4rem; flex: none; align-self: center; }
  .connect-row { margin-top: 14px; }

  button {
    padding: 10px 18px; border: none; border-radius: 9px;
    background: var(--accent); color: var(--accent-ink); font-weight: 600; cursor: pointer;
  }
  button:disabled { opacity: 0.5; cursor: not-allowed; }

  .error { margin-top: 16px; padding: 12px 14px; border-radius: 10px; background: var(--danger-bg); color: var(--danger); font-size: 0.9rem; }

  .found { margin-top: 26px; }
  .found h2, .plan h2 { font-size: 1.15rem; margin: 0 0 10px; }
  .cards { display: flex; gap: 12px; }
  .card { flex: 1; min-width: 0; border: 1px solid var(--line); border-radius: 12px; padding: 14px; background: var(--card); }
  .card-head { font-weight: 600; font-size: 0.85rem; margin-bottom: 8px; word-break: break-all; }
  .status-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 5px; }
  .status-list li { display: flex; justify-content: space-between; font-size: 0.85rem; }
  .lbl { color: var(--muted); }
  .ok { color: var(--accent); font-weight: 600; }
  .miss { color: var(--muted); }

  .callout { margin: 0 0 12px; padding: 11px 13px; border-radius: 10px; background: color-mix(in srgb, var(--accent) 14%, transparent); font-size: 0.85rem; }
  .callout em { font-style: normal; font-weight: 600; }

  .servers { margin-top: 12px; border-top: 1px solid var(--line); padding-top: 10px; }
  .servers summary { cursor: pointer; font-size: 0.8rem; color: var(--muted); }
  .fixers { display: flex; flex-direction: column; gap: 8px; margin-top: 10px; }
  .fix-hint { margin: 0; font-size: 0.78rem; color: var(--muted); }
  .fixers .f { display: flex; flex-direction: column; gap: 4px; font-size: 0.78rem; color: var(--muted); }
  .fixers input {
    padding: 7px 9px; border: 1px solid var(--line); border-radius: 8px; background: var(--bg); color: var(--ink); font-size: 0.82rem;
  }
  .fixers input:focus { outline: 2px solid var(--accent); outline-offset: -1px; }
  .try { align-self: flex-start; padding: 7px 13px; font-size: 0.85rem; }

  .plan { margin-top: 26px; }
  .group { margin-bottom: 18px; }
  .group h3 { font-size: 0.95rem; margin: 0 0 8px; }

  .checklist { list-style: none; margin: 0; padding: 0; border: 1px solid var(--line); border-radius: 10px; overflow: hidden; }
  .checklist li { padding: 8px 12px; background: var(--card); }
  .checklist li:not(:last-child) { border-bottom: 1px solid var(--line); }
  .checklist label { display: flex; align-items: center; gap: 9px; font-size: 0.88rem; cursor: pointer; }
  .checklist label.disabled { cursor: default; }
  .arrow-to { color: var(--muted); font-size: 0.8rem; margin-left: auto; }

  .muted { color: var(--muted); }
  .small { font-size: 0.78rem; margin-left: auto; }
  .note { font-size: 0.82rem; margin-top: 8px; }

  .controls { margin-top: 10px; display: flex; align-items: center; justify-content: space-between; gap: 16px; flex-wrap: wrap; }
  .dry { display: flex; align-items: flex-start; gap: 9px; font-size: 0.85rem; color: var(--muted); }

  .status { margin-top: 26px; display: flex; flex-direction: column; gap: 14px; }
  .current-head { display: flex; justify-content: space-between; font-size: 0.9rem; margin-bottom: 6px; }
  .current .folder { font-weight: 600; }
  .bar { height: 8px; border-radius: 999px; background: var(--line); overflow: hidden; }
  .fill { height: 100%; background: var(--accent); transition: width 0.2s ease; }

  table { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
  th, td { text-align: right; padding: 7px 10px; border-bottom: 1px solid var(--line); }
  th:first-child, td.folder { text-align: left; }
  th { color: var(--muted); font-weight: 600; }
  td.bad, .bad { color: var(--danger); font-weight: 600; }

  .dav-line { display: flex; justify-content: space-between; gap: 12px; font-size: 0.86rem; padding: 6px 2px; border-bottom: 1px solid var(--line); }
  .dav-line .name { font-weight: 500; }

  .summary { padding: 11px 13px; border-radius: 9px; background: color-mix(in srgb, var(--accent) 16%, transparent); font-size: 0.9rem; }

  .warnings { font-size: 0.83rem; color: var(--muted); }
  .warnings ul { margin: 8px 0 0; padding-left: 18px; }
  .warnings li { margin-bottom: 3px; word-break: break-word; }
</style>
