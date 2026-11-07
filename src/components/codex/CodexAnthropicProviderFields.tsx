import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { listModelProviderModels } from '../../services/modelProviderUsageService';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { readText } from '@tauri-apps/plugin-clipboard-manager';
import { parseCodexAnthropicImport, type CodexAnthropicImport } from '../../utils/codexAnthropicImport';
import { Info } from 'lucide-react';
import './CodexAnthropicProviderFields.css';

export function CodexAnthropicProviderFields({ baseUrl, apiKey, authScheme, disabled, onAuthScheme, onModels, onImport }: {
  baseUrl: string;
  apiKey: string;
  authScheme: 'x-api-key' | 'bearer';
  disabled: boolean;
  onAuthScheme: (value: 'x-api-key' | 'bearer') => void;
  onModels: (models: string[]) => void;
  onImport: (config: CodexAnthropicImport) => void;
}) {
  const { t } = useTranslation();
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [importFailed, setImportFailed] = useState(false);
  const [importing, setImporting] = useState(false);
  const errorRef = useRef<HTMLParagraphElement>(null);
  const generation = useRef(0);
  const modelCallback = useRef(onModels);
  modelCallback.current = onModels;
  useEffect(() => {
    generation.current += 1;
    setFailed(false);
    setImportFailed(false);
    setLoading(false);
    return () => { generation.current += 1; };
  }, [baseUrl, apiKey, authScheme]);
  useEffect(() => {
    if (failed || importFailed) errorRef.current?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }, [failed, importFailed]);
  async function importConfig() {
    setFailed(false); setImportFailed(false); setImporting(true);
    const current = ++generation.current;
    try {
      const config = parseCodexAnthropicImport(await readText() ?? '');
      if (current !== generation.current) return;
      if (!config) { setImportFailed(true); return; }
      onImport(config);
    } catch { if (current === generation.current) setImportFailed(true); }
    finally { setImporting(false); }
  }
  async function fetchModels() {
    setFailed(false);
    setImportFailed(false);
    setLoading(true);
    const current = ++generation.current;
    try {
      const result = await listModelProviderModels({ baseUrl, apiKey, wireApi: 'anthropic', authScheme });
      if (current !== generation.current) return;
      if (!result.models.length) { setFailed(true); return; }
      modelCallback.current(result.models.map(model => model.id));
    } catch {
      if (current === generation.current) setFailed(true);
    } finally {
      if (current === generation.current) setLoading(false);
    }
  }
  return <div className="form-group codex-anthropic-fields">
    <label>{t('codex.modelProviders.anthropic.auth')}</label>
    <SingleSelectDropdown value={authScheme} disabled={disabled || loading || importing}
      options={[{ value: 'x-api-key', label: 'x-api-key' }, { value: 'bearer', label: 'Bearer' }]}
      onChange={value => { setFailed(false); onAuthScheme(value as 'x-api-key' | 'bearer'); }} />
    <p className="codex-anthropic-hint"><Info size={15} aria-hidden="true" /><span>{t('codex.modelProviders.anthropic.hint')}</span></p>
    <div className="codex-anthropic-actions">
    <button type="button" className="btn btn-secondary" disabled={disabled || loading || importing || !baseUrl.trim() || !apiKey.trim()}
      onClick={() => { void fetchModels(); }}>
      {t(loading ? 'codex.modelProviders.anthropic.fetching' : 'codex.modelProviders.anthropic.fetch')}
    </button>
    <button type="button" className="btn btn-secondary" disabled={disabled || loading || importing}
      onClick={() => { void importConfig(); }}>{t('codex.modelProviders.anthropic.import')}</button>
    </div>
    {(failed || importFailed) && <p ref={errorRef} className="form-error" role="alert">{t(importFailed ? 'codex.modelProviders.anthropic.importFailed' : 'codex.modelProviders.anthropic.fetchFailed')}</p>}
  </div>;
}
