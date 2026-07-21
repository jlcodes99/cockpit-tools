export type SshAuthConfig =
  | { kind: 'agent' }
  | { kind: 'private_key_file'; path: string };

export interface SshCodexStateRepairStatus {
  success: boolean;
  error_stage: string | null;
  error: string | null;
  database_found: boolean;
  backup_path: string | null;
  provider_schema_supported: boolean;
  visibility_schema_supported: boolean;
  rollout_schema_supported: boolean;
  provider_rows_to_repair: number;
  visibility_rows_to_repair: number;
  rollout_files_to_repair: number;
  rows_repaired: number;
  rollout_files_repaired: number;
  provider_rows_remaining: number;
  visibility_rows_remaining: number;
  rollout_files_remaining: number;
  quick_check: string | null;
  rollback_performed: boolean;
  rollback_verified: boolean;
  orphan_rollouts_found: number;
  orphan_threads_recovered: number;
  rollout_paths_repaired: number;
  user_events_recovered: number;
  cwd_rows_repaired: number;
}

export interface SshCodexSyncStatus {
  account_id: string;
  account_email: string;
  token_generation: number;
  bundle_hash: string;
  bundle_verified: boolean;
  model_provider: string | null;
  model_provider_verified: boolean;
  state_repair: SshCodexStateRepairStatus | null;
  app_server_reload_status: string | null;
  app_server_quiesce_status: string | null;
  app_server_restore_status: string | null;
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
