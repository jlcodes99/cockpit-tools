import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import {
  getCodexAccountProxyRecentRequests,
  getCodexProxyRuntimeStatus,
  type CodexProxyRecentRequest,
  type CodexProxyRuntimeStatus,
} from '../../services/codexAccountProxyService';
import { codexProxyStatusEvents } from '../../utils/codexProxyStatusEvents';
import type { CodexAccount } from '../../types/codex';
import { createProxyCardReadCache, createProxyCardReadPool, PROXY_CARD_REFRESH_MS, type ProxyCardReadErrorKind } from '../../utils/codexProxyCardReads';

const pool = createProxyCardReadPool();
const statusReads = createProxyCardReadCache(getCodexProxyRuntimeStatus, pool);
// Invalidate unmounted/offscreen snapshots as well, without reading the host.
codexProxyStatusEvents.subscribe(({ accountId }) => statusReads.invalidateAccount(accountId));
const requestReads = createProxyCardReadCache(async (accountId: string) => {
  const requests = await getCodexAccountProxyRecentRequests(accountId);
  return requests.reduce<CodexProxyRecentRequest | null>((latest, item) =>
    !latest || item.timestamp > latest.timestamp ? item : latest, null);
}, pool);

interface CardSnapshot {
  key: string;
  detailed: boolean;
  status: CodexProxyRuntimeStatus | null;
  request: CodexProxyRecentRequest | null;
  statusError: boolean;
  statusErrorKind?: ProxyCardReadErrorKind;
  requestsError: boolean;
  loading: boolean;
}

function cachedSnapshot(key: string, detailed: boolean): CardSnapshot {
  const status = statusReads.peek(key);
  const request = detailed ? requestReads.peek(key) : null;
  return { key, detailed, status: status.value, request: request?.value ?? null,
    statusError: status.error, statusErrorKind: status.errorKind, requestsError: request?.error ?? false, loading: !statusReads.has(key) || (detailed && !requestReads.has(key)) };
}

export function useCodexProxyCardData(account: CodexAccount, detailed: boolean): {
  ref: RefObject<HTMLDivElement | null>;
  status: CodexProxyRuntimeStatus | null;
  request: CodexProxyRecentRequest | null;
  statusError: boolean;
  statusErrorKind?: ProxyCardReadErrorKind;
  requestsError: boolean;
  loading: boolean;
  refresh: () => void;
} {
  const ref = useRef<HTMLDivElement>(null);
  // Include legacy direct bindings as well as current structured bindings.
  const key = JSON.stringify([account.id, account.egress_proxy_disabled ?? false,
    account.egress_proxy ?? null, account.egress_proxy_url ?? null]);
  const [snapshot, setSnapshot] = useState<CardSnapshot>(() => cachedSnapshot(key, detailed));
  const refreshRef = useRef<() => void>(() => {});
  const refresh = useCallback(() => refreshRef.current(), []);

  useEffect(() => {
    let disposed = false;
    let visible = false;
    let running: number | null = null;
    let generation = 0;
    let timer: ReturnType<typeof setInterval> | undefined;
    const active = () => !disposed && visible && document.visibilityState !== 'hidden';
    setSnapshot(cachedSnapshot(key, detailed));
    const read = async (force = false) => {
      if (!active() || running !== null) return;
      const current = ++generation;
      running = current;
      const valid = () => active() && generation === current;
      setSnapshot((previous) => ({ ...previous, loading: true }));
      // Publish status independently: a slow request log must not hide a fresh status.
      const status = statusReads.read(key, valid, force).then((result) => {
        if (valid()) setSnapshot((previous) => ({ ...previous, status: result.value, statusError: result.error, statusErrorKind: result.errorKind }));
      });
      const request = detailed ? requestReads.read(key, valid, force).then((result) => {
        if (valid()) setSnapshot((previous) => ({ ...previous, request: result.value, requestsError: result.error }));
      }) : Promise.resolve();
      await Promise.all([status, request]);
      if (running !== current) return;
      running = null;
      if (!disposed) setSnapshot((previous) => ({ ...previous, loading: cachedSnapshot(key, detailed).loading }));
      // A card may re-enter while the obsolete request is still settling.
      if (active() && current !== generation) void read();
    };
    const unsubscribe = codexProxyStatusEvents.subscribe((event) => {
      if (event.accountId !== account.id || disposed) return;
      generation++;
      running = null;
      // Clear a stale success even while offscreen; no native read until visible.
      setSnapshot((previous) => ({ ...previous, status: event.status, statusError: false, statusErrorKind: undefined }));
      if (active()) void read(true);
    });
    const updatePolling = () => {
      if (disposed) return;
      clearInterval(timer);
      if (active()) {
        void read();
        timer = setInterval(() => { void read(); }, PROXY_CARD_REFRESH_MS);
      } else {
        generation++;
        setSnapshot((previous) => ({ ...previous, loading: cachedSnapshot(key, detailed).loading }));
      }
    };
    const observer = typeof IntersectionObserver === 'undefined' ? null : new IntersectionObserver((entries) => {
      visible = entries.some((entry) => entry.isIntersecting);
      updatePolling();
    });
    if (observer && ref.current) observer.observe(ref.current);
    // Older WebViews without observers still have a visible document fallback.
    if (!observer) { visible = true; updatePolling(); }
    document.addEventListener('visibilitychange', updatePolling);
    refreshRef.current = () => { void read(true); };
    return () => {
      disposed = true;
      unsubscribe();
      generation++;
      clearInterval(timer);
      observer?.disconnect();
      document.removeEventListener('visibilitychange', updatePolling);
      refreshRef.current = () => {};
    };
  }, [key, detailed]);

  // Never expose an old account/binding between render and effect cleanup.
  const current = snapshot.key === key && snapshot.detailed === detailed ? snapshot : cachedSnapshot(key, detailed);
  return { ref, status: current.status, request: detailed ? current.request : null,
    statusError: current.statusError, statusErrorKind: current.statusErrorKind, requestsError: detailed && current.requestsError,
    loading: current.loading, refresh };
}
