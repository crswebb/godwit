<script lang="ts">
  import type { Account, Provider } from "./types";
  import { errorText, googleSignIn, msSignIn } from "./api";
  import { getClientId } from "./settings";

  let {
    label,
    account = $bindable(),
    disabled = false,
  }: { label: string; account: Account; disabled?: boolean } = $props();

  let signingIn = $state(false);
  let signInError = $state<string | null>(null);

  async function signIn(provider: Provider, run: (id: string) => Promise<{ email: string }>) {
    const clientId = getClientId(provider);
    if (clientId.trim() === "") {
      const which = provider === "google" ? "Google" : "Microsoft";
      const doc = provider === "google" ? "google-setup.md" : "microsoft-setup.md";
      signInError = `No ${which} client ID set — add one under Settings (see docs/${doc}).`;
      return;
    }
    signingIn = true;
    signInError = null;
    try {
      const result = await run(clientId);
      account.email = result.email;
      account.provider = provider;
    } catch (err) {
      signInError = errorText(err);
    } finally {
      signingIn = false;
    }
  }

  const signInMicrosoft = () => signIn("microsoft", msSignIn);
  const signInGoogle = () => signIn("google", googleSignIn);
  const usePassword = () => { account.provider = "password"; };

  const providerName = $derived(account.provider === "google" ? "Google" : "Microsoft 365");
</script>

<fieldset {disabled}>
  <legend>{label}</legend>

  {#if account.provider === "microsoft" || account.provider === "google"}
    <div class="oauth">
      <span class="badge">{providerName}</span>
      {#if account.email}
        <p class="signed">Signed in as <strong>{account.email}</strong></p>
      {:else}
        <button
          type="button"
          class="oauth-btn"
          onclick={account.provider === "google" ? signInGoogle : signInMicrosoft}
          disabled={signingIn}
        >
          {signingIn ? "Waiting for browser…" : `Sign in with ${providerName}`}
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
    <div class="providers">
      <button type="button" class="link" onclick={signInMicrosoft} disabled={signingIn}>Use Microsoft 365</button>
      <button type="button" class="link" onclick={signInGoogle} disabled={signingIn}>Use Google</button>
    </div>
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

  .oauth { display: flex; flex-direction: column; gap: 10px; align-items: flex-start; }
  .badge {
    font-size: 0.72rem; font-weight: 600; padding: 2px 8px; border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent);
  }
  .signed { margin: 0; font-size: 0.85rem; }
  .oauth-btn {
    padding: 9px 14px; border: none; border-radius: 8px;
    background: var(--accent); color: var(--accent-ink); font-weight: 600; cursor: pointer;
  }
  .oauth-btn:disabled { opacity: 0.5; cursor: not-allowed; }
  .providers { display: flex; gap: 14px; }
  .link {
    background: none; border: none; padding: 0; color: var(--accent);
    font-size: 0.8rem; cursor: pointer; text-align: left;
  }
  .link:disabled { opacity: 0.5; cursor: not-allowed; }
  .err { margin: 0; font-size: 0.8rem; color: var(--danger); }
</style>
