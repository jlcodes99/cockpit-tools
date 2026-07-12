export const CURRENT_ACCOUNT_REFRESH_STORAGE_KEY = 'agtools.current_account_refresh_minutes.v1';
export const DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES = 1;
export const MIN_CURRENT_ACCOUNT_REFRESH_MINUTES = 1;
export const MAX_CURRENT_ACCOUNT_REFRESH_MINUTES = 999;

export type CurrentAccountRefreshPlatform =
  | 'antigravity'
  | 'codex'
  | 'claude'
  | 'ghcp'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'gemini'
  | 'grok'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'workbuddy'
  | 'qoder'
  | 'zcode'
  | 'trae'
  | 'trae_solo'
  | 'trae_cn'
  | 'trae_solo_cn'
  | 'zed';

export const CURRENT_ACCOUNT_REFRESH_PLATFORMS: CurrentAccountRefreshPlatform[] = [
  'antigravity',
  'codex',
  'claude',
  'ghcp',
  'windsurf',
  'kiro',
  'cursor',
  'gemini',
  'grok',
  'codebuddy',
  'codebuddy_cn',
  'workbuddy',
  'qoder',
  'zcode',
  'trae',
  'trae_solo',
  'trae_cn',
  'trae_solo_cn',
  'zed',
];

export type CurrentAccountRefreshMinutesMap = Record<CurrentAccountRefreshPlatform, number>;

export function sanitizeCurrentAccountRefreshMinutes(value: unknown): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) {
    return DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES;
  }
  const normalized = Math.floor(parsed);
  if (normalized < MIN_CURRENT_ACCOUNT_REFRESH_MINUTES) {
    return MIN_CURRENT_ACCOUNT_REFRESH_MINUTES;
  }
  if (normalized > MAX_CURRENT_ACCOUNT_REFRESH_MINUTES) {
    return MAX_CURRENT_ACCOUNT_REFRESH_MINUTES;
  }
  return normalized;
}

export function buildDefaultCurrentAccountRefreshMinutesMap(): CurrentAccountRefreshMinutesMap {
  return {
    antigravity: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    codex: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    claude: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    ghcp: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    windsurf: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    kiro: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    cursor: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    gemini: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    grok: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    codebuddy: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    codebuddy_cn: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    workbuddy: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    qoder: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    zcode: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    trae: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    trae_solo: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    trae_cn: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    trae_solo_cn: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    zed: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
  };
}

function normalizeCurrentAccountRefreshMinutesMap(
  raw?: Partial<Record<CurrentAccountRefreshPlatform, unknown>> | null,
): CurrentAccountRefreshMinutesMap {
  const defaults = buildDefaultCurrentAccountRefreshMinutesMap();
  if (!raw) {
    return defaults;
  }

  const next = { ...defaults };
  for (const platform of CURRENT_ACCOUNT_REFRESH_PLATFORMS) {
    next[platform] = sanitizeCurrentAccountRefreshMinutes(raw[platform]);
  }
  return next;
}

export function loadCurrentAccountRefreshMinutesMap(): CurrentAccountRefreshMinutesMap {
  try {
    const raw = localStorage.getItem(CURRENT_ACCOUNT_REFRESH_STORAGE_KEY);
    if (!raw) {
      return buildDefaultCurrentAccountRefreshMinutesMap();
    }
    const parsed = JSON.parse(raw) as Partial<Record<CurrentAccountRefreshPlatform, unknown>>;
    return normalizeCurrentAccountRefreshMinutesMap(parsed);
  } catch {
    return buildDefaultCurrentAccountRefreshMinutesMap();
  }
}

export function saveCurrentAccountRefreshMinutesMap(
  raw: Partial<Record<CurrentAccountRefreshPlatform, unknown>>,
): CurrentAccountRefreshMinutesMap {
  const normalized = normalizeCurrentAccountRefreshMinutesMap(raw);
  try {
    localStorage.setItem(CURRENT_ACCOUNT_REFRESH_STORAGE_KEY, JSON.stringify(normalized));
  } catch {
    // 忽略持久化失败，保持运行时可用
  }
  return normalized;
}

export const ACCOUNT_REFRESH_OVERRIDES_KEY = 'agtools.account_refresh_overrides.v1';

export type AccountRefreshOverrides = Partial<Record<CurrentAccountRefreshPlatform, Record<string, number>>>;

export function loadAccountRefreshOverrides(): AccountRefreshOverrides {
  try {
    const raw = localStorage.getItem(ACCOUNT_REFRESH_OVERRIDES_KEY);
    if (!raw) {
      return {};
    }
    return JSON.parse(raw) as AccountRefreshOverrides;
  } catch {
    return {};
  }
}

export function saveAccountRefreshOverrides(overrides: AccountRefreshOverrides): void {
  try {
    localStorage.setItem(ACCOUNT_REFRESH_OVERRIDES_KEY, JSON.stringify(overrides));
  } catch {
    // 忽略持久化失败
  }
}

export function getAccountRefreshMinutes(
  platform: CurrentAccountRefreshPlatform,
  email: string,
  platformDefault: number,
): number {
  const overrides = loadAccountRefreshOverrides();
  const platformOverrides = overrides[platform];
  if (platformOverrides && email in platformOverrides) {
    const value = platformOverrides[email];
    if (value === -1) return -1;
    return sanitizeCurrentAccountRefreshMinutes(value);
  }
  return platformDefault;
}

export function setAccountRefreshMinutes(
  platform: CurrentAccountRefreshPlatform,
  email: string,
  minutes: number,
): void {
  // 验证输入：-1 表示禁用，其他值需要在有效范围内
  if (minutes !== -1) {
    if (!Number.isFinite(minutes) || minutes < MIN_CURRENT_ACCOUNT_REFRESH_MINUTES || minutes > MAX_CURRENT_ACCOUNT_REFRESH_MINUTES) {
      return;
    }
  }
  const overrides = loadAccountRefreshOverrides();
  if (!overrides[platform]) {
    overrides[platform] = {};
  }
  overrides[platform][email] = minutes;
  saveAccountRefreshOverrides(overrides);
}

export function removeAccountRefreshOverride(
  platform: CurrentAccountRefreshPlatform,
  email: string,
): void {
  const overrides = loadAccountRefreshOverrides();
  if (overrides[platform] && email in overrides[platform]) {
    delete overrides[platform][email];
    if (Object.keys(overrides[platform]).length === 0) {
      delete overrides[platform];
    }
    saveAccountRefreshOverrides(overrides);
  }
}
