<script lang="ts">
  import AccountEntry from "./lib/AccountEntry.svelte";
  import DiscoveryCard from "./lib/DiscoveryCard.svelte";
  import CollectionGroup from "./lib/CollectionGroup.svelte";
  import MigrationProgress from "./lib/MigrationProgress.svelte";
  import Settings from "./lib/Settings.svelte";
  import { blankAccount } from "./lib/types";
  import type { Account, Current, DavCollection, Probe, UnifiedReport } from "./lib/types";
  import { errorText, onProgress, probeAccount, runMigration } from "./lib/api";

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

  let current = $state<Current | null>(null);
  let warnings = $state<string[]>([]);

  function accountReady(a: Account): boolean {
    return a.provider === "microsoft" ? a.email.trim() !== "" : a.email.trim() !== "" && a.password !== "";
  }
  const canConnect = $derived(accountReady(source) && accountReady(destination) && !probing && !running);

  // Same address on both sides = a domain move; autodiscovery can't tell the
  // two servers apart, so the user must set them explicitly.
  const sameEmail = $derived(
    source.provider === "password" && destination.provider === "password" &&
      source.email.trim() !== "" &&
      source.email.trim().toLowerCase() === destination.email.trim().toLowerCase(),
  );

  const destMatch = (list: DavCollection[] | undefined, name: string) =>
    list?.find((c) => c.name === name)?.href ?? null;

  const anyDiscovered = $derived(
    !!(sourceProbe &&
      (sourceProbe.imap.folders.length ||
        sourceProbe.calendars.collections.length ||
        sourceProbe.contacts.collections.length)),
  );

  function syncHost(a: Account, p: Probe) {
    if (p.imap.status === "ok") { a.imapHost = p.imap.host; a.imapPort = p.imap.port; }
  }

  function initSelection() {
    if (!sourceProbe || !destProbe) return;
    folderChecked = {};
    for (const f of sourceProbe.imap.folders) folderChecked[f.name] = true;
    calChecked = {};
    for (const c of sourceProbe.calendars.collections) calChecked[c.href] = destMatch(destProbe.calendars.collections, c.name) !== null;
    abChecked = {};
    for (const c of sourceProbe.contacts.collections) abChecked[c.href] = destMatch(destProbe.contacts.collections, c.name) !== null;
  }

  async function connect() {
    probing = "both";
    error = null;
    report = null;
    try {
      const [sp, dp] = await Promise.all([probeAccount(source), probeAccount(destination)]);
      sourceProbe = sp; syncHost(source, sp);
      destProbe = dp; syncHost(destination, dp);
      initSelection();
    } catch (err) {
      error = errorText(err);
    } finally {
      probing = null;
    }
  }

  async function retry(side: "source" | "dest") {
    probing = side;
    error = null;
    try {
      const acc = side === "source" ? source : destination;
      const p = await probeAccount(acc);
      if (side === "source") { sourceProbe = p; syncHost(source, p); }
      else { destProbe = p; syncHost(destination, p); }
      initSelection();
    } catch (err) {
      error = errorText(err);
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

  $effect(() =>
    onProgress((p) => {
      switch (p.type) {
        case "folderStart": current = { name: p.name, done: 0, total: p.sourceTotal }; break;
        case "tick": if (current && current.name === p.folder) current = { ...current, done: p.done, total: p.total }; break;
        case "folderDone": current = null; break;
        case "warning": warnings = [...warnings, p.message]; break;
      }
    }),
  );

  async function migrate() {
    if (!sourceProbe || !destProbe) return;
    running = true; error = null; report = null; warnings = []; current = null;

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
      report = await runMigration(source, destination, selection, dryRun);
    } catch (err) {
      error = errorText(err);
    } finally {
      running = false; current = null;
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
        <DiscoveryCard bind:account={source} probe={sourceProbe} busy={probing === "source"} {sameEmail} {running} onRetry={() => retry("source")} />
        <DiscoveryCard bind:account={destination} probe={destProbe} busy={probing === "dest"} {sameEmail} {running} onRetry={() => retry("dest")} />
      </div>
    </section>

    {#if selectionCount > 0 || anyDiscovered}
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

        <CollectionGroup title="Calendars" source={sourceProbe.calendars} dest={destProbe.calendars} bind:checked={calChecked} {running} />
        <CollectionGroup title="Address books" source={sourceProbe.contacts} dest={destProbe.contacts} bind:checked={abChecked} {running} />
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
    <MigrationProgress {current} {report} {dryRun} {warnings} />
  {/if}

  <details class="settings-section">
    <summary>Settings</summary>
    <Settings />
  </details>
</main>

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

  .callout { margin: 0 0 12px; padding: 11px 13px; border-radius: 10px; background: color-mix(in srgb, var(--accent) 14%, transparent); font-size: 0.85rem; }
  .callout em { font-style: normal; font-weight: 600; }

  .plan { margin-top: 26px; }
  .group { margin-bottom: 18px; }
  .group h3 { font-size: 0.95rem; margin: 0 0 8px; }
  .checklist { list-style: none; margin: 0; padding: 0; border: 1px solid var(--line); border-radius: 10px; overflow: hidden; }
  .checklist li { padding: 8px 12px; background: var(--card); }
  .checklist li:not(:last-child) { border-bottom: 1px solid var(--line); }
  .checklist label { display: flex; align-items: center; gap: 9px; font-size: 0.88rem; cursor: pointer; }

  .muted { color: var(--muted); }
  .note { font-size: 0.82rem; margin-top: 8px; }

  .controls { margin-top: 10px; display: flex; align-items: center; justify-content: space-between; gap: 16px; flex-wrap: wrap; }
  .dry { display: flex; align-items: flex-start; gap: 9px; font-size: 0.85rem; color: var(--muted); }

  .settings-section { margin-top: 34px; border-top: 1px solid var(--line); padding-top: 16px; }
  .settings-section summary { cursor: pointer; font-weight: 600; font-size: 0.92rem; color: var(--muted); }
</style>
