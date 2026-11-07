import { useEffect, useMemo, useState } from 'react';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import type { CodexAccount } from '../../types/codex';
import { getCodexProxyTarget, isCodexProxyTarget, peekCodexProxyTarget, subscribeCodexProxyTargets, type CodexProxyTargetAccount } from '../../services/codexProxyTargetService';

/** Read-only projections stay out of the real account store and its upstream proxy fields. */
export function useCodexProxyTargets(accounts: CodexAccount[]): CodexProxyTargetAccount[] {
  const [revision, setRevision] = useState(0);
  const refresh = () => setRevision((value) => value + 1);
  const storedAccounts = useCodexAccountStore((state) => state.accounts);
  const key = JSON.stringify(accounts.filter(isCodexProxyTarget).map((account) => {
    const boundId = account.bound_oauth_account_id ?? peekCodexProxyTarget(account.id)?.bound_oauth_account_id;
    const bound = storedAccounts.find((entry) => entry.id === boundId);
    return [account.id, boundId, bound?.egress_proxy, bound?.egress_proxy_disabled];
  }));
  useEffect(() => subscribeCodexProxyTargets(refresh), []);
  useEffect(() => {
    let live = true;
    const targets: [string, string?][] = JSON.parse(key);
    for (const [id] of targets) void getCodexProxyTarget(id).catch(() => { if (live) refresh(); });
    return () => { live = false; };
  }, [key]);
  return useMemo(() => accounts.map((account) => {
    if (!isCodexProxyTarget(account)) return account;
    const snapshot = peekCodexProxyTarget(account.id);
    return snapshot ? { ...account, egress_proxy: snapshot.egress_proxy ?? null, egress_proxy_url: undefined,
      egress_proxy_disabled: snapshot.egress_proxy_disabled ?? false, egress_proxy_error: snapshot.egress_proxy_error ?? null,
      egress_proxy_legacy_bound: snapshot.egress_proxy_legacy_bound ?? false,
      bound_oauth_account_id: account.bound_oauth_account_id ?? snapshot.bound_oauth_account_id,
      proxyTargetLoading: snapshot.proxyTargetLoading, proxyTargetError: snapshot.proxyTargetError } : {
      ...account, egress_proxy: null, egress_proxy_url: undefined, egress_proxy_disabled: false,
      proxyTargetLoading: true, proxyTargetError: false,
    };
  }), [accounts, revision]);
}
