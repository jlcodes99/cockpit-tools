export type SshAuthConfig =
  | { kind: 'agent' }
  | { kind: 'private_key_file'; path: string };

export interface SshCodexSyncStatus {
  account_id: string;
  account_email: string;
  token_generation: number;
  bundle_hash: string;
  synced_at: number;
  verified: boolean;
  error: string | null;
}

export interface SshCodexSyncResult extends SshCodexSyncStatus {
  server_id: string;
  server_name: string;
}

export interface SshServer {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  codex_home: string;
  auth: SshAuthConfig;
  sync_on_codex_switch: boolean;
  created_at: number;
  updated_at: number;
  last_sync: SshCodexSyncStatus | null;
}

export interface SshServerList {
  selected_server_id: string | null;
  servers: SshServer[];
}

export interface SshServerDraft {
  id?: string;
  name: string;
  host: string;
  port?: number;
  username: string;
  codex_home?: string;
  auth: SshAuthConfig;
  sync_on_codex_switch?: boolean;
}
