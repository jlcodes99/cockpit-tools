import { useMemo, useState } from 'react';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { Check, Gauge, Search, ShieldAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { catalogErrorKey, catalogUnsupportedKey, type ProxyCatalogSource } from '../../services/codexProxyCatalogService';
import type { useProxyLatency } from './useProxyLatency';
import { CodexProxyLatencyBadge } from './CodexProxyLatencyBadge';

/** Nodes are visible where the subscription was expanded; credentials never enter this view. */
export function CodexProxyResourceNodes({ source, itemId, busy, latency, onChoose, onInsecure }: {
  source: ProxyCatalogSource;
  itemId: string;
  busy: boolean;
  latency: ReturnType<typeof useProxyLatency>;
  onChoose: (id: string) => void;
  onInsecure: (id: string, enabled: boolean) => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  const visible = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return source.nodes.filter((node) => !needle || `${node.name} ${node.protocol}`.toLocaleLowerCase().includes(needle));
  }, [query, source.nodes]);
  const testable = visible.filter((node) => node.supported).map((node) => node.id);
  const selected = source.nodes.find((node) => node.id === itemId);
  const locked = busy || latency.running;
  return <div className="codex-resource-node-browser">
    <ModalErrorMessage message={latency.errorKey ? t(latency.errorKey) : null} />
    <div className="codex-resource-node-toolbar">
      <label className="codex-proxy-search"><Search size={15} aria-hidden="true" />
        <input value={query} aria-label={t('codex.proxy.catalog.search')} placeholder={t('codex.proxy.catalog.search')} onChange={(event) => setQuery(event.target.value)} />
      </label>
      <button type="button" className="btn btn-secondary compact" disabled={locked || !testable.length} onClick={() => latency.measure(testable)}><Gauge size={15} />{t('codex.proxy.catalog.check')} · {testable.length}</button>
    </div>
    <div className="codex-resource-visible-nodes" role="group" aria-label={t('codex.proxy.catalog.nodes')}>
      {visible.map((node) => {
        const measure = latency.results[node.id];
        const active = itemId === node.id;
        const detail = measure?.status === 'error' ? t(catalogErrorKey(measure.error)) : !node.supported ? t(catalogUnsupportedKey(node)) : node.protocol.toUpperCase();
        return <div key={node.id} className={'codex-resource-visible-node' + (active ? ' is-selected' : '')}>
          <button type="button" className="codex-resource-node-choice" aria-pressed={active} disabled={busy || (!node.supported && !node.insecure)}
            onClick={() => { latency.cancel(); onChoose(node.id); latency.measure([node.id], true); }}>
            <span className="codex-resource-choice-mark">{active && <Check size={12} />}</span>
            <span><strong>{node.name}</strong><small>{detail}</small></span>
          </button>
          <div className="codex-resource-node-measure">
            <CodexProxyLatencyBadge result={measure} />
            <button type="button" className="btn btn-secondary compact" aria-label={`${node.name} · ${t('codex.proxy.catalog.check')}`} title={t('codex.proxy.catalog.check')}
              disabled={locked || !node.supported} onClick={() => latency.measure([node.id])}><Gauge size={15} /></button>
          </div>
        </div>;
      })}
      {!visible.length && <p className="codex-proxy-page-note">{t('codex.proxy.catalog.noResults')}</p>}
    </div>
    {selected?.insecure && <div className="codex-resource-tls"><ShieldAlert size={17} />
      <span>{t('codex.proxy.catalog.insecureHint')}</span>
      <button type="button" role="switch" aria-checked={selected.supported} className={'btn btn-secondary compact' + (selected.supported ? ' active' : '')} disabled={locked}
        onClick={() => onInsecure(selected.id, !selected.supported)}>{t('codex.proxy.catalog.allowInsecure')}</button>
    </div>}
    {latency.running && <div className="codex-resource-progress">
      <button type="button" className="btn btn-secondary compact" onClick={latency.cancel}>{t('codex.proxy.cancelCheck')}</button></div>}
  </div>;
}
