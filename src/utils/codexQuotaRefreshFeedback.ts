import { proxyErrorKey } from '../services/codexAccountProxyService';
import { getLocalCodexProxyErrorCode } from './codexProxyRuntimeError';
import type { TFunction } from 'i18next';

type RefreshMessage = { text: string; tone?: 'error' | 'success' } | null;
export type CodexAccountRefreshMessage = { text: string; tone: 'error' | 'success' };
export type CodexAccountRefreshOutcome = { ok: true } | { ok: false; error: unknown };

function getLocalRefreshErrorKey(error: unknown): string | null {
  const code = getLocalCodexProxyErrorCode(error instanceof Error ? error.message : String(error));
  return code ? proxyErrorKey(code, 'probeFailed') : null;
}

export async function getCodexAccountRefreshFeedback(
  refresh: () => Promise<CodexAccountRefreshOutcome | undefined>,
  t: TFunction,
): Promise<CodexAccountRefreshMessage | null> {
  let outcome: CodexAccountRefreshOutcome | undefined;
  try {
    outcome = await refresh();
  } catch (error) {
    outcome = { ok: false, error };
  }
  if (!outcome) return null;
  if (outcome.ok) return { text: t('common.shared.refreshSuccess'), tone: 'success' };

  const localErrorKey = getLocalRefreshErrorKey(outcome.error);
  return {
    text: t('common.shared.refreshFailed', {
      error: localErrorKey
        ? t(localErrorKey)
        : String(outcome.error).replace(/^Error:\s*/, '').trim(),
    }),
    tone: 'error',
  };
}

/** Manual refresh must surface local failures without turning them into account errors. */
export async function refreshCodexQuotaWithFeedback(
  refresh: () => Promise<unknown>,
  translate: (key: string) => string,
  setMessage: (message: RefreshMessage) => void,
): Promise<void> {
  setMessage(null);
  try {
    await refresh();
  } catch (error) {
    const key = getLocalRefreshErrorKey(error);
    if (key) setMessage({ text: translate(key), tone: 'error' });
    // Existing callers still receive the failure; cached account/quota data stays untouched.
    throw error;
  }
}
