import type { CodexProxyDesktopEntryStatus, CodexProxyRuntimeState, CodexProxyRuntimeStatus } from '../services/codexAccountProxyService';
import type { CodexAccount } from '../types/codex';

/** A UI deadline must not start duplicate native work when a previous read is stuck. */
export function singleFlightRead<T>(read: (key: string) => Promise<T>, timeoutMs = 10000) {
  const pending = new Map<string, Promise<T>>();
  return async (key: string): Promise<T> => {
    let work = pending.get(key);
    if (!work) {
      work = Promise.resolve().then(() => read(key));
      pending.set(key, work);
      const clear = () => { if (pending.get(key) === work) pending.delete(key); };
      void work.then(clear, clear);
    }
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      return await Promise.race([work, new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error('PROXY_PROBE_TIMEOUT')), timeoutMs);
      })]);
    } finally { clearTimeout(timer); }
  };
}

export interface ProxyRuntimeSnapshot { timestamp: number; status: CodexProxyRuntimeStatus }
export interface ProxyRuntimeRow {
  kind: 'account' | 'sidecar' | 'combined' | 'desktop';
  state: CodexProxyRuntimeState | 'ready' | 'failed' | 'entry_failed' | undefined;
  port: number | null | undefined;
  node: string | null | undefined;
  selection?: import('../services/codexAccountProxyService').CodexProxySelection | null;
  entry?: CodexProxyDesktopEntryStatus;
  kernelState?: CodexProxyRuntimeState;
  kernelPort?: number | null;
}

export function proxyRuntimeLabelKey(kind: ProxyRuntimeRow['kind']) {
  return kind === 'combined' ? 'codex.proxy.combinedRuntime' : kind === 'account' ? 'codex.proxy.runtimeMain'
    : kind === 'desktop' ? 'codex.proxy.runtimeDesktop' : 'codex.proxy.apiRuntime';
}

export function proxyPreviewBinding(saved: CodexAccount['egress_proxy'], status: CodexProxyRuntimeStatus | null) {
  if (status?.proxySource) {
    return { source: status.proxySource, summary: status.proxySource === 'none' ? null
      : status.effectiveProxy === undefined && status.proxySource === 'account' ? saved : status.effectiveProxy };
  }
  // Older hosts only returned runtime fields. A missing independent binding cannot
  // establish the effective source until the status request has completed.
  return { source: saved ? 'account' as const : status ? 'none' as const : 'unknown' as const, summary: saved };
}

export function proxyRuntimeRows(status: CodexProxyRuntimeStatus | null): ProxyRuntimeRow[] {
  // 账号通道与 API 服务通道展示内容一致时合并成一行：既包括「共用同一个代理」，也包括
  // 「两者都未绑定 / 都未启动」这类等价状态——未配置代理时不该出现两行完全相同的内容。
  const shared = !!status
    && status.account === status.sidecar
    && (status.accountPort ?? null) === (status.sidecarPort ?? null)
    && (status.accountNode ?? null) === (status.sidecarNode ?? null);
  return (shared ? ['combined', 'desktop'] as const : ['account', 'sidecar', 'desktop'] as const).map((kind) => {
    const source = kind === 'combined' ? 'account' : kind;
    const row: ProxyRuntimeRow = { kind, state: status?.[source], port: status?.[`${source}Port`], node: status?.[`${source}Node`] };
    const selection = status?.[`${source}Selection`];
    if (selection && selection.name === row.node) row.selection = selection;
    const entry = kind === 'desktop' ? status?.desktopEntry : null;
    if (entry) {
      row.entry = entry;
      row.kernelState = status?.desktop;
      row.kernelPort = status?.desktopPort;
      row.port = entry.port;
      if (entry.state !== 'listening') row.state = entry.state === 'failed' ? 'entry_failed' : 'stopped';
      // An entry listener does not repair a missing, stopped or starting kernel.
      else if (!['missing', 'stopped', 'starting', 'unbound'].includes(status!.desktop)) {
        if (entry.lastRequestState === 'failed') row.state = 'failed';
        else if (status!.desktop === 'idle' || status!.desktop === 'direct') row.state = entry.lastRequestState === 'connecting' ? 'starting' : 'ready';
      }
    }
    return row;
  });
}

export function proxyRuntimeChanges(current: CodexProxyRuntimeStatus, previous?: CodexProxyRuntimeStatus) {
  const old = proxyRuntimeRows(previous ?? null);
  return proxyRuntimeRows(current).filter((row) => !previous || !old.some((entry) => JSON.stringify(entry) === JSON.stringify(row)));
}

export function appendRuntimeSnapshot(history: ProxyRuntimeSnapshot[], status: CodexProxyRuntimeStatus, timestamp: number): ProxyRuntimeSnapshot[] {
  if (history[0] && proxyRuntimeChanges(status, history[0].status).length === 0) return history;
  return [{ timestamp, status }, ...history].slice(0, 20);
}
