// Per-provider OAuth client IDs, stored locally (they are public identifiers,
// not secrets). Microsoft ships a default that users may override; Google has
// no default — each user brings their own client ID (see docs/google-setup.md),
// which is how open-source apps avoid the maintainer owning verification/quota
// for every user.

import type { Provider } from "./types";

const KEY = "godwit.clientIds";

const DEFAULTS = {
  microsoft: "5746290e-9198-489b-b325-be452af41cc5",
  google: "",
};

export const DEFAULT_MICROSOFT = DEFAULTS.microsoft;

export type ClientIds = { microsoft: string; google: string };

function readOverrides(): Partial<ClientIds> {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? JSON.parse(raw) : {};
  } catch {
    return {};
  }
}

/// The user's raw overrides (empty string = not set), for editing in Settings.
export function getOverrides(): ClientIds {
  const o = readOverrides();
  return { microsoft: o.microsoft ?? "", google: o.google ?? "" };
}

export function saveOverrides(ids: ClientIds): void {
  try {
    const clean: Partial<ClientIds> = {};
    if (ids.microsoft.trim()) clean.microsoft = ids.microsoft.trim();
    if (ids.google.trim()) clean.google = ids.google.trim();
    localStorage.setItem(KEY, JSON.stringify(clean));
  } catch {
    // localStorage may be unavailable (private window etc.) — non-fatal.
  }
}

/// The client ID to actually use for a provider: the user's override, else the
/// shipped default.
export function getClientId(provider: Provider): string {
  if (provider !== "microsoft" && provider !== "google") return "";
  const override = (readOverrides()[provider] ?? "").trim();
  return override || DEFAULTS[provider];
}
