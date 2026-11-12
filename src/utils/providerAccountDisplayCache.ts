import { isClaudeDesktopOAuthAccount, type ClaudeAccount } from '../types/claude';
import { getTraeAccountDisplayName, getTraeAccountPlatformId, type TraeAccount } from '../types/trae';
import type { PlatformId } from '../types/platform';

const REQUIRED_EMPTY_FIELDS = ['access_token', 'github_access_token', 'copilot_token', 'zcode_jwt_token'];
const PRIVATE_FIELDS = [
  'refresh_token', 'id_token', 'api_key', 'auth_raw', 'windsurf_api_key', 'windsurf_auth_token',
  'devin_auth1_token', 'devin_session_token', 'api_extra_env', 'claude_config_raw', 'claude_credentials_raw',
];

function selectScalars(value: unknown, keys: string[]): Record<string, unknown> | undefined {
  if (!value || typeof value !== 'object') return undefined;
  const source = value as Record<string, unknown>;
  return Object.fromEntries(keys.flatMap(key => {
    const field = source[key];
    return typeof field === 'string' || typeof field === 'boolean' ? [[key, field]] : [];
  }));
}

/** The startup cache supports display. Authentication and exports use the native account. */
export function toProviderAccountDisplayCache<T extends { id: string }>(account: T, platform: PlatformId): T {
  const cached: Record<string, unknown> = { ...account };
  for (const field of REQUIRED_EMPTY_FIELDS) {
    if (field in cached) cached[field] = '';
  }
  for (const field of PRIVATE_FIELDS) delete cached[field];

  if (platform === 'cursor') {
    cached.cursor_auth_raw = selectScalars(cached.cursor_auth_raw, [
      'isEnterprise', 'is_enterprise', 'teamMembershipType', 'team_membership_type', 'isTeamMember', 'is_team_member',
    ]);
  } else if (platform === 'kiro') {
    cached.kiro_auth_token_raw = selectScalars(cached.kiro_auth_token_raw, [
      'email', 'userEmail', 'login_hint', 'loginHint', 'userId', 'user_id', 'sub', 'accountId',
      'profileArn', 'profile_arn', 'arn', 'provider', 'loginProvider', 'login_option', 'authMethod',
    ]);
  } else if (platform === 'windsurf') {
    cached.windsurf_auth_status_raw = selectScalars(cached.windsurf_auth_status_raw, ['email']);
  } else if (platform === 'claude_manager' && isClaudeDesktopOAuthAccount(account as T & ClaudeAccount)) {
    cached.auth_mode = 'desktop_oauth';
  } else if (platform === 'trae') {
    const trae = account as T & TraeAccount;
    cached.trae_auth_raw = { platformId: getTraeAccountPlatformId(trae) };
    cached.nickname = getTraeAccountDisplayName(trae);
  }
  return cached as T;
}

/** A late startup cleanup must not overwrite a cache already refreshed by the host. */
export function replaceLegacyAccountDisplayCache(key: string, before: string, after: string): void {
  if (before === after) return;
  setTimeout(() => {
    try {
      if (localStorage.getItem(key) === before) localStorage.setItem(key, after);
    } catch {
      // The native account store remains authoritative when the cache is unavailable.
    }
  }, 0);
}
