import { useEffect, useId, useRef, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { catalogErrorKey, type ProxyCatalogNetwork, type ProxyCatalogSource } from '../../services/codexProxyCatalogService';

export function CodexProxyResourceNetwork({ source, disabled, saving, onSave }: {
  source: ProxyCatalogSource;
  disabled: boolean;
  saving: boolean;
  onSave: (network: ProxyCatalogNetwork) => Promise<void>;
}) {
  const { t } = useTranslation();
  const id = useId();
  const saved = source.network ?? { doh: false };
  const [draft, setDraft] = useState<{ doh: boolean; ipv6?: boolean; servers: string } | null>(null);
  const [error, setError] = useState('');
  const serversField = useRef<HTMLTextAreaElement>(null);
  const errorElement = useRef<HTMLParagraphElement>(null);
  const value = draft ?? { doh: saved.doh, ipv6: saved.ipv6, servers: (saved.dnsServers ?? []).join('\n') };
  const servers = value.doh ? value.servers.split('\n').map((line) => line.trim()).filter(Boolean) : saved.dnsServers ?? [];
  const ipv6 = value.ipv6 ?? source.ipv6Default ?? true;
  const savedIpv6 = saved.ipv6 ?? source.ipv6Default ?? true;
  const dirty = value.doh !== saved.doh || value.ipv6 !== saved.ipv6
    || servers.join('\n') !== (saved.dnsServers ?? []).join('\n');
  const dnsError = error === 'codex.proxy.catalog.invalidDnsServer';

  useEffect(() => {
    if (!error) return;
    const target = dnsError ? serversField.current : errorElement.current;
    target?.scrollIntoView({ block: 'nearest' });
    target?.focus({ preventScroll: true });
  }, [error, dnsError]);

  return <form className="codex-resource-network" onSubmit={async (event) => {
    event.preventDefault();
    if (disabled || (!dirty && !error)) return;
    setError('');
    try {
      await onSave({ doh: value.doh, ipv6: value.ipv6, dnsServers: servers });
      setDraft(null);
    } catch (cause) {
      const key = catalogErrorKey(cause);
      setError(key === 'codex.proxy.catalog.failed' ? 'codex.proxy.catalog.networkSaveFailed' : key);
    }
  }}>
    <div className="codex-resource-network-row">
      <label htmlFor={`${id}-dns`}>{t('codex.proxy.catalog.dnsOverride')}</label>
      <button id={`${id}-dns`} type="button" className="btn codex-resource-network-switch" role="switch"
        aria-checked={value.doh} disabled={disabled}
        onClick={() => { setError(''); setDraft({ ...value, doh: !value.doh }); }}><span /></button>
    </div>
    {value.doh && <div className="codex-resource-network-servers">
      <label htmlFor={`${id}-servers`}>{t('codex.proxy.catalog.dnsServers')}</label>
      <textarea ref={serversField} id={`${id}-servers`} rows={2} value={value.servers} disabled={disabled}
        placeholder="https://1.1.1.1/dns-query" spellCheck={false} autoCapitalize="none"
        aria-invalid={dnsError} aria-describedby={`${id}-dns-hint${dnsError ? ` ${id}-network-error` : ''}`}
        onChange={(event) => { setError(''); setDraft({ ...value, servers: event.target.value }); }} />
      <p id={`${id}-dns-hint`} className="codex-proxy-page-note">{t('codex.proxy.catalog.dnsServersHint')}</p>
      {dnsError && <p id={`${id}-network-error`} className="codex-proxy-page-error" role="alert">{t(error)}</p>}
    </div>}
    <div className="codex-resource-network-row">
      <label htmlFor={`${id}-ipv6`}>{t('codex.proxy.catalog.ipv6')}</label>
      <button id={`${id}-ipv6`} type="button" className="btn codex-resource-network-switch" role="switch"
        aria-checked={ipv6} disabled={disabled}
        onClick={() => {
          const enabled = !ipv6;
          setError('');
          setDraft({ ...value, ipv6: enabled === savedIpv6 ? saved.ipv6 : enabled });
        }}><span /></button>
    </div>
    <div className="codex-resource-network-footer">
      <p className="codex-proxy-page-note">{t('codex.proxy.catalog.networkScope')}</p>
      <button type="submit" className="btn btn-secondary compact" aria-busy={saving} disabled={disabled || (!dirty && !error)}>
        {saving && <RefreshCw size={14} className="loading-spinner" />}
        {saving ? t('common.processing') : t(error && !dnsError ? 'common.retry' : 'common.save')}
      </button>
    </div>
    {error && !dnsError && <p ref={errorElement} className="codex-proxy-page-error" role="alert" tabIndex={-1}>{t(error)}</p>}
  </form>;
}
