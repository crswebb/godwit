<script lang="ts" module>
  export type Account = {
    email: string;
    password: string;
    imapHost: string | null;
    imapPort: number | null;
    davUrl: string | null;
  };

  export const blankAccount = (): Account => ({
    email: "",
    password: "",
    imapHost: null,
    imapPort: null,
    davUrl: null,
  });
</script>

<script lang="ts">
  let {
    label,
    account = $bindable(),
    disabled = false,
  }: { label: string; account: Account; disabled?: boolean } = $props();
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

  fieldset:disabled {
    opacity: 0.6;
  }
</style>
