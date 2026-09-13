<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  type FolderInfo = {
    name: string;
    delimiter: string | null;
    attributes: string[];
  };

  let host = $state("");
  let port = $state(993);
  let username = $state("");
  let password = $state("");

  let loading = $state(false);
  let error = $state<string | null>(null);
  let folders = $state<FolderInfo[] | null>(null);

  const canSubmit = $derived(
    host.trim() !== "" && username.trim() !== "" && password !== "" && !loading,
  );

  async function connect(event: SubmitEvent) {
    event.preventDefault();
    loading = true;
    error = null;
    folders = null;
    try {
      folders = await invoke<FolderInfo[]>("list_folders", {
        creds: {
          host: host.trim(),
          port: Number(port),
          username: username.trim(),
          password,
        },
      });
    } catch (err) {
      error = typeof err === "string" ? err : String(err);
    } finally {
      loading = false;
    }
  }
</script>

<main>
  <header>
    <h1>Godwit</h1>
    <p class="tagline">
      Phase&nbsp;0 spike — connect to a mailbox and list its folders. Nothing leaves
      this machine.
    </p>
  </header>

  <form onsubmit={connect}>
    <div class="row">
      <label class="grow">
        <span>IMAP host</span>
        <input
          type="text"
          bind:value={host}
          placeholder="imap.example.com"
          autocomplete="off"
          spellcheck="false"
        />
      </label>
      <label class="port">
        <span>Port</span>
        <input type="number" bind:value={port} min="1" max="65535" />
      </label>
    </div>

    <label>
      <span>Username</span>
      <input
        type="text"
        bind:value={username}
        placeholder="you@example.com"
        autocomplete="off"
        spellcheck="false"
      />
    </label>

    <label>
      <span>Password / app password</span>
      <input type="password" bind:value={password} autocomplete="off" />
    </label>

    <button type="submit" disabled={!canSubmit}>
      {loading ? "Connecting…" : "Connect & list folders"}
    </button>
  </form>

  {#if error}
    <div class="error" role="alert">
      <strong>Couldn't connect.</strong>
      <span>{error}</span>
    </div>
  {/if}

  {#if folders}
    <section class="results">
      <h2>{folders.length} folder{folders.length === 1 ? "" : "s"}</h2>
      <ul>
        {#each folders as folder (folder.name)}
          <li>
            <span class="folder-name">{folder.name}</span>
            {#if folder.attributes.length}
              <span class="attrs">{folder.attributes.join(" · ")}</span>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</main>

<style>
  main {
    max-width: 560px;
    margin: 0 auto;
    padding: 32px 20px 56px;
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
  }

  form {
    margin-top: 26px;
    background: var(--card);
    border: 1px solid var(--line);
    border-radius: 14px;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .row {
    display: flex;
    gap: 12px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 0.85rem;
    color: var(--muted);
  }

  .grow {
    flex: 1;
  }

  .port {
    width: 92px;
  }

  input {
    padding: 9px 11px;
    border: 1px solid var(--line);
    border-radius: 9px;
    background: var(--bg);
    color: var(--ink);
  }

  input:focus {
    outline: 2px solid var(--accent);
    outline-offset: -1px;
  }

  button {
    margin-top: 4px;
    padding: 11px 14px;
    border: none;
    border-radius: 9px;
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

  .results {
    margin-top: 26px;
  }

  .results h2 {
    font-size: 1rem;
    color: var(--muted);
    font-weight: 600;
    margin: 0 0 10px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: 12px;
    overflow: hidden;
  }

  li {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 12px;
    padding: 10px 14px;
    background: var(--card);
  }

  li:not(:last-child) {
    border-bottom: 1px solid var(--line);
  }

  .folder-name {
    font-weight: 500;
  }

  .attrs {
    color: var(--muted);
    font-size: 0.78rem;
    text-align: right;
  }
</style>
