import { invoke } from '@tauri-apps/api/core';
import type { PlatformId } from '../types/platform';
import { ALL_PLATFORM_IDS } from '../types/platform';
import type {
  RemoteConfigAppliedRule,
  RemoteConfigState,
  RemoteUpdatePromptMode,
} from '../types/remoteConfig';

type RemoteConfigAppliedRuleRaw = {
  platformIds?: unknown;
  platform_ids?: unknown;
  reason?: unknown;
};

type RemoteConfigStateRaw = {
  version?: unknown;
  codexOAuthAppVersion?: unknown;
  codex_oauth_app_version?: unknown;
  updatedAt?: unknown;
  updated_at?: unknown;
  currentOs?: unknown;
  current_os?: unknown;
  hiddenPlatformIds?: unknown;
  hidden_platform_ids?: unknown;
  appliedRules?: unknown;
  applied_rules?: unknown;
  refreshIntervalMs?: unknown;
  refresh_interval_ms?: unknown;
  updatePromptMode?: unknown;
  update_prompt_mode?: unknown;
};

const DEFAULT_REFRESH_INTERVAL_MS = 60 * 60 * 1000;
const NEVER_REMOTE_HIDE_PLATFORM_IDS = new Set<PlatformId>(['claude_manager']);

function normalizeUpdatePromptMode(value: unknown): RemoteUpdatePromptMode {
  return typeof value === 'string' && value.trim().toLowerCase() === 'popup'
    ? 'popup'
    : 'normal';
}

function normalizePlatformIds(value: unknown): PlatformId[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<PlatformId>();
  const result: PlatformId[] = [];
  for (const item of value) {
    if (typeof item !== 'string') continue;
    const platformId = item as PlatformId;
    if (!ALL_PLATFORM_IDS.includes(platformId)) continue;
    if (seen.has(platformId)) continue;
    seen.add(platformId);
    result.push(platformId);
  }
  return result;
}

function normalizeAppliedRules(value: unknown): RemoteConfigAppliedRule[] {
  if (!Array.isArray(value)) return [];
  return value
    .map((item): RemoteConfigAppliedRule | null => {
      if (!item || typeof item !== 'object') return null;
      const raw = item as RemoteConfigAppliedRuleRaw;
      const platformIds = normalizePlatformIds(raw.platformIds ?? raw.platform_ids);
      if (platformIds.length === 0) return null;
      return {
        platformIds,
        reason: typeof raw.reason === 'string' ? raw.reason : null,
      };
    })
    .filter((item): item is RemoteConfigAppliedRule => Boolean(item));
}

function normalizeRemoteConfigState(raw: RemoteConfigStateRaw): RemoteConfigState {
  const refreshIntervalMs = Number(raw.refreshIntervalMs ?? raw.refresh_interval_ms);
  const hiddenPlatformIds = normalizePlatformIds(
    raw.hiddenPlatformIds ?? raw.hidden_platform_ids,
  ).filter((platformId) => !NEVER_REMOTE_HIDE_PLATFORM_IDS.has(platformId));
  const appliedRules = normalizeAppliedRules(raw.appliedRules ?? raw.applied_rules)
    .map((rule) => ({
      ...rule,
      platformIds: rule.platformIds.filter(
        (platformId) => !NEVER_REMOTE_HIDE_PLATFORM_IDS.has(platformId),
      ),
    }))
    .filter((rule) => rule.platformIds.length > 0);
  return {
    version: typeof raw.version === 'string' ? raw.version : '',
    codexOAuthAppVersion:
      typeof (raw.codexOAuthAppVersion ?? raw.codex_oauth_app_version) === 'string'
        ? String(raw.codexOAuthAppVersion ?? raw.codex_oauth_app_version)
        : '26.820.60940',
    updatedAt: Number(raw.updatedAt ?? raw.updated_at) || 0,
    currentOs: typeof (raw.currentOs ?? raw.current_os) === 'string'
      ? String(raw.currentOs ?? raw.current_os)
      : '',
    hiddenPlatformIds,
    appliedRules,
    refreshIntervalMs:
      Number.isFinite(refreshIntervalMs) && refreshIntervalMs >= 60_000
        ? refreshIntervalMs
        : DEFAULT_REFRESH_INTERVAL_MS,
    updatePromptMode: normalizeUpdatePromptMode(
      raw.updatePromptMode ?? raw.update_prompt_mode,
    ),
  };
}

export async function getRemoteConfigState(): Promise<RemoteConfigState> {
  const raw = await invoke<RemoteConfigStateRaw>('remote_config_get_state');
  return normalizeRemoteConfigState(raw);
}

export async function forceRefreshRemoteConfigState(): Promise<RemoteConfigState> {
  const raw = await invoke<RemoteConfigStateRaw>('remote_config_force_refresh');
  return normalizeRemoteConfigState(raw);
}
