import { useCallback, useEffect, useRef, useState } from 'react';
import { getAntigravityCliStatus, type AntigravityCliStatus } from '../services/antigravityCliService';

export type AntigravityCliState =
  | 'loading'
  | 'error'
  | 'keyringError'
  | 'apiKey'
  | 'managed'
  | 'notImported'
  | 'signedOut';

/** 以钥匙环中的实际凭据判断 CLI 登录态；读取失败不能被当成已登出。 */
export function resolveAntigravityCliState(
  status: AntigravityCliStatus | null,
  error: string | null,
): AntigravityCliState {
  if (error) return 'error';
  if (!status) return 'loading';
  if (status.credential_error) return 'keyringError';
  if (status.api_key_mode) return 'apiKey';
  if (status.current_account_id) return 'managed';
  if (status.credential_present) return 'notImported';
  return 'signedOut';
}

export interface AntigravityCliStatusController {
  status: AntigravityCliStatus | null;
  state: AntigravityCliState;
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * 读取 agy 安装与登录状态。当前账号变化或窗口重新获得焦点时重新检查，
 * 因为用户可能在终端中执行了 /logout 或重新登录。
 */
export function useAntigravityCliStatus(
  enabled: boolean,
  currentAccountId: string | null,
  onWindowFocus?: () => void,
): AntigravityCliStatusController {
  const [status, setStatus] = useState<AntigravityCliStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestId = useRef(0);
  const onWindowFocusRef = useRef(onWindowFocus);
  onWindowFocusRef.current = onWindowFocus;

  const refresh = useCallback(async () => {
    const id = ++requestId.current;
    setLoading(true);
    setError(null);
    try {
      const next = await getAntigravityCliStatus();
      if (id === requestId.current) setStatus(next);
    } catch (e) {
      if (id === requestId.current) {
        setError(String(e));
        setStatus(null);
      }
    } finally {
      if (id === requestId.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!enabled) return;
    void refresh();
    const handleFocus = () => {
      void refresh();
      onWindowFocusRef.current?.();
    };
    window.addEventListener('focus', handleFocus);
    return () => {
      requestId.current += 1;
      window.removeEventListener('focus', handleFocus);
    };
  }, [enabled, currentAccountId, refresh]);

  return {
    status,
    state: resolveAntigravityCliState(status, error),
    loading,
    error,
    refresh,
  };
}
