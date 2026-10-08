import type { CodexAccount } from '../types/codex';
import type { CodexProxyRuntimeStatus } from '../services/codexAccountProxyService';
import { proxyPreviewBinding, proxyRuntimeRows } from './codexProxyPreview';

export type CodexProxyDisplay = 'compact' | 'summary' | 'detailed';
export const CODEX_PROXY_DISPLAY_KEY = 'agtools.codex_proxy_display';
export const CODEX_PROXY_DISPLAY_EVENT = 'agtools:codex-proxy-display-changed';
export const CODEX_PROXY_DISPLAYS: readonly CodexProxyDisplay[] = ['compact', 'summary', 'detailed'];

export function getCodexProxyDisplay(): CodexProxyDisplay {
  try {
    const value = localStorage.getItem(CODEX_PROXY_DISPLAY_KEY);
    if (CODEX_PROXY_DISPLAYS.includes(value as CodexProxyDisplay)) return value as CodexProxyDisplay;
  } catch { /* Unavailable storage retains the existing compact layout. */ }
  return 'compact';
}

export function persistCodexProxyDisplay(value: CodexProxyDisplay): boolean {
  if (!CODEX_PROXY_DISPLAYS.includes(value)) return false;
  try {
    localStorage.setItem(CODEX_PROXY_DISPLAY_KEY, value);
    window.dispatchEvent(new Event(CODEX_PROXY_DISPLAY_EVENT));
    return true;
  } catch (error) {
    console.warn('[Codex proxy display] Could not save preference', error);
    return false;
  }
}

/** A configured group is not evidence of a running leaf or a successful connection. */
export function codexProxyCardPresentation(account: CodexAccount, status: CodexProxyRuntimeStatus | null) {
  const binding = account.egress_proxy_disabled
    ? { source: 'disabled' as const, summary: null }
    : proxyPreviewBinding(account.egress_proxy, status);
  const modeKey = binding.source === 'disabled' ? 'codex.proxy.modeDisabled'
    : binding.source === 'account' ? 'codex.proxy.modeIndependent'
      : binding.source === 'unified' ? 'codex.proxy.modeUnified'
        : binding.source === 'none' ? 'codex.proxy.modeDefault' : 'codex.proxy.managerAccounts.following';
  const rows = proxyRuntimeRows(status);
  const live = binding.source === 'disabled' || binding.source === 'none' ? []
    : rows.filter((row) => (row.kernelState ?? row.state) === 'running' && row.node);
  const nodes = [...new Set(live.map((row) => row.node!))].map((name) => {
    const matching = live.filter((row) => row.node === name);
    const selection = (['account', 'desktop', 'sidecar'] as const).flatMap((kind) => {
      const measured = status?.[`${kind}Selection`];
      return status?.[kind] === 'running' && status[`${kind}Node`] === name && measured?.name === name ? [measured] : [];
    })
      .sort((a, b) => (b.checkedAt ?? 0) - (a.checkedAt ?? 0))[0];
    return { name, selection, kinds: matching.map((row) => row.kind) };
  });
  return { modeKey, saved: binding.summary, nodes, rows };
}
