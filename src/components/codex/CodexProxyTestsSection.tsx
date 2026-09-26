import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Activity, Gauge, RefreshCw, ShieldCheck, TriangleAlert } from 'lucide-react';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';
import { useCodexProxyAccountName, useCodexProxyExitEditor } from './useCodexProxyExitEditor';
import { cancelProxyCatalog, catalogErrorKey, probeProxyCatalog } from '../../services/codexProxyCatalogService';
import type { CodexProxyProbeResult } from '../../services/codexAccountProxyService';
import { defaultProxySelections } from '../../utils/codexProxySelection';
import { formatProxyDateTime } from '../../utils/codexProxyFormat';
import '../../styles/pages/codex-proxy-resources-page.css';

interface ProbeState {
  running: boolean;
  result?: CodexProxyProbeResult;
  error?: string;
}

/**
 * Tests page: the account's own exit check plus per-node latency probes. Both numbers describe
 * the probe request only — neither is a download speed, and probing never changes a binding.
 */
export function CodexProxyTestsSection() {
  const { t } = useTranslation();
  const { accounts, selectedId, catalog, catalogLoading, catalogError, goSection } = useCodexProxyWorkspace();
  const editor = useCodexProxyExitEditor(selectedId);
  const resolveName = useCodexProxyAccountName();
  const [probes, setProbes] = useState<Record<string, ProbeState>>({});
  const requests = useRef<Record<string, string>>({});
  const alive = useRef(true);
  const account = accounts.find((entry) => entry.id === selectedId);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      for (const id of Object.values(requests.current)) void cancelProxyCatalog(id).catch(() => {});
      requests.current = {};
    };
  }, []);
  // Switching accounts drops the previous account's probe results instead of mixing them.
  useEffect(() => {
    for (const id of Object.values(requests.current)) void cancelProxyCatalog(id).catch(() => {});
    requests.current = {};
    setProbes({});
  }, [selectedId]);

  /** Only nodes and non-manual groups can be probed without an explicit member selection. */
  const targets = useMemo(() => catalog.sources.flatMap((source) => [
    ...source.nodes.filter((node) => node.supported).map((node) => ({ source, id: node.id, name: node.name, kind: t('codex.proxy.catalog.nodes') })),
    ...source.groups
      .filter((group) => group.supported && group.kind !== 'select')
      .map((group) => ({ source, id: group.id, name: group.name, kind: t(`codex.proxy.catalog.${group.kind === 'url-test' ? 'latencyGroup' : group.kind === 'fallback' ? 'fallbackGroup' : 'loadBalanceGroup'}`) })),
  ]), [catalog.sources, t]);
  const manualGroups = useMemo(() => catalog.sources.reduce(
    (total, source) => total + source.groups.filter((group) => group.supported && group.kind === 'select').length, 0,
  ), [catalog.sources]);

  const run = async (sourceId: string, itemId: string, selections: Record<string, string>) => {
    const key = `${sourceId}:${itemId}`;
    if (probes[key]?.running) return;
    const requestId = `${itemId}-${Date.now().toString(36)}`;
    requests.current[key] = requestId;
    setProbes((old) => ({ ...old, [key]: { running: true } }));
    try {
      const result = await probeProxyCatalog(requestId, sourceId, itemId, selections);
      if (alive.current) setProbes((old) => ({ ...old, [key]: { running: false, result } }));
    } catch (caught) {
      if (alive.current) setProbes((old) => ({ ...old, [key]: { running: false, error: t(catalogErrorKey(caught)) } }));
    } finally {
      delete requests.current[key];
    }
  };
  const cancel = async (key: string) => {
    const requestId = requests.current[key];
    if (!requestId) return;
    try {
      await cancelProxyCatalog(requestId);
    } catch (caught) {
      if (alive.current) setProbes((old) => ({ ...old, [key]: { running: false, error: t(catalogErrorKey(caught)) } }));
    }
  };

  return <div className="codex-proxy-tests">
    <section className="codex-proxy-tests-card" aria-labelledby="codex-proxy-tests-egress">
      <header>
        <h3 id="codex-proxy-tests-egress"><Activity size={17} aria-hidden="true" />{t('codex.proxy.activity.probe')}</h3>
        <p>{t('codex.proxy.tests.egressHint')}</p>
      </header>
      <p className="codex-proxy-tests-scope">{account ? t('codex.proxy.overview.intro', { name: resolveName(account) }) : t('codex.proxy.tests.noAccount')}</p>
      <div className="codex-proxy-tests-actions">
        <button type="button" className="btn btn-primary" disabled={!account || editor.testing} onClick={() => editor.test()}>
          {editor.testing ? <RefreshCw size={15} className="loading-spinner" /> : <ShieldCheck size={15} />}
          {t(editor.testing ? 'codex.proxy.testing' : 'codex.proxy.test')}
        </button>
        {editor.testing && <button type="button" className="btn btn-secondary" onClick={() => editor.cancelTest()}>{t('codex.proxy.cancelCheck')}</button>}
      </div>
      {editor.error && <p className="codex-proxy-page-error" role="alert">{editor.error}</p>}
      {editor.result && <output className="codex-proxy-check-result" aria-live="polite"><ShieldCheck size={18} aria-hidden="true" /><div>
        <strong>{t('codex.proxy.testPassed')}</strong>
        <span>{editor.result.protocol} · {editor.result.ip} · {editor.result.latencyMs} ms</span>
        <time dateTime={new Date(editor.result.checkedAt).toISOString()}>{formatProxyDateTime(editor.result.checkedAt)}</time>
      </div></output>}
      <p className="codex-proxy-page-note">{t('codex.proxy.testNotice')}</p>
    </section>

    <section className="codex-proxy-tests-card" aria-labelledby="codex-proxy-tests-nodes">
      <header>
        <h3 id="codex-proxy-tests-nodes"><Gauge size={17} aria-hidden="true" />{t('codex.proxy.tests.nodesTitle')}</h3>
        <p>{t('codex.proxy.tests.nodesHint')}</p>
      </header>
      {catalogError && <div className="codex-proxy-page-error" role="alert">{catalogError}
        <button type="button" className="btn btn-secondary compact" onClick={() => goSection('resources')}>{t('codex.proxy.manager.proxies')}</button></div>}
      {catalogLoading && <p className="codex-proxy-page-note" role="status">{t('common.loading')}</p>}
      {!catalogLoading && targets.length === 0 && <div className="codex-proxy-tests-empty">
        <p>{t('codex.proxy.tests.empty')}</p>
        <p className="codex-proxy-page-note">{t('codex.proxy.tests.emptyHint')}</p>
        <button type="button" className="btn btn-secondary compact" onClick={() => goSection('resources')}>{t('codex.proxy.manager.proxies')}</button>
      </div>}
      {targets.length > 0 && <ul className="codex-proxy-tests-list">
        {targets.map(({ source, id, name, kind }) => {
          const key = `${source.id}:${id}`;
          const state = probes[key];
          const selections = defaultProxySelections(source, id, {});
          const ready = selections !== null;
          return <li key={key} className={state?.error ? 'is-error' : ''}>
            <span className="codex-proxy-tests-name"><strong>{name}</strong><small>{source.name} · {kind}</small></span>
            <span className="codex-proxy-tests-result">
              {state?.running && <><RefreshCw size={14} className="loading-spinner" />{t('codex.proxy.catalog.latency_running')}</>}
              {!state?.running && state?.result && <>{state.result.latencyMs} ms · {state.result.ip}</>}
              {!state?.running && state?.error && <><TriangleAlert size={14} aria-hidden="true" />{state.error}</>}
              {!state && <span className="codex-proxy-page-note">{t('codex.proxy.notChecked')}</span>}
            </span>
            <span className="codex-proxy-tests-buttons">
              {state?.running
                ? <button type="button" className="btn btn-secondary compact" onClick={() => void cancel(key)}>{t('common.cancel')}</button>
                : <button type="button" className="btn btn-secondary compact" disabled={!ready}
                  onClick={() => void run(source.id, id, selections ?? {})}>{t('codex.proxy.catalog.measure')}</button>}
            </span>
          </li>;
        })}
      </ul>}
      {manualGroups > 0 && <p className="codex-proxy-page-note">{t('codex.proxy.tests.selectGroupHint', { count: manualGroups })}</p>}
      <p className="codex-proxy-page-note">{t('codex.proxy.catalog.latencyNotice')}</p>
      <p className="codex-proxy-page-note">{t('codex.proxy.activity.probeHint')}</p>
    </section>
  </div>;
}
