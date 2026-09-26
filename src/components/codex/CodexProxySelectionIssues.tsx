import { useEffect, useRef, useState } from 'react';
import { RefreshCw, ShieldAlert } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { cancelProxyCatalog, catalogErrorKey, catalogUnsupportedKey, getProxyCatalog, refreshProxyCatalog, setProxyGroupInsecure, setProxyNodeInsecure,
  type ProxyCatalog, type ProxyCatalogGroup, type ProxyCatalogNode, type ProxyCatalogSource } from '../../services/codexProxyCatalogService';

/** Configuration permission is explicit and scoped to the displayed choice, never an account rebind. */
export function CodexProxySelectionIssues({ source, target, busy, onCatalogChange, onPendingChange }: {
  source: ProxyCatalogSource;
  target?: ProxyCatalogGroup | ProxyCatalogNode;
  busy: boolean;
  onCatalogChange?: (catalog: ProxyCatalog) => void;
  onPendingChange: (pending: boolean) => void;
}) {
  const { t } = useTranslation();
  const [pending, setPending] = useState('');
  const [error, setError] = useState('');
  const lifetime = useRef(0);
  const request = useRef<string | null>(null);
  const working = useRef(false);
  const notify = useRef(onPendingChange); notify.current = onPendingChange;
  useEffect(() => {
    lifetime.current += 1; setError(''); setPending(''); working.current = false;
    notify.current(false);
    return () => {
      lifetime.current += 1;
      if (request.current) void cancelProxyCatalog(request.current).catch(() => {});
      request.current = null;
      notify.current(false);
    };
  }, [source.id]);
  useEffect(() => { setError(''); }, [target?.id, source.revision]);
  const group = target && 'kind' in target ? target : undefined;
  const node = target && 'protocol' in target ? target : undefined;
  const permissionNodes = group
    ? source.nodes.filter((node) => group.insecureNodeIds?.includes(node.id))
    : node?.insecure && node.error === 'PROXY_TLS_INSECURE' ? [node] : [];
  const issues = group ? group.issues ?? [] : node?.error ? [{ name: node.name, error: node.error }] : [];
  const run = async (kind: 'refresh' | 'allow') => {
    if (working.current || busy || !onCatalogChange) return;
    working.current = true; setPending(kind); setError(''); notify.current(true);
    const current = lifetime.current;
    try {
      let catalog: ProxyCatalog;
      if (kind === 'refresh') {
        const id = crypto.randomUUID(); request.current = id;
        catalog = await refreshProxyCatalog(id, source.id);
      } else if (group) {
        catalog = await setProxyGroupInsecure(source.id, group.id, source.revision, true);
      } else if (target) {
        catalog = await setProxyNodeInsecure(source.id, target.id, source.revision, true);
      } else return;
      if (lifetime.current === current) onCatalogChange(catalog);
    } catch (caught) {
      if (lifetime.current === current) setError(t(catalogErrorKey(caught)));
      // A background subscription update can invalidate the displayed revision. Re-read
      // metadata once so a deliberate retry applies to the newly displayed node options.
      if (lifetime.current === current && String(caught).replace(/^Error:\s*/, '') === 'CATALOG_CHANGED') {
        try {
          const catalog = await getProxyCatalog();
          if (lifetime.current === current) onCatalogChange(catalog);
        } catch (readError) {
          if (lifetime.current === current) setError(t(catalogErrorKey(readError)));
        }
      }
    } finally {
      if (lifetime.current === current) {
        request.current = null; working.current = false; setPending(''); notify.current(false);
      }
    }
  };
  return <>
    <ModalErrorMessage message={error} />
    {source.needsRefresh && <div className="codex-picker-issue-card" role="status">
      <RefreshCw size={17} /><div><strong>{t('codex.proxy.catalog.refreshRequired')}</strong>
        <p>{t('codex.proxy.catalog.refreshRequiredHint')}</p></div>
      {onCatalogChange && <button type="button" className="btn btn-secondary compact" disabled={busy || Boolean(pending)} onClick={() => void run('refresh')}>
        {pending === 'refresh' && <RefreshCw size={14} className="loading-spinner" />}{t('common.refresh')}</button>}
      {pending === 'refresh' && <button type="button" className="btn btn-secondary compact" onClick={() => {
        if (request.current) void cancelProxyCatalog(request.current).catch((caught) => setError(t(catalogErrorKey(caught))));
      }}>{t('common.cancel')}</button>}
    </div>}
    {permissionNodes.length > 0 && <div className="codex-picker-issue-card is-permission">
      <ShieldAlert size={17} /><div><strong>{t('codex.proxy.catalog.permissionRequired', { count: permissionNodes.length })}</strong>
        <p>{t('codex.proxy.catalog.permissionHint')}</p>
        <ul>{permissionNodes.map((node) => <li key={node.id}>{node.name}</li>)}</ul></div>
      {onCatalogChange && <button type="button" className="btn btn-secondary compact" disabled={busy || Boolean(pending)} onClick={() => void run('allow')}>
        {pending === 'allow' && <RefreshCw size={14} className="loading-spinner" />}{t('codex.proxy.catalog.allowSelectedOptions')}</button>}
    </div>}
    {issues.some((issue) => issue.error !== 'PROXY_TLS_INSECURE') && <details className="codex-picker-issue-details" open>
      <summary>{t('codex.proxy.catalog.memberIssues')}</summary>
      <ul>{issues.filter((issue) => issue.error !== 'PROXY_TLS_INSECURE').map((issue, index) => <li key={`${issue.name}:${index}`}>
        <strong>{issue.name}</strong><span>{t(catalogUnsupportedKey({ error: issue.error }))}</span>
      </li>)}</ul>
    </details>}
  </>;
}
