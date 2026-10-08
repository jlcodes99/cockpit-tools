import { Download, FileUp, Loader2, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { open, save } from '@tauri-apps/plugin-dialog';
import { readTextFile, stat, writeTextFile } from '@tauri-apps/plugin-fs';
import { listenSafely } from '../../utils/tauriEventListener';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { exportCodexModelConfig, importCodexModelConfig, previewCodexModelConfigImport } from '../../services/codexService';
import type { CodexExperimentalModelDefinition, CodexModelConfigImportPreview } from '../../types/codex';
import { codexModelConfigErrorKey } from '../../utils/codexModelConfig';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import './CodexModelConfigTransferModal.css';
import { restartCodexLocalAccessSidecar } from '../../services/codexLocalAccessService';

interface Props {
  instanceId: string;
  onClose: () => void;
  onImported: (models: CodexExperimentalModelDefinition[], defaultModelId: string | null) => void;
}

export function CodexModelConfigTransferModal({ instanceId, onClose, onImported }: Props) {
  const { t } = useTranslation();
  const [jsonContent, setJsonContent] = useState('');
  const [strategy, setStrategy] = useState<'keep_existing' | 'replace'>('keep_existing');
  const [preview, setPreview] = useState<CodexModelConfigImportPreview | null>(null);
  const [busy, setBusy] = useState<'preview' | 'import' | 'export' | 'file' | 'reload' | null>(null);
  const [applyFailed, setApplyFailed] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [fieldError, setFieldError] = useState<string | null>(null);
  const error = useModalErrorState();
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const previewRef = useRef<HTMLDivElement | null>(null);
  const dialogRef = useRef<HTMLDivElement | null>(null);
  // Ignore late read/preview responses after closing or changing the target.
  const generation = useRef(0);
  const mounted = useRef(true);
  const mutationInFlight = useRef(false);
  const applyWatch = useRef<{ revision: string; instanceId: string; generation: number } | null>(null);
  const applyErrorRevision = useRef<string | null>(null);
  useEscCloseTopmost(true, onClose);
  useModalFocusTrap(dialogRef, true);
  useModalScrollLock(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; generation.current += 1; };
  }, []);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listenSafely<{ instanceId: string; revision: string; error: string }>('codex-model-config-apply-error', ({ payload }) => {
      const watch = applyWatch.current;
      if (disposed || !mounted.current || !watch || watch.revision !== payload.revision
        || watch.instanceId !== payload.instanceId || watch.generation !== generation.current) return;
      applyErrorRevision.current = payload.revision;
      setApplyFailed(true); setNotice(null);
      error.report(t('codex.modelConfig.errors.apply', { error: payload.error }));
    }).then((stop) => { if (disposed) stop(); else unlisten = stop; }).catch((value) => {
      console.warn('[CodexModelConfig] Failed to listen for gateway application errors', value);
      if (!disposed && mounted.current) error.report(t('codex.modelConfig.errors.read'));
    });
    return () => { disposed = true; unlisten?.(); };
  }, [t, instanceId]);
  useEffect(() => {
    generation.current += 1;
    setBusy(null); setPreview(null); error.clear(); setFieldError(null); setNotice(null);
    applyWatch.current = null; applyErrorRevision.current = null; setApplyFailed(false);
  }, [instanceId]);
  const clear = () => { error.clear(); setFieldError(null); setNotice(null); };
  const invalidate = () => { generation.current += 1; setBusy(null); setPreview(null); clear();
    applyWatch.current = null; setApplyFailed(false); };
  const report = (value: unknown, fallback = 'invalid') => error.report(t(codexModelConfigErrorKey(value, fallback)));

  async function chooseFile() {
    clear(); setBusy('file'); const request = ++generation.current;
    try {
      const selected = await open({ multiple: false, filters: [{ name: 'JSON', extensions: ['json'] }] });
      if (!selected || Array.isArray(selected) || !mounted.current || request !== generation.current) return;
      const info = await stat(selected);
      if (!mounted.current || request !== generation.current) return;
      if (info.size > 4 * 1024 * 1024) throw new Error('MODEL_CONFIG_TOO_LARGE');
      const content = await readTextFile(selected);
      if (!mounted.current || request !== generation.current) return;
      setJsonContent(content); setPreview(null);
    } catch (value) { if (mounted.current && request === generation.current) report(value, 'read'); }
    finally { if (mounted.current && request === generation.current) setBusy(null); }
  }

  async function previewImport() {
    clear(); setPreview(null);
    if (!jsonContent.trim()) {
      setFieldError(t('codex.modelConfig.errors.json')); inputRef.current?.focus();
      inputRef.current?.scrollIntoView({ block: 'center' }); return;
    }
    setBusy('preview'); const request = ++generation.current;
    try {
      const result = await previewCodexModelConfigImport({ instanceId, jsonContent, conflictStrategy: strategy });
      if (!mounted.current || request !== generation.current) return;
      setPreview(result);
      if (result.errors.length) {
        error.report(t('codex.modelConfig.errors.validation'));
        previewRef.current?.scrollIntoView({ block: 'start' });
      }
    } catch (value) {
      if (!mounted.current || request !== generation.current) return;
      const key = codexModelConfigErrorKey(value);
      if (['json', 'fields', 'version', 'tooLarge'].some((suffix) => key.endsWith(`.${suffix}`))) {
        setFieldError(t(key)); inputRef.current?.focus(); inputRef.current?.scrollIntoView({ block: 'center' });
      } else report(value);
    } finally { if (mounted.current && request === generation.current) setBusy(null); }
  }

  async function commitImport() {
    if (!preview || preview.errors.length || mutationInFlight.current) return;
    clear(); setBusy('import'); mutationInFlight.current = true;
    const request = ++generation.current;
    applyWatch.current = { revision: preview.revision, instanceId, generation: request };
    applyErrorRevision.current = null; setApplyFailed(false);
    try {
      const result = await importCodexModelConfig({ instanceId, jsonContent, conflictStrategy: strategy, expectedRevision: preview.revision });
      if (!mounted.current || request !== generation.current) return;
      if (result.committed > 0) onImported(result.models, result.defaultModelId);
      if (applyErrorRevision.current !== preview.revision) setNotice(t('codex.modelConfig.imported', { count: result.committed }));
      setPreview(null);
    } catch (value) {
      if (!mounted.current || request !== generation.current) return;
      applyWatch.current = null; setPreview(null); report(value, 'write');
    } finally {
      mutationInFlight.current = false;
      if (mounted.current && request === generation.current) setBusy(null);
    }
  }

  async function retryGateway() {
    clear(); setBusy('reload'); const request = ++generation.current;
    applyWatch.current = null;
    try {
      await restartCodexLocalAccessSidecar();
      if (!mounted.current || request !== generation.current) return;
      setApplyFailed(false); setNotice(t('codex.localAccess.restartSuccess'));
    } catch (value) {
      if (!mounted.current || request !== generation.current) return;
      setApplyFailed(true); error.report(t('codex.modelConfig.errors.apply', { error: String(value) }));
    } finally { if (mounted.current && request === generation.current) setBusy(null); }
  }

  async function exportConfig() {
    clear(); setBusy('export'); const request = ++generation.current;
    try {
      const content = await exportCodexModelConfig(instanceId);
      if (!mounted.current || request !== generation.current) return;
      const target = await save({ defaultPath: 'codex-model-config.json', filters: [{ name: 'JSON', extensions: ['json'] }] });
      if (!target || !mounted.current || request !== generation.current) return;
      await writeTextFile(target, content);
      if (mounted.current && request === generation.current) setNotice(t('codex.modelConfig.exported'));
    } catch (value) { if (mounted.current && request === generation.current) report(value, 'write'); }
    finally { if (mounted.current && request === generation.current) setBusy(null); }
  }

  return createPortal(<div className="modal-overlay codex-model-config-overlay">
    <div ref={dialogRef} tabIndex={-1} className="modal codex-model-config-dialog" role="dialog" aria-modal="true" aria-labelledby="codex-model-config-title">
      <div className="modal-header">
        <h2 id="codex-model-config-title">{t('codex.modelConfig.title')}</h2>
        <button type="button" className="btn btn-secondary" onClick={onClose} aria-label={t('common.close')}><X size={18} /></button>
      </div>
      <div className="modal-body">
        <p className="codex-model-config-description">{t('codex.modelConfig.description')}</p>
        <p className="codex-model-config-hint">{t('codex.modelConfig.importHint')}</p>
        <p className="codex-model-config-description">{t('codex.modelConfig.closeHint')}</p>
        <div className="codex-model-config-toolbar">
          <button type="button" className="btn btn-secondary" disabled={Boolean(busy)} onClick={() => void chooseFile()}><FileUp size={15} />{t('codex.modelConfig.file')}</button>
          <button type="button" className="btn btn-secondary" disabled={Boolean(busy)} onClick={() => void exportConfig()}><Download size={15} />{t('codex.modelConfig.export')}</button>
        </div>
        <label className="codex-model-config-field">
          <span>{t('codex.modelConfig.jsonLabel')}</span>
          <textarea ref={inputRef} rows={7} value={jsonContent} className={fieldError ? 'has-error' : ''} aria-invalid={Boolean(fieldError)}
            disabled={busy === 'import'} onChange={(event) => { invalidate(); setJsonContent(event.target.value); }} spellCheck={false} />
          {fieldError && <span className="codex-model-config-field-error" role="alert">{fieldError}</span>}
        </label>
        <label className="codex-model-config-field">
          <span>{t('codex.modelConfig.strategy')}</span>
          <SingleSelectDropdown value={strategy} options={[
            { value: 'keep_existing', label: t('codex.modelConfig.keepExisting') },
            { value: 'replace', label: t('codex.modelConfig.replace') },
          ]} disabled={Boolean(busy)} ariaLabel={t('codex.modelConfig.strategy')}
            onChange={(value) => { invalidate(); setStrategy(value as typeof strategy); }} />
          <small>{t('codex.modelConfig.strategyHint')}</small>
        </label>
        {preview && <div ref={previewRef} className="codex-model-config-preview">
          <div className="codex-model-config-counts">{(['added', 'updated', 'conflicts', 'skipped', 'errors'] as const).map((key) => <span key={key}>
            {t(`codex.modelConfig.actions.${({ conflicts: 'conflict', errors: 'error' } as Record<string, string>)[key] ?? key}`)}: {preview[key].length}
          </span>)}</div>
          <table><thead><tr><th>{t('codex.modelConfig.section')}</th><th>{t('codex.modelConfig.item')}</th><th>{t('codex.modelConfig.result')}</th></tr></thead>
            <tbody>{preview.entries.map((entry, index) => <tr key={`${entry.section}-${entry.id}-${index}`} className={entry.action === 'error' ? 'has-error' : ''}>
              <td>{t(`codex.modelConfig.sections.${entry.section}`)}</td><td><code>{entry.id}</code></td>
              <td>{t(`codex.modelConfig.actions.${entry.action}`)}{entry.errorCode && <small>{t(codexModelConfigErrorKey(entry.errorCode))}</small>}</td>
            </tr>)}</tbody>
          </table>
        </div>}
        {!preview && <p className="codex-model-config-description">{t('codex.modelConfig.previewFirst')}</p>}
      </div>
      <div className="codex-model-config-feedback">
        <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />
        {applyFailed && <button type="button" className="btn btn-secondary" disabled={Boolean(busy)} onClick={() => void retryGateway()}>{t('codex.localAccess.restartAction')}</button>}
        {notice && <p role="status">{notice}</p>}
      </div>
      <div className="modal-footer">
        <button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.close')}</button>
        <button type="button" className="btn btn-secondary" disabled={Boolean(busy)} onClick={() => void previewImport()}>
          {busy === 'preview' && <Loader2 size={15} className="spin" />}{t(busy === 'preview' ? 'codex.modelConfig.previewLoading' : 'codex.modelConfig.preview')}
        </button>
        <button type="button" className="btn btn-primary" disabled={Boolean(busy) || !preview || Boolean(preview.errors.length) || !(preview.added.length + preview.updated.length)} onClick={() => void commitImport()}>
          {busy === 'import' && <Loader2 size={15} className="spin" />}{t(busy === 'import' ? 'codex.modelConfig.importLoading' : 'codex.modelConfig.import')}
        </button>
      </div>
    </div>
  </div>, document.body);
}
