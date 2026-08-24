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
