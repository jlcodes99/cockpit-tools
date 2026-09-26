import type { CodexAccount } from '../types/codex';
export interface ProxyFormDraft { protocol: string; host: string; port: string; username: string; password: string }
export const emptyProxyForm = (): ProxyFormDraft => ({ protocol: 'http', host: '', port: '', username: '', password: '' });
/** Never return userinfo, query parameters or embedded node credentials. */
export function proxySummary(summary: CodexAccount['egress_proxy']): string {
  if (!summary) return '';
  if (summary.name) return summary.sourceName ? `${summary.sourceName} · ${summary.name}` : summary.name;
  const protocol = summary.protocol.toUpperCase();
  const host = summary.server;
  if (!host) return protocol;
  const displayHost = host.includes(':') && !host.startsWith('[') ? `[${host}]` : host;
  return `${protocol} · ${displayHost}${summary.port ? `:${summary.port}` : ''}`;
}
export function proxyFormUrl(form: ProxyFormDraft): string | null {
  const host = form.host.trim();
  if (!['http', 'https', 'socks5', 'socks5h'].includes(form.protocol) || !host || /[\s\\/@?#]/.test(host)
    || !/^\d+$/.test(form.port) || Number(form.port) < 1 || Number(form.port) > 65535 || (form.password && !form.username)) return null;
  const address = host.includes(':') && !host.startsWith('[') ? `[${host}]` : host;
  try {
    const auth = form.username ? `${encodeURIComponent(form.username)}:${encodeURIComponent(form.password)}@` : '';
    const result = `${form.protocol}://${auth}${address}:${form.port}`;
    new URL(result);
    return result;
  } catch { return null; }
}

/** A hint only: users can override HTTPS subscription detection before any request. */
export function detectProxyInputKind(input: string): 'subscription' | 'manual' {
  const value = input.trim();
  if (/[\r\n]/.test(value)) return 'manual';
  try {
    const u = new URL(value);
    return u.protocol === 'https:' && !u.username && !u.password && !u.port && (u.pathname !== '/' || !!u.search) ? 'subscription' : 'manual';
  } catch { return 'manual'; }
}
