import type { Page } from '../types/navigation';
import { PLATFORM_PAGE_MAP, type PlatformId } from '../types/platform';

export const EXTERNAL_PROVIDER_IMPORT_EVENT = 'app:provider-import';

export type ExternalProviderImportPayload = {
  providerId: PlatformId;
  page: Page;
  token: string;
  importUrl?: string | null;
  minAppVersion?: string | null;
  autoImport: boolean;
  activate: boolean;
  source?: string | null;
  rawUrl?: string | null;
};

type RawExternalProviderImportPayload = {
  providerId?: unknown;
  provider?: unknown;
  platform?: unknown;
  target?: unknown;
  page?: unknown;
  token?: unknown;
  importToken?: unknown;
  payload?: unknown;
  importPayload?: unknown;
  importUrl?: unknown;
  import_url?: unknown;
  minAppVersion?: unknown;
  min_app_version?: unknown;
  autoImport?: unknown;
  autoSubmit?: unknown;
  activate?: unknown;
  autoActivate?: unknown;
  source?: unknown;
  rawUrl?: unknown;
  url?: unknown;
};

function isJsonLikePayload(raw: string): boolean {
  const trimmed = raw.trim();
  if (!trimmed) return false;
  if (
    !(trimmed.startsWith('{') && trimmed.endsWith('}')) &&
    !(trimmed.startsWith('[') && trimmed.endsWith(']'))
  ) {
    return false;
  }
  try {
    const parsed = JSON.parse(trimmed) as unknown;
    return Boolean(parsed && typeof parsed === 'object');
  } catch {
    return false;
  }
}

const IMPORT_TARGET_PAGES: ReadonlySet<Page> = new Set<Page>([
  'overview',
  'codex',
  'github-copilot',
  'windsurf',
  'kiro',
  'cursor',
  'gemini',
  'codebuddy',
  'codebuddy-cn',
  'qoder',
  'trae',
  'workbuddy',
  'zed',
]);

const PROVIDER_ALIAS_MAP: Record<string, PlatformId> = {
  antigravity: 'antigravity',
  overview: 'antigravity',
  accounts: 'antigravity',
  codex: 'codex',
  zed: 'zed',
  github_copilot: 'github-copilot',
  githubcopilot: 'github-copilot',
  ghcp: 'github-copilot',
  windsurf: 'windsurf',
  kiro: 'kiro',
  cursor: 'cursor',
  gemini: 'gemini',
  codebuddy: 'codebuddy',
  codebuddy_cn: 'codebuddy_cn',
  codebuddycn: 'codebuddy_cn',
  qoder: 'qoder',
  trae: 'trae',
  workbuddy: 'workbuddy',
};

let pendingExternalProviderImport: ExternalProviderImportPayload | null = null;

function normalizeAliasKey(raw: string): string {
  return raw
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '');
}

function resolveProviderId(raw: unknown): PlatformId | null {
  if (typeof raw !== 'string') return null;
  const key = normalizeAliasKey(raw);
  if (!key) return null;
  return PROVIDER_ALIAS_MAP[key] ?? null;
}

function parseBooleanLike(value: unknown): boolean {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'number') return value > 0;
  if (typeof value !== 'string') return false;
  const normalized = value.trim().toLowerCase();
  return normalized === '1' || normalized === 'true' || normalized === 'yes' || normalized === 'on';
}

function readString(raw: unknown): string | null {
  if (typeof raw !== 'string') return null;
  const trimmed = raw.trim();
  return trimmed ? trimmed : null;
}

function resolvePage(providerId: PlatformId, rawPage: unknown): Page {
  const candidate = readString(rawPage);
  if (candidate && IMPORT_TARGET_PAGES.has(candidate as Page)) {
    return candidate as Page;
  }
  return PLATFORM_PAGE_MAP[providerId];
}

export function normalizeAntigravityExternalImportToken(rawToken: string): string {
  const trimmed = rawToken.trim();
  if (!trimmed) return '';
  if (isJsonLikePayload(trimmed)) return trimmed;
  if (trimmed.startsWith('1//')) {
    return JSON.stringify({ refresh_token: trimmed });
  }
  return trimmed;
}

export function normalizeExternalProviderImportPayload(
  raw: unknown,
): ExternalProviderImportPayload | null {
  if (!raw || typeof raw !== 'object') return null;

  const payload = raw as RawExternalProviderImportPayload;
  const providerId = resolveProviderId(
    payload.providerId ?? payload.provider ?? payload.platform ?? payload.target,
  );
  if (!providerId) return null;

  const token =
    readString(
      payload.token ?? payload.importToken ?? payload.payload ?? payload.importPayload,
    ) ?? '';
  const importUrl = readString(payload.importUrl ?? payload.import_url);
  if (!token && !importUrl) return null;
  const minAppVersion =
    readString(payload.minAppVersion ?? payload.min_app_version)?.replace(/^v/i, '') ?? null;

  const page =
    providerId === 'antigravity' ? 'overview' : resolvePage(providerId, payload.page);

  console.info('[ExternalImport][Utils] payload 归一化结果', {
    providerId,
    page,
    autoImport: parseBooleanLike(payload.autoImport ?? payload.autoSubmit),
    tokenLength: token.length,
    hasImportUrl: Boolean(importUrl),
    minAppVersion,
  });

  return {
    providerId,
    page,
    token,
    importUrl,
    minAppVersion,
    autoImport: parseBooleanLike(payload.autoImport ?? payload.autoSubmit),
    activate: parseBooleanLike(payload.activate ?? payload.autoActivate),
    source: readString(payload.source),
    rawUrl: readString(payload.rawUrl ?? payload.url),
  };
}

export function queueExternalProviderImport(payload: ExternalProviderImportPayload): void {
  console.info('[ExternalImport][Utils] 写入队列', {
    providerId: payload.providerId,
    page: payload.page,
    autoImport: payload.autoImport,
    tokenLength: payload.token.length,
    minAppVersion: payload.minAppVersion ?? null,
  });
  pendingExternalProviderImport = payload;
}

export function consumeQueuedExternalProviderImportForPlatform(
  platformId: PlatformId,
): ExternalProviderImportPayload | null {
  if (!pendingExternalProviderImport) {
    console.info('[ExternalImport][Utils] 消费队列: 当前为空', { platformId });
    return null;
  }
  if (pendingExternalProviderImport.providerId !== platformId) {
    console.info('[ExternalImport][Utils] 消费队列: 平台不匹配', {
      platformId,
      queuedProviderId: pendingExternalProviderImport.providerId,
    });
    return null;
  }
  const payload = pendingExternalProviderImport;
  pendingExternalProviderImport = null;
  console.info('[ExternalImport][Utils] 消费队列成功', {
    platformId,
    page: payload.page,
    autoImport: payload.autoImport,
    tokenLength: payload.token.length,
  });
  return payload;
}

export function dispatchExternalProviderImportEvent(payload: ExternalProviderImportPayload): void {
  queueExternalProviderImport(payload);
  window.dispatchEvent(
    new CustomEvent<ExternalProviderImportPayload>(EXTERNAL_PROVIDER_IMPORT_EVENT, {
      detail: payload,
    }),
  );
}
