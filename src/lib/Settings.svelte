<script lang="ts">
  import { DEFAULT_MICROSOFT, getOverrides, saveOverrides } from "./settings";

  let ids = $state(getOverrides());

  function persist() {
    saveOverrides(ids);
  }
</script>

<div class="settings">
  <p class="hint">
    OAuth client IDs (public identifiers, stored on this device only). Leave Microsoft blank to use
    Godwit's default. Google has no default — create your own client ID and paste it here; see
    <code>docs/google-setup.md</code>.
  </p>

  <label>
    <span>Microsoft 365 client ID</span>
    <input type="text" bind:value={ids.microsoft} oninput={persist} placeholder={`default: ${DEFAULT_MICROSOFT}`} spellcheck="false" autocomplete="off" />
  </label>
  <label>
    <span>Google client ID</span>
    <input type="text" bind:value={ids.google} oninput={persist} placeholder="required — your own Google OAuth client ID" spellcheck="false" autocomplete="off" />
  </label>
</div>

<style>
  .settings { display: flex; flex-direction: column; gap: 12px; padding-top: 8px; }
  .hint { margin: 0; font-size: 0.82rem; color: var(--muted); max-width: 66ch; }
  .hint code { font-size: 0.9em; }
  label { display: flex; flex-direction: column; gap: 5px; font-size: 0.82rem; color: var(--muted); }
  input {
    padding: 8px 10px; border: 1px solid var(--line); border-radius: 8px;
    background: var(--bg); color: var(--ink); width: 100%;
  }
  input:focus { outline: 2px solid var(--accent); outline-offset: -1px; }
</style>
