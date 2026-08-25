import type { CodexModelProvider } from '../services/codexModelProviderService';
import {
  OPENCODE_GO_API_PROVIDER_ID,
  resolveCodexApiProviderPresetId,
} from './codexProviderPresets.ts';

export type OpenCodeGoUsageErrorKind =
  | 'authentication'
  | 'rate_limit'
  | 'network'
  | 'unavailable';

export const OPEN_CODE_GO_CONNECTION_LIMIT = 4;

export type OpenCodeGoConnectionCapacity = {
  count: number;
  remaining: number;
  canAdd: boolean;
};

export function openCodeGoConnectionCapacity(
  connectionCount: number,
): OpenCodeGoConnectionCapacity {
  if (!Number.isInteger(connectionCount) || connectionCount < 0) {
    throw new Error('OPENCODE_GO_CONNECTION_COUNT_INVALID');
  }
  if (connectionCount > OPEN_CODE_GO_CONNECTION_LIMIT) {
    throw new Error('OPENCODE_GO_CONNECTION_LIMIT_EXCEEDED');
  }
  const remaining = OPEN_CODE_GO_CONNECTION_LIMIT - connectionCount;
  return { count: connectionCount, remaining, canAdd: remaining > 0 };
}

export function orderOpenCodeGoConnections<
  T extends { id: string; createdAt: number },
>(connections: readonly T[]): T[] {
  return [...connections].sort(
    (left, right) =>
      left.createdAt - right.createdAt || left.id.localeCompare(right.id),
  );
}

export function findOpenCodeGoProvider(
  providers: CodexModelProvider[],
): CodexModelProvider | null {
  return (
    providers.find(
      (provider) =>
        resolveCodexApiProviderPresetId(provider.baseUrl) ===
        OPENCODE_GO_API_PROVIDER_ID,
    ) ?? null
  );
}

/**
 * Redacted shape of an OpenCode Go provider API key used in transfer bundles:
 * key material is never exported, only a display hint and the key id so a
 * same-machine import can re-attach the local credential.
 */
export type OpenCodeGoRedactedApiKey = {
  id: string;
  apiKey: string;
  name?: string;
  createdAt?: number;
  updatedAt?: number;
};

/** Marker prefix proving an exported key string is a redaction, not material. */
export const OPEN_CODE_GO_REDACTED_PREFIX = 'redacted:';

/** True unless the value is empty or already carries the redacted marker. */
export function isPlaintextOpenCodeGoApiKey(value: unknown): boolean {
  if (typeof value !== 'string') return false;
  const trimmed = value.trim();
  return (
    trimmed.length > 0 &&
    !trimmed.startsWith(OPEN_CODE_GO_REDACTED_PREFIX)
  );
}

/** Replace key material with `redacted:<hint>` for export bundles. */
export function redactOpenCodeGoApiKey(
  apiKey: OpenCodeGoRedactedApiKey,
): OpenCodeGoRedactedApiKey {
  return {
    ...apiKey,
    apiKey: `${OPEN_CODE_GO_REDACTED_PREFIX}${maskOpenCodeGoApiKey(apiKey.apiKey)}`,
  };
}

/** Keep only non-secret fields when importing a redacted provider key. */
export function stripOpenCodeGoKeyMaterial(
  apiKey: OpenCodeGoRedactedApiKey,
): OpenCodeGoRedactedApiKey {
  return {
    id: typeof apiKey?.id === 'string' ? apiKey.id : '',
    apiKey:
      typeof apiKey?.apiKey === 'string'
        ? OPEN_CODE_GO_REDACTED_PREFIX
        : '',
  };
}

/**
 * Sanitize a Codex model provider for config-transfer export. OpenCode Go
 * providers have their connection keys redacted; other providers pass through
 * unchanged (their handling is owned by the codex platform scope).
 */
export function sanitizeOpenCodeGoProviderForTransfer<
  T extends {
    baseUrl: string;
    apiKeys: OpenCodeGoRedactedApiKey[];
  },
>(provider: T): T {
  if (findOpenCodeGoProvider([provider as never]) === null) {
    return provider;
  }
  return {
    ...provider,
    apiKeys: provider.apiKeys.map(redactOpenCodeGoApiKey),
  };
}

export function maskOpenCodeGoApiKey(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return '';
  if (trimmed.length <= 8) return `${trimmed.slice(0, 2)}****`;
  return `${trimmed.slice(0, 4)}****${trimmed.slice(-4)}`;
}

export function classifyOpenCodeGoUsageError(
  error: unknown,
): OpenCodeGoUsageErrorKind {
  const message = String(error).toUpperCase();
  if (message.includes('HTTP_401') || message.includes('HTTP_403')) {
    return 'authentication';
  }
  if (message.includes('HTTP_429')) return 'rate_limit';
  if (message.includes('NETWORK')) return 'network';
  return 'unavailable';
}
