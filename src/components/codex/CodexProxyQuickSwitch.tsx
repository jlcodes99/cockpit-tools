import { useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { ArrowLeftRight, Check, RefreshCw, X } from 'lucide-react';
import type { CodexAccount } from '../../types/codex';
import type { CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';
import { restoreExitChoice } from '../../utils/codexProxyDraft';
import { sourceDefaultDraft } from '../../utils/codexProxySelection';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';
import { useCodexProxyExitEditor } from './useCodexProxyExitEditor';
import { useProxyLatency } from './useProxyLatency';
import { CodexProxyPicker } from './CodexProxyPicker';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import '../../styles/pages/codex-proxy-resources.css';
import '../../styles/pages/codex-proxy-switch-dialog.css';

/** A separate account-only dialog reusing the same draft and binding transaction. */
export function CodexProxyQuickSwitch({ accountId, displayName, initialBinding, bindingReady, onApplied, onClose, runtimeStatus }: {
  accountId: string; displayName: string; initialBinding: CodexAccount['egress_proxy']; bindingReady: boolean;
  onApplied(): void; onClose(): void; runtimeStatus?: CodexProxyRuntimeStatus | null;
}) {
  const { t } = useTranslation();
  const titleId = useId();
  const dialog = useRef<HTMLDivElement>(null);
  const { catalog, catalogLoading, catalogError, reloadCatalog, acceptCatalog } = useCodexProxyWorkspace();
  const editor = useCodexProxyExitEditor(accountId);
  const latency = useProxyLatency(editor.source);
  const [catalogPending, setCatalogPending] = useState(false);
  const initialized = useRef(false);
  const previousBusy = useRef(editor.busy);
  const applied = useRef(onApplied); applied.current = onApplied;
  const close = () => {
    if (editor.busy === 'save') return;
    latency.cancel();
    onClose();
  };
  useEscCloseTopmost(true, close);
  useModalScrollLock(true);
  useModalFocusTrap(dialog, true);
  useEffect(() => {
    if (catalogLoading || initialized.current || !bindingReady) return;
    initialized.current = true;
    if (!editor.bound && initialBinding) editor.select(restoreExitChoice(catalog, initialBinding));
  }, [catalogLoading, catalog, editor, initialBinding, bindingReady]);
  useEffect(() => {
    if (previousBusy.current === 'save' && !editor.busy && !editor.error && editor.notice) applied.current();
    previousBusy.current = editor.busy;
  }, [editor.busy, editor.error, editor.notice]);

  return createPortal(<div className="modal-overlay codex-proxy-switch-overlay">
    <div ref={dialog} className="modal codex-proxy-switch-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1}>
      <header className="modal-header">
        <div className="codex-proxy-switch-heading"><span className="codex-proxy-switch-icon"><ArrowLeftRight size={22} /></span>
          <div><h2 id={titleId}>{t('codex.proxy.quickSwitch')}</h2><p title={displayName}>{displayName}</p></div></div>
        <button type="button" className="btn btn-secondary compact" disabled={editor.busy === 'save'} onClick={close} aria-label={t('common.close')}><X size={18} /></button>
      </header>
      <div className="modal-body">
        <p className="codex-proxy-switch-note">{t('codex.proxy.quickSwitchHint')}</p>
        <ModalErrorMessage message={editor.error || catalogError} />
        {catalogError ? <button type="button" className="btn btn-secondary" onClick={reloadCatalog}>{t('common.retry')}</button>
          : catalogLoading ? <p role="status">{t('common.loading')}</p>
            : !catalog.sources.length ? <p>{t('codex.proxy.unified.emptyCatalog')}</p>
              : <div className="codex-proxy-switch-fields">
                <label className="codex-proxy-switch-source"><span>{t('codex.proxy.catalog.sources')}</span>
                  <SingleSelectDropdown value={editor.sourceId} disabled={Boolean(editor.busy) || catalogPending} ariaLabel={t('codex.proxy.catalog.sources')}
                    options={catalog.sources.map((source) => ({ value: source.id, label: source.name }))}
                    onChange={(sourceId) => {
                      initialized.current = true;
                      const preset = sourceDefaultDraft(catalog.sources.find((source) => source.id === sourceId));
                      editor.select({ sourceId, itemId: preset?.itemId ?? '', groupId: preset?.groupId ?? '', selections: preset?.selections ?? {} });
                    }} /></label>
                {editor.source && <CodexProxyPicker source={editor.source} itemId={editor.itemId} selectedGroupId={editor.groupId}
                  selections={editor.selections} busy={Boolean(editor.busy)} latency={latency} onCatalogChange={acceptCatalog} onPendingChange={setCatalogPending} runtimeStatus={editor.saved ? runtimeStatus : null}
                  choose={(itemId, groupId) => editor.select({ sourceId: editor.sourceId, itemId, groupId, selections: {} })}
                  chooseMember={(id, member) => editor.select({ sourceId: editor.sourceId, itemId: editor.itemId, groupId: editor.groupId,
                    selections: { ...editor.selections, [id]: member } })} />}
              </div>}
        <p className="codex-proxy-switch-note">{t('codex.proxy.restartHint')}</p>
        <p className="codex-proxy-switch-note">{t('codex.proxy.testNotice')}</p>
      </div>
      <footer className="modal-footer">
        <button type="button" className="btn btn-secondary" disabled={editor.busy === 'save'} onClick={close}>{t('common.cancel')}</button>
        <button type="button" className="btn btn-primary" disabled={Boolean(editor.busy) || catalogPending || catalogLoading || Boolean(catalogError) || !editor.selectionReady || editor.saved}
          onClick={() => editor.save()}>{editor.busy ? <RefreshCw size={15} className="loading-spinner" /> : <Check size={15} />}
          {t(editor.busy ? 'common.saving' : 'common.save')}</button>
      </footer>
    </div>
  </div>, document.body);
}
