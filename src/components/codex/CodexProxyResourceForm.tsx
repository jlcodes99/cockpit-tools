import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useTranslation } from 'react-i18next';
import { Download, Eye, EyeOff, ClipboardPaste, RefreshCw, Server, X } from 'lucide-react';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { detectProxyInputKind, emptyProxyForm, proxyFormUrl } from '../../utils/codexProxyPresentation';
import { catalogErrorKey, catalogUnsupportedKey, previewProxyImport, type ProxyImportOptions, type ProxyImportPreview } from '../../services/codexProxyCatalogService';
export interface ProxySourceDraft { name: string; kind: 'subscription' | 'manual'; input: string; options: ProxyImportOptions }
interface Props { busy: boolean; error: string; onSubmit: (draft: ProxySourceDraft) => void; onClose: () => void; onCancel?: () => void; onEdit: () => void }
const protocols = ['http', 'https', 'socks5', 'socks5h'];
function rememberedProtocol() {
  try { const v = localStorage.getItem('codex.proxyImportProtocol'); return v && protocols.includes(v) ? v : 'socks5'; } catch { return 'socks5'; }
}
export function CodexProxyResourceForm({ busy, error, onSubmit, onClose, onCancel, onEdit }: Props) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<'paste' | 'manual'>('paste');
  const [kind, setKind] = useState('auto');
  const [name, setName] = useState('');
  const [input, setInput] = useState('');
  const [form, setForm] = useState(emptyProxyForm);
  const [protocol, setProtocol] = useState(rememberedProtocol);
  const [format, setFormat] = useState('auto');
  const [skipInvalid, setSkipInvalid] = useState(false);
  const [skipDuplicates, setSkipDuplicates] = useState(true);
  const [revealed, setRevealed] = useState(false);
  const [invalid, setInvalid] = useState(false);
  const [preview, setPreview] = useState<ProxyImportPreview | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [previewError, setPreviewError] = useState('');
  const root = useRef<HTMLDivElement>(null);
  useModalFocusTrap(root, true);
  useModalScrollLock(true);
  useEscCloseTopmost(true, () => { if (!busy) onClose(); });
  const textarea = useRef<HTMLTextAreaElement>(null);
  const value = mode === 'manual' ? proxyFormUrl(form) ?? '' : input;
  const resolvedKind = mode === 'manual' ? 'manual' : kind === 'auto' ? detectProxyInputKind(value) : kind as 'manual' | 'subscription';
  const fallbackName = t('codex.proxy.catalog.listName');
  const options = { protocol, format, skipInvalid, skipDuplicates, fallbackName };
  useEffect(() => {
    let stale = false; setPreview(null); setPreviewError('');
    if (!value.trim() || resolvedKind !== 'manual') { setPreviewing(false); return; }
    setPreviewing(true);
    const timer = setTimeout(() => {
      void previewProxyImport(value, { protocol, format, skipInvalid, skipDuplicates, fallbackName })
        .then((result) => { if (!stale) setPreview(result); })
        .catch((caught) => { if (!stale) setPreviewError(catalogErrorKey(caught)); })
        .finally(() => { if (!stale) setPreviewing(false); });
    }, 300);
    return () => { stale = true; clearTimeout(timer); };
  }, [value, resolvedKind, protocol, format, skipInvalid, skipDuplicates, fallbackName]);
  useEffect(() => {
    if (!invalid && !error && !previewError) return;
    const target = root.current?.querySelector<HTMLElement>('[aria-invalid="true"], [role="alert"]');
    target?.scrollIntoView({ block: 'nearest' }); target?.focus({ preventScroll: true });
  }, [invalid, error, previewError]);
  const edit = () => { setInvalid(false); onEdit(); };
  const locateLine = (line: number) => {
    const field = textarea.current; if (!field) return;
    const start = input.split('\n').slice(0, line - 1).reduce((n, text) => n + text.length + 1, 0);
    field.focus(); field.setSelectionRange(start, input.indexOf('\n', start) < 0 ? input.length : input.indexOf('\n', start));
  };
  const blocked = resolvedKind === 'manual' && (!preview || previewing || !!previewError || (preview.invalid > 0 && !skipInvalid) || preview.valid + (skipDuplicates ? 0 : preview.duplicates) === 0);
  const submit = () => { if (!value.trim() || blocked) { setInvalid(true); return; } onSubmit({ name: name.trim(), kind: resolvedKind, input: value, options }); };
  return createPortal(<div className="modal-overlay codex-resource-add-overlay">
    <div ref={root} className="modal codex-resource-add-dialog" role="dialog" aria-modal="true" aria-labelledby="codex-resource-add-title" tabIndex={-1}>
    <header className="modal-header"><h2 id="codex-resource-add-title">{t('codex.proxy.managerResources.add')}</h2>
      <button type="button" className="btn btn-secondary compact" disabled={busy} aria-label={t('common.close')} onClick={onClose}><X size={16} /></button></header>
    <div className="modal-body codex-resource-add">
    <label className="codex-proxy-field"><span>{t('codex.proxy.catalog.optionalName')}</span><div className="codex-proxy-secret"><input value={name} disabled={busy} maxLength={80} autoComplete="off" placeholder={t('codex.proxy.catalog.autoName')} onChange={(event) => { setName(event.target.value); edit(); }} /></div></label>
    <div className="codex-proxy-mode" role="group" aria-label={t('codex.proxy.inputMode')}>
      {(['paste', 'manual'] as const).map((option) => <button key={option} type="button" className={'btn btn-secondary compact' + (mode === option ? ' active' : '')}
        aria-pressed={mode === option} disabled={busy} onClick={() => { setMode(option); setInput(''); setForm(emptyProxyForm()); setRevealed(false); edit(); }}>
        {option === 'paste' ? <ClipboardPaste size={14} /> : <Server size={14} />}{t('codex.proxy.catalog.input_' + option)}</button>)}
    </div>
    {mode === 'paste' ? <>
      <label className="codex-proxy-field"><span>{t('codex.proxy.catalog.pasteHint')}</span>
        <textarea ref={textarea} className={'codex-resource-links' + (revealed ? '' : ' concealed')} value={input} disabled={busy} rows={4}
          autoComplete="off" spellCheck={false} maxLength={2 * 1024 * 1024} aria-invalid={invalid && !value}
          onChange={(event) => { setInput(event.target.value); edit(); }} />{invalid && !value && <span className="codex-strategy-field-error" role="alert">{t('codex.proxy.catalog.invalidInput')}</span>}</label>
      <div className="codex-resource-import-controls">
        <button type="button" className="btn btn-secondary compact" disabled={busy} onClick={() => setRevealed(!revealed)}>{revealed ? <EyeOff size={15} /> : <Eye size={15} />}{t(revealed ? 'codex.proxy.hideCredentials' : 'codex.proxy.showCredentials')}</button>
        <SingleSelectDropdown value={kind} disabled={busy} ariaLabel={t('codex.proxy.catalog.inputType')}
          options={[{ value: 'auto', label: t('codex.proxy.catalog.autoDetect') }, { value: 'subscription', label: t('codex.proxy.catalog.input_subscription') }, { value: 'manual', label: t('codex.proxy.catalog.input_text') }]}
          onChange={(next) => { setKind(next); edit(); }} /></div>
      {!!value && <p className="codex-proxy-page-note">{t(resolvedKind === 'subscription' ? 'codex.proxy.catalog.detectedSubscription' : 'codex.proxy.catalog.detectedNodes')}</p>}
      {resolvedKind === 'manual' && <div className="codex-resource-import-controls">
        <label className="codex-proxy-field"><span>{t('codex.proxy.catalog.defaultProtocol')}</span><SingleSelectDropdown value={protocol} disabled={busy} options={protocols.map((p) => ({ value: p, label: p.toUpperCase() }))} onChange={(next) => {
          setProtocol(next); edit(); try { localStorage.setItem('codex.proxyImportProtocol', next); } catch { /* preference only */ }
        }} /></label>
        <label className="codex-proxy-field"><span>{t('codex.proxy.catalog.credentialFormat')}</span><SingleSelectDropdown value={format} disabled={busy}
          options={[{ value: 'auto', label: t('codex.proxy.catalog.autoDetect') }, { value: 'host_auth', label: 'host:port:username:password' }, { value: 'auth_at_host', label: 'username:password@host:port' }, { value: 'host_at_auth', label: 'host:port@username:password' }]}
          onChange={(next) => { setFormat(next); edit(); }} /></label></div>}
    </> : <div className="codex-proxy-form">
      <label className="codex-proxy-field"><span>{t('codex.proxy.protocol')}</span><SingleSelectDropdown value={form.protocol} disabled={busy} options={protocols.map((p) => ({ value: p, label: p.toUpperCase() }))} onChange={(next) => { setForm({ ...form, protocol: next }); edit(); }} /></label>
      {(['host', 'port', 'username', 'password'] as const).map((field) => <label className={'codex-proxy-field field-' + field} key={field}><span>{t('codex.proxy.' + field)}</span><div className="codex-proxy-secret">
        <input value={form[field]} type={['username', 'password'].includes(field) && !revealed ? 'password' : 'text'} autoComplete="off" spellCheck={false} disabled={busy} maxLength={field === 'port' ? 5 : 2048} inputMode={field === 'port' ? 'numeric' : undefined}
          aria-invalid={invalid && !value && ['host', 'port'].includes(field)} onChange={(event) => { setForm({ ...form, [field]: event.target.value }); edit(); }} />
        {field === 'password' && <button type="button" className="btn btn-secondary compact" disabled={busy} aria-label={t(revealed ? 'codex.proxy.hideCredentials' : 'codex.proxy.showCredentials')} onClick={() => setRevealed(!revealed)}>{revealed ? <EyeOff size={14} /> : <Eye size={14} />}</button>}</div></label>)}
    </div>}
    {previewing && <p role="status" className="codex-proxy-page-note">{t('codex.proxy.catalog.recognizing')}</p>}
    {previewError && <div role="alert" tabIndex={-1} className="codex-proxy-page-error">{t(previewError)}</div>}
    {preview && <div className="codex-resource-preview">
      <p role="status">{t('codex.proxy.catalog.previewCount', { valid: preview.valid, invalid: preview.invalid, duplicates: preview.duplicates })}</p>
      <div className="codex-resource-preview-rows">{[...preview.rows].sort((a, b) => Number(!!b.error) - Number(!!a.error)).slice(0, 100).map((row) => <div key={row.line} className={row.error ? 'invalid' : ''}>
        <button type="button" className="btn btn-secondary compact" disabled={busy || preview.structured} onClick={() => locateLine(row.line)}>{preview.structured ? '#' + row.line : t('codex.proxy.catalog.line', { line: row.line })}</button><span>
          {row.error ? t(row.error === 'IMPORT_AMBIGUOUS' ? 'codex.proxy.catalog.ambiguous' : row.error === 'IMPORT_INVALID' ? 'codex.proxy.catalog.invalidInput' : catalogUnsupportedKey({ error: row.error })) : row.name}
          {!row.error && <small>{row.protocol} · {t(row.duplicate ? 'codex.proxy.catalog.duplicate' : row.authenticated ? 'codex.proxy.catalog.authenticated' : 'codex.proxy.catalog.noAuth')}</small>}</span></div>)}</div>
      {preview.rows.length > 100 && <p className="codex-proxy-page-note">{t('codex.proxy.catalog.previewLimit')}</p>}
      <div className="codex-resource-import-controls">
        {preview.invalid > 0 && <button type="button" className={'btn btn-secondary compact' + (skipInvalid ? ' active' : '')} disabled={busy} aria-pressed={skipInvalid} onClick={() => { setSkipInvalid(!skipInvalid); edit(); }}>{t('codex.proxy.catalog.skipInvalid')}</button>}
        {preview.duplicates > 0 && <button type="button" className={'btn btn-secondary compact' + (skipDuplicates ? ' active' : '')} disabled={busy} aria-pressed={skipDuplicates} onClick={() => { setSkipDuplicates(!skipDuplicates); edit(); }}>{t('codex.proxy.catalog.skipDuplicates')}</button>}</div>
    </div>}
    {invalid && mode === 'manual' && <div className="codex-proxy-page-error" role="alert">{t('codex.proxy.catalog.invalidInput')}</div>}
    {error && <div className="codex-proxy-page-error" role="alert" tabIndex={-1}>{error}</div>}
    <details className="codex-proxy-page-details codex-resource-import-notice"><summary>{t('common.advancedSettings')}</summary><p className="codex-proxy-page-note">{t('codex.proxy.catalog.importNotice')}</p></details>
    </div>
    <footer className="modal-footer">
      <button type="button" className="btn btn-secondary" disabled={busy && !onCancel} onClick={busy ? onCancel : onClose}>{t('common.cancel')}</button>
      <button type="button" className="btn btn-primary" disabled={busy || blocked} onClick={submit}>{busy ? <RefreshCw size={15} className="loading-spinner" /> : <Download size={15} />}{t(busy ? 'common.processing' : 'codex.proxy.catalog.import')}</button>
    </footer>
    </div>
  </div>, document.body);
}
