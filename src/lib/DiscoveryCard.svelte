<script lang="ts">
  import type { Account, Probe } from "./types";

  let {
    account = $bindable(),
    probe,
    busy,
    sameEmail,
    running,
    onRetry,
  }: {
    account: Account;
    probe: Probe;
    busy: boolean;
    sameEmail: boolean;
    running: boolean;
    onRetry: () => void;
  } = $props();
</script>

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
        <button class="try" onclick={onRetry} disabled={busy || running}>{busy ? "Trying…" : "Try again"}</button>
      </div>
    </details>
  {/if}
</div>

<style>
  .card { flex: 1; min-width: 0; border: 1px solid var(--line); border-radius: 12px; padding: 14px; background: var(--card); }
  .card-head { font-weight: 600; font-size: 0.85rem; margin-bottom: 8px; word-break: break-all; }
  .status-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 5px; }
  .status-list li { display: flex; justify-content: space-between; font-size: 0.85rem; }
  .lbl { color: var(--muted); }
  .ok { color: var(--accent); font-weight: 600; }
  .miss { color: var(--muted); }

  .servers { margin-top: 12px; border-top: 1px solid var(--line); padding-top: 10px; }
  .servers summary { cursor: pointer; font-size: 0.8rem; color: var(--muted); }
  .fixers { display: flex; flex-direction: column; gap: 8px; margin-top: 10px; }
  .fix-hint { margin: 0; font-size: 0.78rem; color: var(--muted); }
  .fixers .f { display: flex; flex-direction: column; gap: 4px; font-size: 0.78rem; color: var(--muted); }
  .fixers input {
    padding: 7px 9px; border: 1px solid var(--line); border-radius: 8px; background: var(--bg); color: var(--ink); font-size: 0.82rem;
  }
  .fixers input:focus { outline: 2px solid var(--accent); outline-offset: -1px; }
  .try {
    align-self: flex-start; padding: 7px 13px; font-size: 0.85rem; border: none; border-radius: 8px;
    background: var(--accent); color: var(--accent-ink); font-weight: 600; cursor: pointer;
  }
  .try:disabled { opacity: 0.5; cursor: not-allowed; }
</style>
