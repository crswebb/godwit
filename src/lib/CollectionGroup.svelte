<script lang="ts">
  import type { DavProbe } from "./types";

  let {
    title,
    source,
    dest,
    checked = $bindable(),
    running,
  }: {
    title: string;
    source: DavProbe;
    dest: DavProbe;
    checked: Record<string, boolean>;
    running: boolean;
  } = $props();

  const destHref = (name: string) => dest.collections.find((d) => d.name === name)?.href ?? null;
</script>

<div class="group">
  <h3>{title}</h3>
  {#if source.status === "ok" && source.collections.length}
    <ul class="checklist">
      {#each source.collections as c (c.href)}
        {@const target = destHref(c.name)}
        <li>
          <label class:disabled={!target}>
            <input type="checkbox" bind:checked={checked[c.href]} disabled={running || !target} />
            {c.name}{c.count === null ? "" : ` (${c.count})`}
            {#if target}<span class="arrow-to">→ {c.name}</span>{:else}<span class="muted small">no match on destination</span>{/if}
          </label>
        </li>
      {/each}
    </ul>
  {:else}
    <p class="muted">Nothing to migrate here.</p>
  {/if}
</div>

<style>
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
</style>
