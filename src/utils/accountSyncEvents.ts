import { emit } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { Page } from '../types/navigation';
import { ALL_PLATFORM_IDS, PLATFORM_PAGE_MAP, type PlatformId } from '../types/platform';

export const ACCOUNTS_CHANGED_EVENT = 'accounts:changed';
export const CURRENT_ACCOUNT_CHANGED_EVENT = 'accounts:current-changed';
/** 主窗口当前平台（侧栏/页面）变化：悬浮窗/菜单应跟随，而不是锁死 antigravity */
export const ACTIVE_PLATFORM_FOCUS_EVENT = 'platform:active-focus';

export const FLOATING_CARD_PLATFORM_STORAGE_KEY = 'agtools.floating_card.platform';

export type AccountSyncEventPayload = {
  platformId: PlatformId;
  accountId?: string | null;
  reason?: string;
  sourceWindowLabel?: string;
};

export type ActivePlatformFocusPayload = {
  platformId: PlatformId;
  page?: string;
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
  grok: 'grok',
  kimi: 'kimi',
  codebuddy: 'codebuddy',
  codebuddycn: 'codebuddy_cn',
  codebuddy_cn: 'codebuddy_cn',
  qoder: 'qoder',
  zcode: 'zcode',
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

/** 主窗口 Page → 平台（dashboard/settings 等非平台页返回 null，不强制改悬浮窗） */
export function resolvePlatformIdFromPage(page: Page | string): PlatformId | null {
  const normalized = String(page || '').trim().toLowerCase();
  if (!normalized) return null;

  // 特殊页
  if (normalized === 'overview' || normalized === 'accounts') {
    return 'antigravity';
  }
  if (normalized === 'claude-cli') {
    return 'claude_manager';
  }
  if (normalized === 'codex-api-service') {
    return 'codex_api_service';
  }
  if (normalized === 'codex-instances') {
    return 'codex';
  }
  if (
    normalized === 'dashboard' ||
    normalized === 'settings' ||
    normalized === 'manual' ||
    normalized === 'api-relay' ||
    normalized === 'wakeup' ||
    normalized === 'verification' ||
    normalized === '2fa' ||
    normalized === 'instances'
  ) {
    return null;
  }

  // PLATFORM_PAGE_MAP 反向
  for (const platformId of ALL_PLATFORM_IDS) {
    if (PLATFORM_PAGE_MAP[platformId] === normalized) {
      return platformId;
    }
  }

  // 兜底：page 名本身就是 platform id
  if (ALL_PLATFORM_IDS.includes(normalized as PlatformId)) {
    return normalized as PlatformId;
  }
  return normalizeProviderPagePlatformId(normalized);
}

export function persistFloatingCardPlatform(platformId: PlatformId): void {
  try {
    localStorage.setItem(FLOATING_CARD_PLATFORM_STORAGE_KEY, platformId);
  } catch {
    // ignore
  }
}

export async function emitActivePlatformFocus(payload: ActivePlatformFocusPayload) {
  persistFloatingCardPlatform(payload.platformId);
  try {
    await emit<ActivePlatformFocusPayload>(ACTIVE_PLATFORM_FOCUS_EVENT, {
      ...payload,
      sourceWindowLabel: payload.sourceWindowLabel ?? resolveSourceWindowLabel(),
    });
  } catch (error) {
    console.warn(`[account-sync] Failed to emit ${ACTIVE_PLATFORM_FOCUS_EVENT}:`, error);
  }
}
