import { useEffect, useRef, useState } from 'react';
import { getCodexProxyRuntimeStatus, getCodexAccountProxyRecentRequests, type CodexProxyRecentRequest, type CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';
import { codexProxyStatusEvents } from '../../utils/codexProxyStatusEvents';
import { appendRuntimeSnapshot, singleFlightRead, type ProxyRuntimeSnapshot } from '../../utils/codexProxyPreview';

const readStatus = singleFlightRead((key) => getCodexProxyRuntimeStatus(JSON.parse(key)[0]));
const readRequests = singleFlightRead(getCodexAccountProxyRecentRequests);

function emptyPreview(accountId: string, bindingKey: string) {
  return { accountId, bindingKey, status: null as CodexProxyRuntimeStatus | null, history: [] as ProxyRuntimeSnapshot[],
    requests: [] as CodexProxyRecentRequest[], statusError: false, requestsError: false, loading: true };
}

export function useCodexProxyPreview(accountId: string, bindingKey = '') {
  const [revision, setRevision] = useState(0);
  const [data, setData] = useState(() => emptyPreview(accountId, bindingKey));
  const refreshPending = useRef(false);
  // Effects run after a render. Account-owned state prevents even that first render
  // from presenting the previous account's ports, request history or source.
  const current = data.accountId === accountId && data.bindingKey === bindingKey ? data : emptyPreview(accountId, bindingKey);

  useEffect(() => {
    let disposed = false;
    let generation = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    refreshPending.current = true;
    setData((old) => old.accountId === accountId && old.bindingKey === bindingKey ? { ...old, loading: true, statusError: false, requestsError: false } : emptyPreview(accountId, bindingKey));
    const update = (patch: Partial<typeof data>) => {
      if (!disposed) setData((old) => old.accountId === accountId && old.bindingKey === bindingKey ? { ...old, ...patch } : old);
    };
    const refreshStatus = async () => {
      const current = generation;
      try {
        const next = await readStatus(JSON.stringify([accountId, bindingKey, revision, codexProxyStatusEvents.revision(accountId)]));
        if (!disposed && current === generation) setData((old) => old.accountId === accountId && old.bindingKey === bindingKey ? {
          ...old, status: next, statusError: false, history: appendRuntimeSnapshot(old.history, next, Date.now()),
        } : old);
      } catch { if (current === generation) update({ status: null, statusError: true }); }
      finally { if (!disposed && current === generation) timer = setTimeout(refreshStatus, 5000); }
    };
    const unsubscribe = codexProxyStatusEvents.subscribe((event) => {
      if (event.accountId !== accountId || disposed) return;
      generation++;
      clearTimeout(timer);
      setData((old) => old.accountId === accountId && old.bindingKey === bindingKey ? { ...old, status: event.status,
        statusError: !event.status, history: event.status ? appendRuntimeSnapshot(old.history, event.status, Date.now()) : old.history } : old);
      timer = setTimeout(refreshStatus, 5000);
    });
    const refreshRequests = async () => {
      // Historical API requests remain useful for accounts following the unified
      // proxy too. An independent binding is not a prerequisite for this read.
      try { update({ requests: await readRequests(accountId), requestsError: false }); }
      catch { update({ requestsError: true }); }
    };
    void Promise.all([refreshStatus(), refreshRequests()]).finally(() => {
      if (!disposed) { update({ loading: false }); refreshPending.current = false; }
    });
    return () => { disposed = true; unsubscribe(); if (timer) clearTimeout(timer); };
  }, [accountId, bindingKey, revision]);

  return { ...current, refresh: () => {
    if (refreshPending.current || current.loading) return;
    refreshPending.current = true;
    setRevision((old) => old + 1);
  } };
}
