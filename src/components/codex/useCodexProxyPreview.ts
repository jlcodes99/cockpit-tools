import { useEffect, useRef, useState } from 'react';
import { getCodexProxyRuntimeStatus, getCodexAccountProxyRecentRequests, type CodexProxyRecentRequest, type CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';
import { appendRuntimeSnapshot, singleFlightRead, type ProxyRuntimeSnapshot } from '../../utils/codexProxyPreview';

const readStatus = singleFlightRead(getCodexProxyRuntimeStatus);
const readRequests = singleFlightRead(getCodexAccountProxyRecentRequests);

function emptyPreview(accountId: string) {
  return { accountId, status: null as CodexProxyRuntimeStatus | null, history: [] as ProxyRuntimeSnapshot[],
    requests: [] as CodexProxyRecentRequest[], statusError: false, requestsError: false, loading: true };
}

export function useCodexProxyPreview(accountId: string) {
  const [revision, setRevision] = useState(0);
  const [data, setData] = useState(() => emptyPreview(accountId));
  const refreshPending = useRef(false);
  // Effects run after a render. Account-owned state prevents even that first render
  // from presenting the previous account's ports, request history or source.
  const current = data.accountId === accountId ? data : emptyPreview(accountId);

  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    refreshPending.current = true;
    setData((old) => old.accountId === accountId ? { ...old, loading: true, statusError: false, requestsError: false } : emptyPreview(accountId));
    const update = (patch: Partial<typeof data>) => {
      if (!disposed) setData((old) => old.accountId === accountId ? { ...old, ...patch } : old);
    };
    const refreshStatus = async () => {
      try {
        const next = await readStatus(accountId);
        if (!disposed) setData((old) => old.accountId === accountId ? {
          ...old, status: next, statusError: false, history: appendRuntimeSnapshot(old.history, next, Date.now()),
        } : old);
      } catch { update({ status: null, statusError: true }); }
      finally { if (!disposed) timer = setTimeout(refreshStatus, 5000); }
    };
    const refreshRequests = async () => {
      // Historical API requests remain useful for accounts following the unified
      // proxy too. An independent binding is not a prerequisite for this read.
      try { update({ requests: await readRequests(accountId), requestsError: false }); }
      catch { update({ requestsError: true }); }
    };
    void Promise.all([refreshStatus(), refreshRequests()]).finally(() => {
      if (!disposed) { update({ loading: false }); refreshPending.current = false; }
    });
    return () => { disposed = true; if (timer) clearTimeout(timer); };
  }, [accountId, revision]);

  return { ...current, refresh: () => {
    if (refreshPending.current || current.loading) return;
    refreshPending.current = true;
    setRevision((old) => old + 1);
  } };
}
