<script lang="ts" module>
  export type Account = {
    email: string;
    password: string;
    imapHost: string | null;
    imapPort: number | null;
    davUrl: string | null;
  };
</script>

<script lang="ts">
  let {
    label,
    account = $bindable(),
    needsHost = false,
    disabled = false,
  }: { label: string; account: Account; needsHost?: boolean; disabled?: boolean } = $props();
</script>

<fieldset {disabled}>
  <legend>{label}</legend>

  <label>
    <span>Email</span>
    <input type="text" bind:value={account.email} placeholder="you@example.com" spellcheck="false" autocomplete="off" />
  </label>
  <label>
    <span>Password / app password</span>
    <input type="password" bind:value={account.password} autocomplete="off" />
  </label>

  {#if needsHost}
    <p class="warn">Couldn't find the mail server automatically — enter it under Advanced.</p>
  {/if}

  <details class="advanced" open={needsHost}>
    <summary>Advanced (only if needed)</summary>
    <div class="adv-body">
      <div class="row">
        <label class="grow">
          <span>IMAP server</span>
          <input type="text" bind:value={account.imapHost} placeholder="imap.example.com" spellcheck="false" autocomplete="off" />
        </label>
        <label class="port">
          <span>Port</span>
          <input type="number" bind:value={account.imapPort} placeholder="993" min="1" max="65535" />
        </label>
      </div>
      <label>
        <span>CalDAV/CardDAV URL</span>
        <input type="text" bind:value={account.davUrl} placeholder="https://caldav.example.com/" spellcheck="false" autocomplete="off" />
      </label>
    </div>
  </details>
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

  input {
    padding: 8px 10px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--bg);
    color: var(--ink);
    width: 100%;
  }

  input:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }

  .warn {
    margin: 0;
    font-size: 0.8rem;
    color: var(--danger);
  }

  .advanced summary {
    cursor: pointer;
    font-size: 0.8rem;
    color: var(--muted);
  }

  .adv-body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 10px;
  }

  .row {
    display: flex;
    gap: 10px;
  }

  .grow {
    flex: 1;
  }

  .port {
    width: 84px;
  }

  fieldset:disabled {
    opacity: 0.6;
  }
</style>
