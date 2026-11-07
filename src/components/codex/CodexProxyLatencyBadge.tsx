import { LoaderCircle } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { catalogErrorKey } from '../../services/codexProxyCatalogService';
import type { LatencyState } from '../../utils/codexProxyLatency';

export function proxyLatencyTone(result: LatencyState | undefined): 'muted' | 'loading' | 'good' | 'medium' | 'slow' | 'error' {
  if (result?.status === 'queued' || result?.status === 'running') return 'loading';
  if (result?.status === 'error') return 'error';
  if (result?.status !== 'success') return 'muted';
  if (!Number.isFinite(result.value.latencyMs) || result.value.latencyMs < 0) return 'error';
  return result.value.latencyMs < 250 ? 'good' : result.value.latencyMs < 400 ? 'medium' : 'slow';
}

/** The engine's delay is shown without mixing it with a separate HTTP/HTTPS probe. */
export function CodexProxyLatencyBadge({ result, className = '' }: { result?: LatencyState; className?: string }) {
  const { t } = useTranslation();
  const tone = proxyLatencyTone(result);
  let text = '-';
  let title = t('codex.proxy.latencyPending');
  if (result?.status === 'success') {
    text = tone === 'error' ? t('common.failed') : `${Math.round(result.value.latencyMs)} ms`;
    title = Number.isFinite(result.value.checkedAt)
      ? `${text} · ${t('codex.proxy.latencyCheckedAt')} ${new Date(result.value.checkedAt).toLocaleString()}` : text;
  } else if (result?.status === 'error') {
    text = t(String(result.error).includes('TIMEOUT') ? 'codex.proxy.latencyTimeout' : 'common.failed');
    title = t(catalogErrorKey(result.error));
  } else if (result) {
    title = t(`codex.proxy.catalog.latency_${result.status}`);
  }
  return <span className={`codex-proxy-latency is-${tone}${className ? ` ${className}` : ''}`} title={title} aria-label={title}>
    {tone === 'loading' ? <LoaderCircle size={14} className="codex-proxy-latency-spinner" aria-hidden="true" /> : text}
  </span>;
}
