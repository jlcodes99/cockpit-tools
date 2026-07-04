import { invoke } from '@tauri-apps/api/core';
import type { SshCodexSyncResult, SshServer, SshServerDraft, SshServerList } from '../types/sshServer';

function toServer(draft: SshServerDraft): SshServer {
  return {
    id: draft.id ?? '',
    name: draft.name,
    host: draft.host,
    port: draft.port ?? 22,
    username: draft.username,
    codex_home: draft.codex_home?.trim() || '~/.codex',
    auth: draft.auth,
    sync_on_codex_switch: draft.sync_on_codex_switch ?? true,
    created_at: 0,
    updated_at: 0,
    last_sync: null,
  };
}

export async function listSshServers(): Promise<SshServerList> {
  return await invoke('list_ssh_servers');
}

export async function upsertSshServer(draft: SshServerDraft): Promise<SshServerList> {
  return await invoke('upsert_ssh_server', { server: toServer(draft) });
}

export async function deleteSshServer(serverId: string): Promise<SshServerList> {
  return await invoke('delete_ssh_server', { serverId });
}

export async function selectSshServer(serverId: string | null): Promise<SshServerList> {
  return await invoke('select_ssh_server', { serverId });
}

export async function testSshServerConnection(serverId: string): Promise<string> {
  return await invoke('test_ssh_server_connection', { serverId });
}

export async function syncCurrentCodexAccountToSshServer(
  serverId?: string | null,
): Promise<SshCodexSyncResult> {
  return await invoke('sync_current_codex_account_to_ssh_server', {
    serverId: serverId ?? null,
  });
}
