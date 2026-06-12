import { invoke } from '@tauri-apps/api/core';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface CodebuddySessionLocation {
  instanceId: string;
  instanceName: string;
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

export type CodebuddySessionPlatform = 'cn' | 'intl';

export async function codebuddyListSessions(
  platform: CodebuddySessionPlatform,
  opts?: { keyword?: string; status?: string },
): Promise<CodebuddySessionRecord[]> {
  return await invoke('codebuddy_list_sessions', {
    platform,
    keyword: opts?.keyword ?? null,
    status: opts?.status ?? null,
  });
}
