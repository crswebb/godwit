<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  type MsFolder = { name: string; count: number };
  type MsProbe = { folders: MsFolder[]; calendars: string[]; contactFolders: string[] };

  // Pre-filled with the Godwit app registered in the CRS Webbproduktion tenant.
  // A client ID is a public identifier, not a secret; change it to use another app.
  let clientId = $state("5746290e-9198-489b-b325-be452af41cc5");
  let email = $state<string | null>(null);
  let signingIn = $state(false);
  let probing = $state(false);
  let error = $state<string | null>(null);
  let probe = $state<MsProbe | null>(null);

  async function signIn() {
    signingIn = true;
    error = null;
    probe = null;
    try {
      const account = await invoke<{ email: string }>("ms_sign_in", { clientId: clientId.trim() });
      email = account.email;
      await discover();
    } catch (err) {
      error = typeof err === "string" ? err : String(err);
    } finally {
      signingIn = false;
    }
  }

  async function discover() {
    if (!email) return;
    probing = true;
    error = null;
    try {
      probe = await invoke<MsProbe>("ms_probe", { email });
    } catch (err) {
      error = typeof err === "string" ? err : String(err);
    } finally {
      probing = false;
    }
  }
</script>

<div class="ms">
  <p class="hint">
    Connect a Microsoft 365 account (work/school or personal). Requires a one-time app registration —
    see <code>docs/microsoft-setup.md</code> — which gives you a client ID to paste below. A browser
    window opens for you to sign in and consent.
  </p>

  <label>
    <span>Microsoft app client ID</span>
    <input type="text" bind:value={clientId} placeholder="00000000-0000-0000-0000-000000000000" spellcheck="false" autocomplete="off" />
  </label>

  <button onclick={signIn} disabled={clientId.trim() === "" || signingIn}>
    {signingIn ? "Waiting for browser…" : "Sign in with Microsoft"}
  </button>

  {#if email}
    <p class="signed">Signed in as <strong>{email}</strong></p>
  {/if}

  {#if error}
    <div class="error" role="alert">{error}</div>
  {/if}

  {#if probe}
    <div class="results">
      <div class="block">
        <h4>Mail folders</h4>
        {#if probe.folders.length}
          <ul>{#each probe.folders as f (f.name)}<li><span>{f.name}</span><span class="muted">{f.count}</span></li>{/each}</ul>
        {:else}<p class="muted">None.</p>{/if}
      </div>
      <div class="block">
        <h4>Calendars</h4>
        {#if probe.calendars.length}
          <ul>{#each probe.calendars as c (c)}<li>{c}</li>{/each}</ul>
        {:else}<p class="muted">None.</p>{/if}
      </div>
      <div class="block">
        <h4>Contact folders</h4>
        {#if probe.contactFolders.length}
          <ul>{#each probe.contactFolders as c (c)}<li>{c}</li>{/each}</ul>
        {:else}<p class="muted">None.</p>{/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .ms { display: flex; flex-direction: column; gap: 12px; padding-top: 6px; }
  .hint { margin: 0; font-size: 0.82rem; color: var(--muted); max-width: 64ch; }
  label { display: flex; flex-direction: column; gap: 5px; font-size: 0.82rem; color: var(--muted); max-width: 420px; }
  input { padding: 8px 10px; border: 1px solid var(--line); border-radius: 8px; background: var(--bg); color: var(--ink); }
  input:focus { outline: 2px solid var(--accent); outline-offset: -1px; }
  button { align-self: flex-start; padding: 9px 16px; border: none; border-radius: 8px; background: var(--accent); color: var(--accent-ink); font-weight: 600; cursor: pointer; }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
  .signed { margin: 0; font-size: 0.88rem; }
  .error { padding: 10px 12px; border-radius: 9px; background: var(--danger-bg); color: var(--danger); font-size: 0.85rem; }
  .results { display: flex; gap: 14px; flex-wrap: wrap; }
  .block { flex: 1; min-width: 180px; }
  .block h4 { margin: 0 0 6px; font-size: 0.85rem; }
  .block ul { list-style: none; margin: 0; padding: 0; border: 1px solid var(--line); border-radius: 8px; overflow: hidden; }
  .block li { display: flex; justify-content: space-between; padding: 6px 10px; background: var(--card); font-size: 0.82rem; }
  .block li:not(:last-child) { border-bottom: 1px solid var(--line); }
  .muted { color: var(--muted); font-size: 0.82rem; }
</style>
