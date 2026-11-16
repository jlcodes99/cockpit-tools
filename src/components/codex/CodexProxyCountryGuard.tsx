import { useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import { getCodexProxyCountryPolicy, setCodexProxyCountryPolicy, getCodexCountryChecks,
  checkCodexCountry, type CodexCountryCheck } from '../../services/codexProxyCountryService';
import { codexCountryErrorKey, codexCountryErrorValues, codexCountryOptions } from '../../utils/codexProxyCountry';
import { CodexProxySelect } from './CodexProxySelect';
import { ModalErrorMessage } from '../ModalErrorMessage';

/** Drafts never change the launch policy until the explicit Save action. */
export function CodexProxyCountryGuard({ accountId, disabled = false, routeDirty = false, revision = 0 }: {
  accountId: string; disabled?: boolean; routeDirty?: boolean; revision?: number;
}) {
  const { t, i18n } = useTranslation();
  const [ready, setReady] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  const [enabled, setEnabled] = useState(false);
  const [country, setCountry] = useState('');
  const [busy, setBusy] = useState(false);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState(false);
  const [reload, setReload] = useState(0);
  const [checks, setChecks] = useState<CodexCountryCheck[]>([]);
  const current = useRef(accountId); current.current = accountId;
  const language = i18n.resolvedLanguage || i18n.language || 'en';
  const options = useMemo(() => codexCountryOptions(language), [language]);
  useEffect(() => {
    let alive = true;
    setReady(false); setBusy(false); setError(''); setNotice(false); setChecking(false);
    void getCodexProxyCountryPolicy(accountId).then(value => {
      if (!alive) return;
      setSaved(value.expectedCountry); setEnabled(Boolean(value.expectedCountry)); setCountry(value.expectedCountry ?? ''); setReady(true);
    }).catch(() => { if (alive) setError(t('codex.proxy.country.storage')); });
    return () => { alive = false; };
  }, [accountId, reload, t]);
  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    setChecks([]);
    void getCodexCountryChecks(accountId).then(value => { if (alive) setChecks(previous => [...value.filter(item => !previous.some(existing => existing.checkId === item.checkId)), ...previous].slice(-12)); })
      .catch(() => { if (alive) setError(t('codex.proxy.country.historyFailed')); });
    void listen<CodexCountryCheck>('codex:country-check', event => {
      const value = event.payload;
      if (!alive || value.accountId !== accountId) return;
      setChecks(previous => [...previous.filter(item => item.checkId !== value.checkId), value].slice(-12));
    }).then(cleanup => { if (alive) unlisten = cleanup; else cleanup(); });
    return () => { alive = false; unlisten?.(); };
  }, [accountId, revision, reload, t]);
  const expected = enabled ? country : null;
  const dirty = expected !== saved || routeDirty;
  const save = async () => {
    if (disabled || !ready || busy || expected === saved || (enabled && !country)) return;
    const id = accountId; setBusy(true); setError(''); setNotice(false);
    try {
      const value = await setCodexProxyCountryPolicy(id, expected);
      if (current.current === id) { setSaved(value.expectedCountry); setNotice(true); setChecks([]); }
    } catch (caught) {
      if (current.current === id) setError(t(codexCountryErrorKey(caught) ?? 'codex.proxy.country.storage'));
    } finally { if (current.current === id) setBusy(false); }
  };
  const check = async () => {
    if (disabled || !ready || checking || busy || dirty) return;
    const id = accountId; setChecking(true); setError('');
    try {
      const value = await checkCodexCountry(id);
      if (current.current === id) setChecks(previous => [...previous.filter(item => item.checkId !== value.checkId), value].slice(-12));
    } catch (caught) {
      if (current.current === id) {
        setError(t(codexCountryErrorKey(caught) ?? 'codex.proxy.country.unavailable', codexCountryErrorValues(caught)));
        try { const items = await getCodexCountryChecks(id); if (current.current === id) setChecks(items); } catch { /* Keep the check failure visible. */ }
      }
    } finally { if (current.current === id) setChecking(false); }
  };
  const latest = checks[checks.length - 1];
  const countryLabel = (code: string | null) => options.find(option => option.value === code)?.label ?? t('codex.proxy.country.notDetected');
  const lines = (value: CodexCountryCheck) => <dl className="codex-proxy-country-result">
    <dt>{t('codex.proxy.country.expected')}</dt><dd>{value.expectedCountry ? countryLabel(value.expectedCountry) : t('codex.proxy.country.notRequired')}</dd>
    <dt>{t('codex.proxy.country.actual')}</dt><dd>{countryLabel(value.actualCountry)}</dd>
    <dt>{t('codex.proxy.country.route')}</dt><dd>{t('codex.proxy.country.route_' + value.source)}</dd>
    <dt>{t('codex.proxy.country.checkedAt')}</dt><dd>{new Date(value.checkedAt).toLocaleString(language)} · {value.elapsedMs} ms</dd>
    {value.reason && <><dt>{t('codex.switchAuth.reasonLabel')}</dt><dd>{t(codexCountryErrorKey(value.reason) ?? 'codex.proxy.country.unavailable', codexCountryErrorValues(value.reason))}</dd></>}
  </dl>;
  return <section className="codex-proxy-country-guard" aria-label={t('codex.proxy.country.title')}>
    <div className="codex-resource-network-row">
      <strong>{t('codex.proxy.country.title')}</strong>
      <button type="button" className="btn codex-resource-network-switch" role="switch"
        aria-label={t('codex.proxy.country.title')} aria-checked={enabled} disabled={disabled || !ready || busy || checking}
        onClick={() => { setEnabled(value => !value); setNotice(false); }}><span /></button>
    </div>
    <p className="codex-proxy-page-note">{t('codex.proxy.country.hint')}</p>
    {enabled && <CodexProxySelect value={country} options={options} disabled={disabled || !ready || busy || checking}
      label={t('codex.proxy.country.expected')} placeholder={t('codex.proxy.country.choose')} searchPlaceholder={t('codex.proxy.country.search')}
      onChange={value => { setCountry(value); setNotice(false); }} />}
    <ModalErrorMessage message={error} />
    {!ready && error && <button type="button" className="btn btn-secondary compact" onClick={() => setReload(v => v + 1)}>{t('common.retry')}</button>}
    <div className="codex-proxy-account-check-actions"><button type="button" className="btn btn-secondary compact"
      disabled={disabled || !ready || busy || checking || expected === saved || (enabled && !country)} onClick={() => { void save(); }}>
      {t(busy ? 'common.saving' : 'codex.proxy.country.save')}</button>
      {notice && <span className="codex-proxy-page-note" role="status">{t('codex.proxy.country.saved')}</span>}
    </div>
    <details className="codex-proxy-country-evidence">
      <summary>{t('codex.proxy.country.resultTitle')}
        <span className="codex-proxy-country-status" data-status={latest && !dirty && !latest.stale ? latest.status : 'stale'}>
          {t(latest ? 'codex.proxy.country.result_' + (dirty || latest.stale ? 'stale' : latest.status) : 'codex.proxy.country.noChecks')}
        </span>
      </summary>
      <div className="codex-proxy-country-evidence-body">
      <p className="codex-proxy-page-note">{t('codex.proxy.country.evidenceHint')}</p>
      {dirty && <p className="codex-proxy-page-note">{t('codex.proxy.country.saveFirst')}</p>}
      {latest ? <div role="status" data-status={dirty || latest.stale ? 'stale' : latest.status}>
        <strong>{t('codex.proxy.country.result_' + (dirty || latest.stale ? 'stale' : latest.status))}</strong>{lines(latest)}
      </div> : <p className="codex-proxy-page-note">{t('codex.proxy.country.noChecks')}</p>}
      <button type="button" className="btn btn-secondary compact" disabled={disabled || !ready || busy || checking || dirty}
        onClick={() => { void check(); }}>{t(checking ? 'codex.proxy.country.result_checking' : 'codex.proxy.country.checkNow')}</button>
      {checks.length > 1 && <details><summary>{t('codex.proxy.country.history')}</summary>
        {[...checks].reverse().slice(1).map(value => <div className="codex-proxy-country-history-item" key={value.checkId}>
          <strong>{t('codex.proxy.country.result_' + (value.stale ? 'stale' : value.status))}</strong>{lines(value)}</div>)}</details>}
    </div></details>
  </section>;
}
