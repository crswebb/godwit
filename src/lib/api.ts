// The only place that talks to the Tauri backend: command invocations and the
// progress event stream. Components and App never call `invoke` directly.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Account, MigrateSelection, Probe, Progress, UnifiedReport } from "./types";

/// Blank optional fields so the backend sees `null`, not empty strings.
function normalize(a: Account): Account {
  const clean = (s: string | null) => (s && s.trim() !== "" ? s.trim() : null);
  return { ...a, imapHost: clean(a.imapHost), davUrl: clean(a.davUrl), imapPort: a.imapPort || null };
}

export const probeAccount = (account: Account): Promise<Probe> =>
  invoke<Probe>("probe", { account: normalize(account) });

export const runMigration = (
  source: Account,
  destination: Account,
  selection: MigrateSelection,
  dryRun: boolean,
): Promise<UnifiedReport> =>
  invoke<UnifiedReport>("run_migration", {
    source: normalize(source),
    destination: normalize(destination),
    selection,
    dryRun,
  });

export const msSignIn = (clientId: string): Promise<{ email: string }> =>
  invoke<{ email: string }>("ms_sign_in", { clientId });

export const googleSignIn = (clientId: string): Promise<{ email: string }> =>
  invoke<{ email: string }>("google_sign_in", { clientId });

/// Subscribe to migration progress; returns an unsubscribe function.
export function onProgress(cb: (p: Progress) => void): () => void {
  const unlisten = listen<Progress>("migration://progress", (event) => cb(event.payload));
  return () => {
    unlisten.then((f) => f());
  };
}

export const errorText = (err: unknown): string => (typeof err === "string" ? err : String(err));
