<script lang="ts" module>
  export type Account = {
    email: string;
    password: string;
    imapHost: string | null;
    imapPort: number | null;
    davUrl: string | null;
    provider: "password" | "microsoft";
  };

  export const blankAccount = (): Account => ({
    email: "",
    password: "",
    imapHost: null,
    imapPort: null,
    davUrl: null,
    provider: "password",
  });

  // The Godwit app registered in the CRS Webbproduktion tenant (public id, not a secret).
  const MS_CLIENT_ID = "5746290e-9198-489b-b325-be452af41cc5";
</script>

<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  let {
    label,
    account = $bindable(),
    disabled = false,
  }: { label: string; account: Account; disabled?: boolean } = $props();

  let signingIn = $state(false);
  let signInError = $state<string | null>(null);

  async function signInMicrosoft() {
    signingIn = true;
    signInError = null;
    try {
      const result = await invoke<{ email: string }>("ms_sign_in", { clientId: MS_CLIENT_ID });
      account.email = result.email;
      account.provider = "microsoft";
    } catch (err) {
      signInError = typeof err === "string" ? err : String(err);
    } finally {
      signingIn = false;
    }
  }

  function usePassword() {
    account.provider = "password";
  }
</script>

<fieldset {disabled}>
  <legend>{label}</legend>

  {#if account.provider === "microsoft"}
    <div class="ms">
      <span class="badge">Microsoft 365</span>
      {#if account.email}
        <p class="signed">Signed in as <strong>{account.email}</strong></p>
      {:else}
        <button type="button" class="ms-btn" onclick={signInMicrosoft} disabled={signingIn}>
          {signingIn ? "Waiting for browser…" : "Sign in with Microsoft"}
        </button>
      {/if}
      {#if signInError}<p class="err">{signInError}</p>{/if}
      <button type="button" class="link" onclick={usePassword}>Use email &amp; password instead</button>
    </div>
  {:else}
    <label>
      <span>Email</span>
      <input type="text" bind:value={account.email} placeholder="you@example.com" spellcheck="false" autocomplete="off" />
    </label>
    <label>
      <span>Password / app password</span>
      <input type="password" bind:value={account.password} autocomplete="off" />
    </label>
    <button type="button" class="link" onclick={signInMicrosoft} disabled={signingIn}>
      {signingIn ? "Waiting for browser…" : "Use Microsoft 365 instead"}
    </button>
    {#if signInError}<p class="err">{signInError}</p>{/if}
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
  legend { padding: 0 8px; font-weight: 600; font-size: 0.9rem; }
  label { display: flex; flex-direction: column; gap: 5px; font-size: 0.82rem; color: var(--muted); }
  input {
    padding: 8px 10px; border: 1px solid var(--line); border-radius: 8px;
    background: var(--bg); color: var(--ink); width: 100%;
  }
  input:focus { outline: 2px solid var(--accent); outline-offset: -1px; }
  fieldset:disabled { opacity: 0.6; }

  .ms { display: flex; flex-direction: column; gap: 10px; align-items: flex-start; }
  .badge {
    font-size: 0.72rem; font-weight: 600; padding: 2px 8px; border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent);
  }
  .signed { margin: 0; font-size: 0.85rem; }
  .ms-btn {
    padding: 9px 14px; border: none; border-radius: 8px;
    background: var(--accent); color: var(--accent-ink); font-weight: 600; cursor: pointer;
  }
  .ms-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .link {
    background: none; border: none; padding: 0; color: var(--accent);
    font-size: 0.8rem; cursor: pointer; text-align: left;
  }
  .link:disabled { opacity: 0.5; cursor: not-allowed; }
  .err { margin: 0; font-size: 0.8rem; color: var(--danger); }
</style>
