<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  type DavKind = "calendar" | "contacts";
  type DavCollection = { name: string; href: string; count: number | null };

  let email = $state("");
  let password = $state("");
  let serverUrl = $state("");
  let kind = $state<DavKind>("calendar");

  let loading = $state(false);
  let error = $state<string | null>(null);
  let collections = $state<DavCollection[] | null>(null);

  const canDiscover = $derived(email.trim() !== "" && password !== "" && !loading);

  async function discover() {
    loading = true;
    error = null;
    collections = null;
    try {
      collections = await invoke<DavCollection[]>("dav_discover", {
        account: {
          url: serverUrl.trim() === "" ? null : serverUrl.trim(),
          username: email.trim(),
          password,
        },
        kind,
      });
    } catch (err) {
      error = typeof err === "string" ? err : String(err);
    } finally {
      loading = false;
    }
  }
</script>

<div class="dav">
  <p class="hint">
    Enter your email and password — Godwit will try to find the calendar/contacts server
    automatically. Discovery is read-only. Not every host offers CalDAV/CardDAV (basic email plans
    often don't).
  </p>

  <div class="fields">
    <label class="grow">
      <span>Email</span>
      <input type="text" bind:value={email} placeholder="you@example.com" spellcheck="false" autocomplete="off" />
    </label>
    <label class="grow">
      <span>Password</span>
      <input type="password" bind:value={password} autocomplete="off" />
    </label>
    <label>
      <span>Type</span>
      <select bind:value={kind}>
        <option value="calendar">Calendars</option>
        <option value="contacts">Address books</option>
      </select>
    </label>
  </div>

  <details class="advanced">
    <summary>Advanced</summary>
    <label>
      <span>Server URL (only if autodiscovery fails)</span>
      <input
        type="text"
        bind:value={serverUrl}
        placeholder="https://caldav.example.com/"
        spellcheck="false"
        autocomplete="off"
      />
    </label>
  </details>

  <button onclick={discover} disabled={!canDiscover}>
    {loading ? "Discovering…" : "Discover"}
  </button>

  {#if error}
    <div class="error" role="alert">{error}</div>
  {/if}

  {#if collections}
    {#if collections.length === 0}
      <p class="empty">No {kind === "calendar" ? "calendars" : "address books"} found for that account.</p>
    {:else}
      <ul>
        {#each collections as c (c.href)}
          <li>
            <span class="name">{c.name}</span>
            <span class="count">{c.count === null ? "?" : c.count} items</span>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<style>
  .dav {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding-top: 6px;
  }

  .hint {
    margin: 0;
    font-size: 0.82rem;
    color: var(--muted);
    max-width: 62ch;
  }

  .fields {
    display: flex;
    gap: 10px;
    flex-wrap: wrap;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 0.82rem;
    color: var(--muted);
  }

  .grow {
    flex: 1;
    min-width: 160px;
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
    padding: 9px 16px;
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
    padding: 10px 12px;
    border-radius: 9px;
    background: var(--danger-bg);
    color: var(--danger);
    font-size: 0.85rem;
  }

  .empty {
    font-size: 0.85rem;
    color: var(--muted);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: 10px;
    overflow: hidden;
  }

  li {
    display: flex;
    justify-content: space-between;
    padding: 9px 12px;
    background: var(--card);
    font-size: 0.88rem;
  }

  li:not(:last-child) {
    border-bottom: 1px solid var(--line);
  }

  .count {
    color: var(--muted);
  }
</style>
