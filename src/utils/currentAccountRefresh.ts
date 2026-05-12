export const CURRENT_ACCOUNT_REFRESH_STORAGE_KEY = 'agtools.current_account_refresh_minutes.v1';
export const DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES = 1;
export const MIN_CURRENT_ACCOUNT_REFRESH_MINUTES = 1;
export const MAX_CURRENT_ACCOUNT_REFRESH_MINUTES = 999;

export type CurrentAccountRefreshPlatform =
  | 'antigravity'
  | 'codex'
  | 'ghcp'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'gemini'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'workbuddy'
  | 'qoder'
  | 'trae'
  | 'zed'
  | 'opencode';

export const CURRENT_ACCOUNT_REFRESH_PLATFORMS: CurrentAccountRefreshPlatform[] = [
  'antigravity',
  'codex',
  'ghcp',
  'windsurf',
  'kiro',
  'cursor',
  'gemini',
  'codebuddy',
  'codebuddy_cn',
  'workbuddy',
  'qoder',
  'trae',
  'zed',
  'opencode',
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
    ghcp: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    windsurf: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    kiro: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    cursor: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    gemini: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    codebuddy: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    codebuddy_cn: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    workbuddy: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    qoder: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    trae: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    zed: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
    opencode: DEFAULT_CURRENT_ACCOUNT_REFRESH_MINUTES,
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
