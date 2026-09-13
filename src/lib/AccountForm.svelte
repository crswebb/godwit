<script lang="ts" module>
  export type Creds = {
    host: string;
    port: number;
    username: string;
    password: string;
  };
</script>

<script lang="ts">
  let {
    creds = $bindable(),
    label,
    disabled = false,
  }: { creds: Creds; label: string; disabled?: boolean } = $props();
</script>

<fieldset {disabled}>
  <legend>{label}</legend>

  <div class="row">
    <label class="grow">
      <span>IMAP host</span>
      <input
        type="text"
        bind:value={creds.host}
        placeholder="imap.example.com"
        autocomplete="off"
        spellcheck="false"
      />
    </label>
    <label class="port">
      <span>Port</span>
      <input type="number" bind:value={creds.port} min="1" max="65535" />
    </label>
  </div>

  <label>
    <span>Username</span>
    <input
      type="text"
      bind:value={creds.username}
      placeholder="you@example.com"
      autocomplete="off"
      spellcheck="false"
    />
  </label>

  <label>
    <span>Password / app password</span>
    <input type="password" bind:value={creds.password} autocomplete="off" />
  </label>
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
    gap: 12px;
    background: var(--card);
  }

  legend {
    padding: 0 8px;
    font-weight: 600;
    font-size: 0.9rem;
  }

  .row {
    display: flex;
    gap: 10px;
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
  }

  .port {
    width: 84px;
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

  fieldset:disabled {
    opacity: 0.6;
  }
</style>
