<script lang="ts">
  import type { Current, UnifiedReport } from "./types";

  let {
    current,
    report,
    dryRun,
    warnings,
  }: {
    current: Current | null;
    report: UnifiedReport | null;
    dryRun: boolean;
    warnings: string[];
  } = $props();
</script>

<section class="status">
  {#if current}
    <div class="current">
      <div class="current-head">
        <span class="folder">{current.name}</span>
        <span class="muted">{current.done} / {current.total}</span>
      </div>
      <div class="bar"><div class="fill" style="width: {current.total ? (current.done / current.total) * 100 : 0}%"></div></div>
    </div>
  {/if}

  {#if report}
    {#if report.folders.length}
      <table>
        <thead><tr><th>Folder</th><th>Source</th><th>{dryRun ? "Would" : "Copied"}</th><th>Skipped</th><th>Failed</th></tr></thead>
        <tbody>
          {#each report.folders as f (f.folder)}
            <tr>
              <td class="folder">{f.folder}</td>
              <td>{f.sourceTotal}</td>
              <td>{f.copied}</td>
              <td>{f.skipped}</td>
              <td class:bad={f.failed > 0 || f.missing > 0}>{f.failed + f.missing}</td>
            </tr>
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

<style>
  .status { margin-top: 26px; display: flex; flex-direction: column; gap: 14px; }
  .current-head { display: flex; justify-content: space-between; font-size: 0.9rem; margin-bottom: 6px; }
  .current .folder { font-weight: 600; }
  .bar { height: 8px; border-radius: 999px; background: var(--line); overflow: hidden; }
  .fill { height: 100%; background: var(--accent); transition: width 0.2s ease; }
  .muted { color: var(--muted); }

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
