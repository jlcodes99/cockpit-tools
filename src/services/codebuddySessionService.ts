import { invoke } from '@tauri-apps/api/core';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface CodebuddySessionLocation {
  instanceId: string;
  instanceName: string;
  local?: boolean;
}

export interface CodebuddySessionRecord {
  conversationId: string;
  title: string;
  cwd: string;
  userId: string;
  status: string;
  createdAt: number | null;
  updatedAt: number | null;
  isPlayground: boolean;
  locations: CodebuddySessionLocation[];
}

// ---------------------------------------------------------------------------
// Service calls
// ---------------------------------------------------------------------------

export type CodebuddySessionPlatform = 'cn' | 'intl' | 'workbuddy';

export interface CodebuddySessionOverview {
  sessions: CodebuddySessionRecord[];
  warnings: string[];
}

export function workbuddyRebindWorkspace(instanceId: string, oldCwd: string, newCwd: string): Promise<number> {
  return invoke('workbuddy_rebind_workspace', { instanceId, oldCwd, newCwd });
}

export async function codebuddyListSessionOverview(
  platform: CodebuddySessionPlatform,
  opts?: { keyword?: string; status?: string },
): Promise<CodebuddySessionOverview> {
  return await invoke('codebuddy_list_session_overview', {
    platform,
    keyword: opts?.keyword ?? null,
    status: opts?.status ?? null,
  });
}
