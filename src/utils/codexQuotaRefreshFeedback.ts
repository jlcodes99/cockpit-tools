import { proxyErrorKey } from '../services/codexAccountProxyService';
import { getLocalCodexProxyErrorCode } from './codexProxyRuntimeError';

type RefreshMessage = { text: string; tone?: 'error' | 'success' } | null;

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
    const code = getLocalCodexProxyErrorCode(
      error instanceof Error ? error.message : String(error),
    );
    if (code) setMessage({ text: translate(proxyErrorKey(code, 'probeFailed')), tone: 'error' });
    // Existing callers still receive the failure; cached account/quota data stays untouched.
    throw error;
  }
}
