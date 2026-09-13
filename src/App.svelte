<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import AccountForm, { type Creds } from "./lib/AccountForm.svelte";
  import DavMigrate from "./lib/DavMigrate.svelte";

  type FolderReport = {
    folder: string;
    sourceTotal: number;
    copied: number;
    skipped: number;
    failed: number;
    missing: number;
  };
  type MigrationReport = {
    folders: FolderReport[];
    copied: number;
    skipped: number;
    failed: number;
    missing: number;
  };
  type Progress =
    | { type: "started"; folders: number }
    | { type: "folderStart"; name: string; index: number; folders: number; sourceTotal: number }
    | { type: "tick"; folder: string; done: number; total: number }
    | { type: "folderDone"; report: FolderReport }
    | { type: "warning"; message: string }
    | { type: "done"; report: MigrationReport };

  let source = $state<Creds>({ host: "", port: 993, username: "", password: "" });
  let destination = $state<Creds>({ host: "", port: 993, username: "", password: "" });
  let dryRun = $state(true);

  let running = $state(false);
  let error = $state<string | null>(null);
  let report = $state<MigrationReport | null>(null);

  // Live state during a run.
  let totalFolders = $state(0);
  let current = $state<{ name: string; index: number; done: number; total: number } | null>(null);
  let doneFolders = $state<FolderReport[]>([]);
  let warnings = $state<string[]>([]);

  const canRun = $derived(
    source.host.trim() !== "" &&
      source.username.trim() !== "" &&
      source.password !== "" &&
      destination.host.trim() !== "" &&
      destination.username.trim() !== "" &&
      destination.password !== "" &&
      !running,
  );

  $effect(() => {
    const unlisten = listen<Progress>("migration://progress", (event) => {
      const p = event.payload;
      switch (p.type) {
        case "started":
          totalFolders = p.folders;
          break;
        case "folderStart":
          current = { name: p.name, index: p.index, done: 0, total: p.sourceTotal };
          break;
        case "tick":
          if (current && current.name === p.folder) {
            current = { ...current, done: p.done, total: p.total };
          }
          break;
        case "folderDone":
          doneFolders = [...doneFolders, p.report];
          current = null;
          break;
        case "warning":
          warnings = [...warnings, p.message];
          break;
        case "done":
          report = p.report;
          current = null;
          break;
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  });

  async function run() {
    running = true;
    error = null;
    report = null;
    doneFolders = [];
    warnings = [];
    current = null;
    totalFolders = 0;
    try {
      report = await invoke<MigrationReport>("migrate", {
        source,
        destination,
        options: { dryRun },
      });
    } catch (err) {
      error = typeof err === "string" ? err : String(err);
    } finally {
      running = false;
    }
  }
</script>

<main>
  <header>
    <h1>Godwit</h1>
    <p class="tagline">
      Copy email from one account to another — folders, flags, and dates preserved.
      Nothing leaves this machine; the source is only ever read.
    </p>
  </header>

  <div class="accounts">
    <AccountForm bind:creds={source} label="Source" disabled={running} />
    <div class="arrow" aria-hidden="true">→</div>
    <AccountForm bind:creds={destination} label="Destination" disabled={running} />
  </div>

  <div class="controls">
    <label class="dry">
      <input type="checkbox" bind:checked={dryRun} disabled={running} />
      <span>
        <strong>Dry run</strong> — walk everything and count, but write nothing.
        {#if dryRun}<em>Uncheck to actually copy.</em>{/if}
      </span>
    </label>
    <button onclick={run} disabled={!canRun}>
      {running ? "Migrating…" : dryRun ? "Preview migration" : "Start migration"}
    </button>
  </div>

  {#if error}
    <div class="error" role="alert">
      <strong>Migration failed.</strong>
      <span>{error}</span>
    </div>
  {/if}

  {#if running || doneFolders.length || report}
    <section class="status">
      {#if current}
        <div class="current">
          <div class="current-head">
            <span class="folder">{current.name}</span>
            <span class="muted">{current.done} / {current.total}</span>
          </div>
          <div class="bar">
            <div
              class="fill"
              style="width: {current.total ? (current.done / current.total) * 100 : 0}%"
            ></div>
          </div>
          <div class="muted small">
            Folder {current.index + 1} of {totalFolders}
          </div>
        </div>
      {/if}

      {#if doneFolders.length}
        <table>
          <thead>
            <tr>
              <th>Folder</th>
              <th>Source</th>
              <th>{dryRun ? "Would copy" : "Copied"}</th>
              <th>{dryRun ? "Already there" : "Skipped"}</th>
              <th>Failed</th>
            </tr>
          </thead>
          <tbody>
            {#each doneFolders as f (f.folder)}
              <tr>
                <td class="folder">{f.folder}</td>
                <td>{f.sourceTotal}</td>
                <td>{f.copied}</td>
                <td>{f.skipped}</td>
                <td class:bad={f.failed > 0}>{f.failed}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}

      {#if report}
        <div class="summary" class:ok={report.failed === 0 && report.missing === 0}>
          <strong>
            {dryRun ? "Preview complete" : "Migration complete"}
          </strong>
          <span>
            {#if dryRun}
              {report.copied} would copy · {report.skipped} already there · {report.failed} failed
            {:else}
              {report.copied} copied · {report.skipped} skipped (already there) · {report.failed}
              failed{report.missing > 0 ? ` · ${report.missing} missing after verify` : " · verified ✓"}
            {/if}
            across {report.folders.length} folders.
          </span>
        </div>
      {/if}

      {#if warnings.length}
        <details class="warnings">
          <summary>{warnings.length} warning{warnings.length === 1 ? "" : "s"}</summary>
          <ul>
            {#each warnings as w, i (i)}
              <li>{w}</li>
            {/each}
          </ul>
        </details>
      {/if}
    </section>
  {/if}

  <details class="dav-section">
    <summary>Calendar &amp; contacts (experimental)</summary>
    <DavMigrate />
  </details>
</main>

<style>
  main {
    max-width: 760px;
    margin: 0 auto;
    padding: 28px 20px 56px;
  }

  header h1 {
    margin: 0;
    font-size: 1.9rem;
    letter-spacing: -0.02em;
  }

  .tagline {
    margin: 4px 0 0;
    color: var(--muted);
    font-size: 0.9rem;
    max-width: 60ch;
  }

  .accounts {
    margin-top: 22px;
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .arrow {
    color: var(--muted);
    font-size: 1.4rem;
    flex: none;
  }

  .controls {
    margin-top: 16px;
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
    max-width: 46ch;
  }

  .dry input {
    margin-top: 2px;
  }

  .dry em {
    font-style: normal;
    color: var(--accent);
  }

  button {
    padding: 11px 18px;
    border: none;
    border-radius: 9px;
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
    margin-top: 18px;
    padding: 12px 14px;
    border-radius: 10px;
    background: var(--danger-bg);
    color: var(--danger);
    font-size: 0.9rem;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .status {
    margin-top: 24px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .current-head {
    display: flex;
    justify-content: space-between;
    font-size: 0.9rem;
    margin-bottom: 6px;
  }

  .current .folder {
    font-weight: 600;
  }

  .bar {
    height: 8px;
    border-radius: 999px;
    background: var(--line);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s ease;
  }

  .muted {
    color: var(--muted);
  }

  .small {
    font-size: 0.78rem;
    margin-top: 5px;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 0.85rem;
  }

  th,
  td {
    text-align: right;
    padding: 7px 10px;
    border-bottom: 1px solid var(--line);
  }

  th:first-child,
  td.folder {
    text-align: left;
  }

  th {
    color: var(--muted);
    font-weight: 600;
  }

  td.bad {
    color: var(--danger);
    font-weight: 600;
  }

  .summary {
    padding: 12px 14px;
    border-radius: 10px;
    background: var(--line);
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: 0.9rem;
  }

  .summary.ok {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }

  .warnings {
    font-size: 0.83rem;
    color: var(--muted);
  }

  .warnings ul {
    margin: 8px 0 0;
    padding-left: 18px;
  }

  .warnings li {
    margin-bottom: 3px;
    word-break: break-word;
  }

  .dav-section {
    margin-top: 32px;
    border-top: 1px solid var(--line);
    padding-top: 16px;
  }

  .dav-section summary {
    cursor: pointer;
    font-weight: 600;
    font-size: 0.92rem;
    color: var(--muted);
  }
</style>
