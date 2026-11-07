import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { listCodexModelProviders, type CodexModelProvider } from '../../../services/codexModelProviderService';
import { useCodexPelicanStore } from '../../../stores/useCodexPelicanStore';
import { SingleSelectDropdown } from '../../SingleSelectDropdown';
import { ModalErrorMessage } from '../../ModalErrorMessage';
import { pelicanProviderModels, pelicanProviderTarget, validatePelicanProviderTargets } from './pelicanProviderModel';
import { pelicanError } from './pelicanUtils';

export function PelicanProviderSetup({ busy, invalid, mask, onValidChange, onChange }: {
  busy: boolean; invalid: boolean; mask: (value: string) => string;
  onValidChange: (valid: boolean) => void; onChange: () => void;
}) {
  const { t } = useTranslation();
  const targets = useCodexPelicanStore((state) => state.providerTargets);
  const setTargets = useCodexPelicanStore((state) => state.setProviderTargets);
  const [providers, setProviders] = useState<CodexModelProvider[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [search, setSearch] = useState('');
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    setLoading(true); setError(null);
    void Promise.race([listCodexModelProviders(), new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error('PELICAN_TIMEOUT')), 15000);
    })]).then((items) => { if (!disposed) setProviders(items); })
      .catch((cause) => { if (!disposed) setError(pelicanError(cause, t)); })
      .finally(() => { clearTimeout(timer); if (!disposed) setLoading(false); });
    return () => { disposed = true; clearTimeout(timer); };
  }, [attempt, t]);
  const filtered = useMemo(() => providers.filter((provider) => `${provider.name} ${provider.apiKeys.map((key) => key.name).join(' ')}`.toLowerCase().includes(search.toLowerCase())), [providers, search]);
  const valid = !loading && !error && validatePelicanProviderTargets(providers, targets);
  useEffect(() => onValidChange(valid), [valid, onValidChange]);
  const update = (providerId: string, patch: { apiKeyId?: string; model?: string }) => {
    setTargets(targets.map((target) => target.providerId === providerId ? { ...target, ...patch } : target)); onChange();
  };
  return <section className="pelican-provider-setup" data-field="providers" tabIndex={-1} aria-invalid={invalid}>
    <div className="pelican-section-heading"><strong>{t('pelican.providers')} ({targets.length})</strong>
      <button type="button" className="btn btn-secondary" disabled={busy || loading || !filtered.length} onClick={() => {
        const candidates = filtered.flatMap((provider) => { const target = pelicanProviderTarget(provider); return target ? [target] : []; });
        const all = candidates.every((entry) => targets.some((target) => target.providerId === entry.providerId));
        setTargets(all ? targets.filter((target) => !candidates.some((entry) => entry.providerId === target.providerId))
          : [...targets, ...candidates.filter((entry) => !targets.some((target) => target.providerId === entry.providerId))]); onChange();
      }}>{t('common.selectAll')}</button>
      <button type="button" className="btn btn-secondary" disabled={busy || !targets.length} onClick={() => { setTargets([]); onChange(); }}>{t('pelican.providerClear')}</button>
    </div>
    <p className="pelican-muted">{t('pelican.providerHelp')}</p>
    <input type="search" value={search} aria-label={t('pelican.providerSearch')} placeholder={t('pelican.providerSearch')} onChange={(event) => setSearch(event.target.value)} />
    {loading && <p role="status">{t('common.loading')}</p>}
    <ModalErrorMessage message={error ?? undefined} />
    {error && <button type="button" className="btn btn-secondary" onClick={() => setAttempt((value) => value + 1)}>{t('common.refresh')}</button>}
    <div className="pelican-provider-list">
      {filtered.map((provider) => {
        const target = targets.find((entry) => entry.providerId === provider.id);
        const initial = pelicanProviderTarget(provider);
        const models = pelicanProviderModels(provider, target?.apiKeyId ?? initial?.apiKeyId);
        return <div className="pelican-provider-row" key={provider.id}>
          <label className="pelican-provider-choice"><input type="checkbox" checked={!!target} disabled={busy || !initial} onChange={() => {
            setTargets(target ? targets.filter((entry) => entry.providerId !== provider.id) : initial ? [...targets, initial] : targets); onChange();
          }} /><span>{mask(provider.name)}</span></label>
          <div><SingleSelectDropdown value={target?.apiKeyId ?? initial?.apiKeyId ?? ''} ariaLabel={t('codex.modelProviders.selectSavedApiKey')} disabled={busy || !target}
            options={provider.apiKeys.filter((key) => key.apiKey.trim()).map((key) => ({ value: key.id, label: mask(key.name || t('codex.modelProviders.unnamedKey')) }))}
            placeholder={t('codex.modelProviders.selectSavedApiKey')} onChange={(apiKeyId) => update(provider.id, { apiKeyId })} /></div>
          <div className="pelican-provider-model"><input value={target?.model ?? initial?.model ?? ''} maxLength={128} aria-label={t('pelican.model')} disabled={busy || !target}
            onChange={(event) => update(provider.id, { model: event.target.value })} />
            {models.length > 0 && <SingleSelectDropdown value={models.includes(target?.model ?? '') ? target!.model : ''} ariaLabel={t('pelican.model')} disabled={busy || !target}
              options={models.map((model) => ({ value: model, label: model }))} placeholder={t('pelican.model')} onChange={(model) => update(provider.id, { model })} />}</div>
        </div>;
      })}
    </div>
    {!loading && !filtered.length && <p className="pelican-muted">{t('pelican.providerEmpty')}</p>}
    {invalid && <span className="pelican-field-error" role="alert">{t('pelican.providerRequired')}</span>}
  </section>;
}
