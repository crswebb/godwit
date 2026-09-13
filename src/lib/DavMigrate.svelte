<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import DavPicker, { type DavAccount, type DavKind } from "./DavPicker.svelte";

  type DavReport = {
    total: number;
    copied: number;
    skipped: number;
    failed: number;
    missing: number;
  };

  let kind = $state<DavKind>("calendar");
  let source = $state<DavAccount>({ url: null, username: "", password: "" });
  let sourceHref = $state<string | null>(null);
  let destination = $state<DavAccount>({ url: null, username: "", password: "" });
  let destHref = $state<string | null>(null);
  let dryRun = $state(true);

  let running = $state(false);
  let error = $state<string | null>(null);
  let report = $state<DavReport | null>(null);

  const canMigrate = $derived(!!sourceHref && !!destHref && !running);

  async function migrate() {
    running = true;
    error = null;
    report = null;
    try {
      report = await invoke<DavReport>("dav_migrate", {
        source,
        sourceCollection: sourceHref,
        destination,
        destCollection: destHref,
        kind,
        dryRun,
      });
    } catch (err) {
      error = typeof err === "string" ? err : String(err);
    } finally {
      running = false;
    }
  }
</script>

<div class="dav">
  <p class="hint">
    Copy a calendar or address book between two CalDAV/CardDAV accounts. Enter each account's email +
    password and press Discover; if autodiscovery can't find the server, add its URL under Advanced.
    Read-only on the source.
  </p>

  <label class="kind">
    <span>Data type</span>
    <select bind:value={kind} disabled={running}>
      <option value="calendar">Calendars</option>
      <option value="contacts">Address books</option>
    </select>
  </label>

  <div class="sides">
    <DavPicker label="Source" {kind} bind:account={source} bind:selectedHref={sourceHref} disabled={running} />
    <div class="arrow" aria-hidden="true">→</div>
    <DavPicker label="Destination" {kind} bind:account={destination} bind:selectedHref={destHref} disabled={running} />
  </div>

  <div class="controls">
    <label class="dry">
      <input type="checkbox" bind:checked={dryRun} disabled={running} />
      <span><strong>Dry run</strong> — count what would copy, write nothing.</span>
    </label>
    <button type="button" onclick={migrate} disabled={!canMigrate}>
      {running ? "Copying…" : dryRun ? "Preview copy" : "Copy items"}
    </button>
  </div>

  {#if error}
    <div class="error" role="alert">{error}</div>
  {/if}

  {#if report}
    <div class="summary" class:ok={report.failed === 0 && report.missing === 0}>
      <strong>{dryRun ? "Preview complete" : "Copy complete"}</strong>
      <span>
        {#if dryRun}
          {report.copied} would copy · {report.skipped} already there · {report.total} total
        {:else}
          {report.copied} copied · {report.skipped} skipped (already there) · {report.failed}
          failed{report.missing > 0 ? ` · ${report.missing} missing after verify` : " · verified ✓"}
        {/if}
      </span>
    </div>
  {/if}
</div>

<style>
  .dav {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding-top: 6px;
  }

  .hint {
    margin: 0;
    font-size: 0.82rem;
    color: var(--muted);
    max-width: 64ch;
  }

  .kind {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 0.82rem;
    color: var(--muted);
    max-width: 200px;
  }

  .kind select {
    padding: 8px 10px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--bg);
    color: var(--ink);
  }

  .sides {
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }

  .arrow {
    color: var(--muted);
    font-size: 1.4rem;
    flex: none;
    align-self: center;
  }

  .controls {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
  }

  .dry {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    font-size: 0.85rem;
    color: var(--muted);
  }

  button {
    padding: 10px 16px;
    border: none;
    border-radius: 8px;
    background: var(--accent);
    color: var(--accent-ink);
    font-weight: 600;
    cursor: pointer;
    flex: none;
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .error {
    padding: 10px 12px;
    border-radius: 9px;
    background: var(--danger-bg);
    color: var(--danger);
    font-size: 0.85rem;
  }

  .summary {
    padding: 11px 13px;
    border-radius: 9px;
    background: var(--line);
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 0.88rem;
  }

  .summary.ok {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
</style>
