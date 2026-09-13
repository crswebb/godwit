<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import AccountEntry, { type Account } from "./lib/AccountEntry.svelte";

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

  const blank = (): Account => ({ email: "", password: "", imapHost: null, imapPort: null, davUrl: null });

  let source = $state<Account>(blank());
  let destination = $state<Account>(blank());

  let probing = $state(false);
  let sourceProbe = $state<Probe | null>(null);
  let destProbe = $state<Probe | null>(null);

  // Selection state, keyed by folder name / source collection href.
  let folderChecked = $state<Record<string, boolean>>({});
  let calChecked = $state<Record<string, boolean>>({});
  let abChecked = $state<Record<string, boolean>>({});

  let dryRun = $state(true);
  let running = $state(false);
  let error = $state<string | null>(null);
  let report = $state<UnifiedReport | null>(null);

  // Live progress
  let current = $state<{ name: string; done: number; total: number } | null>(null);
  let phase = $state<string | null>(null);
  let warnings = $state<string[]>([]);

  const canConnect = $derived(
    source.email.trim() !== "" && source.password !== "" &&
      destination.email.trim() !== "" && destination.password !== "" && !probing && !running,
  );

  function normalize(a: Account): Account {
    const clean = (s: string | null) => (s && s.trim() !== "" ? s.trim() : null);
    return { ...a, imapHost: clean(a.imapHost), davUrl: clean(a.davUrl), imapPort: a.imapPort || null };
  }

  function destMatch(list: DavCollection[] | undefined, name: string): string | null {
    return list?.find((c) => c.name === name)?.href ?? null;
  }

  async function connect() {
    probing = true;
    error = null;
    report = null;
    sourceProbe = null;
    destProbe = null;
    try {
      const [sp, dp] = await Promise.all([
        invoke<Probe>("probe", { account: normalize(source) }),
        invoke<Probe>("probe", { account: normalize(destination) }),
      ]);
      sourceProbe = sp;
      destProbe = dp;

      // Remember discovered mail servers so migration uses them directly.
      if (sp.imap.status === "ok") { source.imapHost = sp.imap.host; source.imapPort = sp.imap.port; }
      if (dp.imap.status === "ok") { destination.imapHost = dp.imap.host; destination.imapPort = dp.imap.port; }

      // Default selection: all source folders; calendars/address books that
      // have a matching collection on the destination.
      folderChecked = {};
      for (const f of sp.imap.folders) folderChecked[f.name] = true;
      calChecked = {};
      for (const c of sp.calendars.collections) calChecked[c.href] = destMatch(dp.calendars.collections, c.name) !== null;
      abChecked = {};
      for (const c of sp.contacts.collections) abChecked[c.href] = destMatch(dp.contacts.collections, c.name) !== null;
    } catch (err) {
      error = typeof err === "string" ? err : String(err);
    } finally {
      probing = false;
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
    running = true;
    error = null;
    report = null;
    warnings = [];
    current = null;
    phase = null;

    const folders = sourceProbe.imap.folders.filter((f) => folderChecked[f.name]).map((f) => f.name);
    const pairs = (src: DavCollection[], dst: DavCollection[], checked: Record<string, boolean>) =>
      src
        .filter((c) => checked[c.href])
        .map((c) => ({ name: c.name, source: c.href, dest: destMatch(dst, c.name) }))
        .filter((p): p is { name: string; source: string; dest: string } => p.dest !== null);

    const selection = {
      folders,
      calendars: pairs(sourceProbe.calendars.collections, destProbe.calendars.collections, calChecked),
      contacts: pairs(sourceProbe.contacts.collections, destProbe.contacts.collections, abChecked),
    };

    try {
      report = await invoke<UnifiedReport>("run_migration", {
        source: normalize(source),
        destination: normalize(destination),
        selection,
        dryRun,
      });
    } catch (err) {
      error = typeof err === "string" ? err : String(err);
    } finally {
      running = false;
      current = null;
      phase = null;
    }
  }
</script>

<main>
  <header>
    <h1>Godwit</h1>
    <p class="tagline">
      Move email, calendars, and contacts from one account to another. Enter both accounts, connect,
      then choose what to bring across. The source is only ever read.
    </p>
  </header>

  <div class="accounts">
    <AccountEntry label="Source" bind:account={source} needsHost={sourceProbe?.imap.status === "needsHost"} disabled={probing || running} />
    <div class="arrow" aria-hidden="true">→</div>
    <AccountEntry label="Destination" bind:account={destination} needsHost={destProbe?.imap.status === "needsHost"} disabled={probing || running} />
  </div>

  <div class="connect-row">
    <button onclick={connect} disabled={!canConnect}>
      {probing ? "Connecting…" : sourceProbe ? "Reconnect" : "Connect"}
    </button>
  </div>

  {#if error}
    <div class="error" role="alert"><strong>Error.</strong> <span>{error}</span></div>
  {/if}

  {#if sourceProbe && destProbe}
    {@const sp = sourceProbe}
    {@const dp = destProbe}
    <section class="plan">
      <h2>What to migrate</h2>

      <!-- Email -->
      <div class="group">
        <h3>Email folders</h3>
        {#if sp.imap.status === "ok"}
          {#if sp.imap.folders.length === 0}
            <p class="muted">No folders found.</p>
          {:else}
            <ul class="checklist">
              {#each sp.imap.folders as f (f.name)}
                <li><label><input type="checkbox" bind:checked={folderChecked[f.name]} disabled={running} /> {f.name}</label></li>
              {/each}
            </ul>
            {#if dp.imap.status !== "ok"}
              <p class="muted note">Destination mail server not reachable yet — email won't copy until it is (check Destination → Advanced).</p>
            {/if}
          {/if}
        {:else if sp.imap.status === "needsHost"}
          <p class="muted">Enter the source mail server (Source → Advanced) and reconnect.</p>
        {:else}
          <p class="muted">Couldn't read email: {sp.imap.error}</p>
        {/if}
      </div>

      <!-- Calendars -->
      {@render davGroup("Calendars", sp.calendars, dp.calendars, calChecked)}
      <!-- Address books -->
      {@render davGroup("Address books", sp.contacts, dp.contacts, abChecked)}
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

{#snippet davGroup(title: string, srcProbe: DavProbe, dstProbe: DavProbe, checked: Record<string, boolean>)}
  <div class="group">
    <h3>{title}</h3>
    {#if srcProbe.status === "ok"}
      {#if srcProbe.collections.length === 0}
        <p class="muted">None found.</p>
      {:else}
        <ul class="checklist">
          {#each srcProbe.collections as c (c.href)}
            {@const dest = dstProbe.collections.find((d) => d.name === c.name)?.href ?? null}
            <li>
              <label class:disabled={!dest}>
                <input type="checkbox" bind:checked={checked[c.href]} disabled={running || !dest} />
                {c.name}{c.count === null ? "" : ` (${c.count})`}
                {#if dest}<span class="arrow-to">→ {c.name}</span>{:else}<span class="muted small">no matching item on destination</span>{/if}
              </label>
            </li>
          {/each}
        </ul>
      {/if}
    {:else}
      <p class="muted">Not available for the source account.</p>
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

  .plan { margin-top: 26px; }
  .plan h2 { font-size: 1.15rem; margin: 0 0 10px; }
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
