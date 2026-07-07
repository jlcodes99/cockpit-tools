import { emit } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { PlatformId } from '../types/platform';

export const ACCOUNTS_CHANGED_EVENT = 'accounts:changed';
export const CURRENT_ACCOUNT_CHANGED_EVENT = 'accounts:current-changed';

export type AccountSyncEventPayload = {
  platformId: PlatformId;
  accountId?: string | null;
  reason?: string;
  sourceWindowLabel?: string;
};

const PROVIDER_PAGE_PLATFORM_MAP: Record<string, PlatformId> = {
  antigravity: 'antigravity',
  codex: 'codex',
  claude: 'claude_manager',
  zed: 'zed',
  githubcopilot: 'github-copilot',
  github_copilot: 'github-copilot',
  windsurf: 'windsurf',
  kiro: 'kiro',
  cursor: 'cursor',
  gemini: 'gemini',
  codebuddy: 'codebuddy',
  codebuddycn: 'codebuddy_cn',
  codebuddy_cn: 'codebuddy_cn',
  qoder: 'qoder',
  trae: 'trae',
  trae_solo: 'trae_solo',
  traesolo: 'trae_solo',
  trae_cn: 'trae_cn',
  traecn: 'trae_cn',
  trae_solo_cn: 'trae_solo_cn',
  traesolocn: 'trae_solo_cn',
  workbuddy: 'workbuddy',
};

function normalizePlatformKey(value: string): string {
  return value.trim().toLowerCase().replace(/[^a-z0-9]+/g, '_');
}

function resolveSourceWindowLabel(): string | undefined {
  try {
    return getCurrentWindow().label;
  } catch {
    return undefined;
  }
}

async function emitAccountSyncEvent(eventName: string, payload: AccountSyncEventPayload) {
  try {
    await emit<AccountSyncEventPayload>(eventName, {
      ...payload,
      sourceWindowLabel: payload.sourceWindowLabel ?? resolveSourceWindowLabel(),
    });
  } catch (error) {
    console.warn(`[account-sync] Failed to emit ${eventName}:`, error);
  }
}

export function normalizeProviderPagePlatformId(platformKey: string): PlatformId | null {
  const normalized = normalizePlatformKey(platformKey);
  return (
    PROVIDER_PAGE_PLATFORM_MAP[normalized] ??
    PROVIDER_PAGE_PLATFORM_MAP[normalized.replace(/_/g, '')] ??
    null
  );
}

export async function emitAccountsChanged(payload: AccountSyncEventPayload) {
  await emitAccountSyncEvent(ACCOUNTS_CHANGED_EVENT, payload);
}

export async function emitCurrentAccountChanged(payload: AccountSyncEventPayload) {
  await emitAccountSyncEvent(CURRENT_ACCOUNT_CHANGED_EVENT, payload);
}
