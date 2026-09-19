// Shapes shared with the Rust backend (must match the serde output in
// src-tauri/src/domain.rs). Single source of truth for the frontend.

export type Provider = "password" | "microsoft" | "google";

export type Account = {
  email: string;
  password: string;
  imapHost: string | null;
  imapPort: number | null;
  davUrl: string | null;
  provider: Provider;
};

export const blankAccount = (): Account => ({
  email: "",
  password: "",
  imapHost: null,
  imapPort: null,
  davUrl: null,
  provider: "password",
});

export type FolderInfo = { name: string; delimiter: string | null; attributes: string[] };
export type DavCollection = { name: string; href: string; count: number | null };

export type ImapProbe = {
  status: string;
  host: string | null;
  port: number | null;
  folders: FolderInfo[];
  error: string | null;
};
export type DavProbe = { status: string; collections: DavCollection[]; error: string | null };
export type Probe = { imap: ImapProbe; calendars: DavProbe; contacts: DavProbe };

export type FolderReport = {
  folder: string;
  sourceTotal: number;
  copied: number;
  skipped: number;
  failed: number;
  missing: number;
};
export type NamedDavReport = {
  name: string;
  total: number;
  copied: number;
  skipped: number;
  failed: number;
  missing: number;
};
export type UnifiedReport = {
  folders: FolderReport[];
  calendars: NamedDavReport[];
  contacts: NamedDavReport[];
};

export type DavPair = { name: string; source: string; dest: string };
export type MigrateSelection = { folders: string[]; calendars: DavPair[]; contacts: DavPair[] };

export type Progress =
  | { type: "started"; folders: number }
  | { type: "folderStart"; name: string; index: number; folders: number; sourceTotal: number }
  | { type: "tick"; folder: string; done: number; total: number }
  | { type: "folderDone"; report: FolderReport }
  | { type: "warning"; message: string };

export type Current = { name: string; done: number; total: number };
