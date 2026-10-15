import { useCallback, useEffect } from 'react';
import { getMfaOtpToken, rememberMfaQuery } from '../utils/mfaVault';

interface RememberMfaQueryOptions {
  enabled: boolean;
  secret: string;
  accountName?: string | null;
  remark?: string | null;
}

/**
 * 记录能够生成验证码的用户输入。短暂防抖避免逐字输入时保存中间秘钥，
 * 返回的 flush 用于关闭或提交弹框前立即保存最终输入。
 */
export function useRememberMfaQuery({
  enabled,
  secret,
  accountName,
  remark,
}: RememberMfaQueryOptions): () => void {
  const remember = useCallback(() => {
    const normalizedSecret = secret.trim();
    if (!enabled || !normalizedSecret || !getMfaOtpToken(normalizedSecret)) return;
    rememberMfaQuery({ secret: normalizedSecret, accountName, remark });
  }, [accountName, enabled, remark, secret]);

  useEffect(() => {
    const timer = window.setTimeout(remember, 500);
    return () => window.clearTimeout(timer);
  }, [remember]);

  return remember;
}
