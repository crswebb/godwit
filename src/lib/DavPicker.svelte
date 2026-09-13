<script lang="ts" module>
  export type DavAccount = { url: string | null; username: string; password: string };
  export type DavCollection = { name: string; href: string; count: number | null };
  export type DavKind = "calendar" | "contacts";
</script>

<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  let {
    label,
    kind,
    account = $bindable(),
    selectedHref = $bindable(),
    disabled = false,
  }: {
    label: string;
    kind: DavKind;
    account: DavAccount;
    selectedHref: string | null;
    disabled?: boolean;
  } = $props();

  let serverUrl = $state("");
  let loading = $state(false);
  let error = $state<string | null>(null);
  let collections = $state<DavCollection[] | null>(null);

  const canDiscover = $derived(
    account.username.trim() !== "" && account.password !== "" && !loading && !disabled,
  );

  async function discover() {
    loading = true;
    error = null;
    collections = null;
    selectedHref = null;
    account.url = serverUrl.trim() === "" ? null : serverUrl.trim();
    try {
      collections = await invoke<DavCollection[]>("dav_discover", { account, kind });
      if (collections.length) selectedHref = collections[0].href;
    } catch (err) {
      error = typeof err === "string" ? err : String(err);
    } finally {
      loading = false;
    }
  }
</script>

<fieldset {disabled}>
  <legend>{label}</legend>

  <label>
    <span>Email</span>
    <input type="text" bind:value={account.username} placeholder="you@example.com" spellcheck="false" autocomplete="off" />
  </label>
  <label>
    <span>Password</span>
    <input type="password" bind:value={account.password} autocomplete="off" />
  </label>

  <details class="advanced">
    <summary>Advanced</summary>
    <label>
      <span>Server URL (only if autodiscovery fails)</span>
      <input type="text" bind:value={serverUrl} placeholder="https://caldav.example.com/" spellcheck="false" autocomplete="off" />
    </label>
  </details>

  <button type="button" onclick={discover} disabled={!canDiscover}>
    {loading ? "Discovering…" : "Discover"}
  </button>

  {#if error}
    <div class="error" role="alert">{error}</div>
  {/if}

  {#if collections}
    {#if collections.length === 0}
      <p class="empty">Nothing found for that account.</p>
    {:else}
      <label>
        <span>{kind === "calendar" ? "Calendar" : "Address book"}</span>
        <select bind:value={selectedHref}>
          {#each collections as c (c.href)}
            <option value={c.href}>{c.name}{c.count === null ? "" : ` (${c.count})`}</option>
          {/each}
        </select>
      </label>
    {/if}
  {/if}
</fieldset>

<style>
  fieldset {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--line);
    border-radius: 12px;
    padding: 16px;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 11px;
    background: var(--card);
  }

  legend {
    padding: 0 8px;
    font-weight: 600;
    font-size: 0.9rem;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 0.82rem;
    color: var(--muted);
  }

  input,
  select {
    padding: 8px 10px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--bg);
    color: var(--ink);
    width: 100%;
  }

  input:focus,
  select:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }

  .advanced summary {
    cursor: pointer;
    font-size: 0.8rem;
    color: var(--muted);
  }

  .advanced label {
    margin-top: 8px;
  }

  button {
    align-self: flex-start;
    padding: 8px 14px;
    border: none;
    border-radius: 8px;
    background: var(--accent);
    color: var(--accent-ink);
    font-weight: 600;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .error {
    padding: 9px 11px;
    border-radius: 8px;
    background: var(--danger-bg);
    color: var(--danger);
    font-size: 0.82rem;
  }

  .empty {
    font-size: 0.82rem;
    color: var(--muted);
  }
</style>
