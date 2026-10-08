import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { CheckCircle2, Download, Loader2, RefreshCw, RotateCcw, ShieldAlert, Trash2, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ModalErrorMessage, useModalErrorState } from './ModalErrorMessage';
import { useModalScrollLock } from '../hooks/useModalScrollLock';
import { useEscCloseTopmost } from '../hooks/useEscClose';
import * as service from '../services/codexRecycleBinService';
import './CodexRecycleBinModal.css';

interface Props {
  onClose: () => void;
  onRestored: () => Promise<void>;
  maskAccountText: (value: string) => string;
}
type Confirmation = { kind: 'delete'; account: service.CodexRecycledAccount } | { kind: 'empty'; ids: string[] };

export function CodexRecycleBinModal({ onClose, onRestored, maskAccountText }: Props) {
  const { t } = useTranslation();
  const [accounts, setAccounts] = useState<service.CodexRecycledAccount[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [exported, setExported] = useState(false);
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const error = useModalErrorState();
  const mounted = useRef(true);
  const busyRef = useRef(false);
  const sequence = useRef(0);
  useModalScrollLock(true);
  useEscCloseTopmost(true, () => {
    if (confirmation && !busy) { setConfirmation(null); error.clear(); }
    else onClose();
  });
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; sequence.current += 1; };
  }, []);
  const reload = useCallback(async () => {
    const request = ++sequence.current;
    setLoading(true);
    error.clear();
    try {
      const result = await service.listCodexRecycledAccounts();
      if (mounted.current && sequence.current === request) setAccounts(result);
    } catch (cause) {
      if (mounted.current && sequence.current === request) error.report(
        String(cause).includes('CODEX_RECYCLE_BIN_READ_TIMEOUT')
          ? t('common.recycleBin.loadTimeout')
          : t('common.recycleBin.operationFailed', { error: String(cause) }),
      );
    } finally {
      if (mounted.current && sequence.current === request) setLoading(false);
    }
  }, [error.clear, error.report, t]);
  useEffect(() => { void reload(); }, [reload]);

  const execute = async (action: () => Promise<void | boolean>, removedIds: string[], restored = false) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    setExported(false);
    error.clear();
    try {
      if (await action() === false) return;
      if (mounted.current && removedIds.length > 0) {
        // The mutation has committed. Never offer the completed action again if a read fails.
        setAccounts((previous) => previous.filter((account) => !removedIds.includes(account.id)));
        setConfirmation(null);
        await reload();
      }
      if (restored) {
        try { await onRestored(); }
        catch {
          if (mounted.current) error.report(t('common.recycleBin.refreshFailed'));
        }
      }
    } catch (cause) {
      if (mounted.current) error.report(t('common.recycleBin.operationFailed', { error: String(cause) }));
    } finally {
      busyRef.current = false;
      if (mounted.current) setBusy(false);
    }
  };
  const save = async (ids: string[]) => {
    const saved = await service.saveCodexRecycledAccounts(ids);
    if (saved && mounted.current) setExported(true);
    return saved;
  };
  const requestConfirmation = (next: Confirmation) => {
    error.clear();
    setExported(false);
    setConfirmation(next);
  };
  const confirm = (exportFirst = false) => {
    if (!confirmation) return;
    const snapshot = confirmation;
    const ids = snapshot.kind === 'empty' ? snapshot.ids : [snapshot.account.id];
    void execute(async () => {
      if (exportFirst && !await save(ids)) return false;
      if (snapshot.kind === 'empty') await service.emptyCodexRecycleBin(ids);
      else await service.deleteCodexRecycledAccount(snapshot.account.id);
    }, ids);
  };
  const name = (account: service.CodexRecycledAccount) => maskAccountText(account.account_name || account.email || account.account_id);
  return createPortal(
    <div className="modal-overlay codex-recycle-bin-overlay">
      <div className="modal codex-recycle-bin-modal" role="dialog" aria-modal="true" aria-labelledby="codex-recycle-bin-title" aria-busy={busy || loading}>
        <div className="modal-header">
          <div className="codex-recycle-bin-heading"><span className="codex-recycle-bin-icon"><Trash2 size={19} /></span><h2 id="codex-recycle-bin-title">{t('common.recycleBin.title')}</h2></div>
          <button type="button" className="btn btn-secondary icon-only" onClick={onClose} aria-label={t('common.close')}><X size={18} /></button>
        </div>
        <div className="modal-body">
          <p className="codex-recycle-bin-description">{t('common.recycleBin.description')}</p>
          <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />
          {exported && <p className="codex-recycle-bin-notice" role="status"><CheckCircle2 size={15} />{t('common.recycleBin.exported')}</p>}
          {confirmation ? (
            <div className="codex-recycle-bin-confirm" role="alert">
              <ShieldAlert size={24} />
              <h3>{t('common.recycleBin.confirmTitle')}</h3>
              <p>{confirmation.kind === 'empty'
                ? t('common.recycleBin.confirmEmpty', { count: confirmation.ids.length })
                : t('common.recycleBin.confirmDelete', { name: name(confirmation.account) })}</p>
              <p className="codex-recycle-bin-export-hint">{t('common.recycleBin.exportHint')}</p>
            </div>
          ) : loading ? (
            <p className="codex-recycle-bin-empty" role="status"><Loader2 size={18} className="spin" />{t('common.loading')}</p>
          ) : accounts.length === 0 ? (
            <div className="codex-recycle-bin-empty"><Trash2 size={30} strokeWidth={1.4} /><p>{t('common.recycleBin.empty')}</p></div>
          ) : (
            <>
            <div className="codex-recycle-bin-toolbar">
              <span>{t('common.recycleBin.count', { count: accounts.length })}</span>
              <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => void execute(() => save(accounts.map((account) => account.id)), [])}><Download size={14} />{t('common.recycleBin.exportAll')}</button>
            </div>
            <ul className="codex-recycle-bin-list">
              {accounts.map((account) => (
                <li key={account.id} className="codex-recycle-bin-row">
                  <div className="codex-recycle-bin-account">
                    <div className="codex-recycle-bin-identity"><strong>{name(account)}</strong>
                    {account.plan_type && <span className="codex-recycle-bin-plan">{account.plan_type}</span>}</div>
                    <small>{t('common.recycleBin.deletedAt', { time: new Date(account.deleted_at * 1000).toLocaleString() })}</small>
                  </div>
                  <div className="codex-recycle-bin-actions">
                    <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => void execute(() => service.restoreCodexRecycledAccount(account.id), [account.id], true)}><RotateCcw size={14} />{t('common.recycleBin.restore')}</button>
                    <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => void execute(() => save([account.id]), [])}><Download size={14} />{t('common.recycleBin.export')}</button>
                    <button type="button" className="btn btn-danger" disabled={busy} onClick={() => requestConfirmation({ kind: 'delete', account })}><Trash2 size={14} />{t('common.recycleBin.deletePermanently')}</button>
                  </div>
                </li>
              ))}
            </ul>
            <p className="codex-recycle-bin-export-hint">{t('common.recycleBin.exportHint')}</p>
            </>
          )}
        </div>
        <div className="modal-footer">
          {busy && <span className="codex-recycle-bin-progress" role="status"><Loader2 size={14} className="spin" />{t('common.processing')}</span>}
          {confirmation ? <>
            <button type="button" className="btn btn-secondary" disabled={busy} onClick={() => { error.clear(); setExported(false); setConfirmation(null); }}>{t('common.cancel')}</button>
            <button type="button" className="btn btn-danger" disabled={busy} onClick={() => confirm()}>{t(confirmation.kind === 'empty' ? 'common.recycleBin.emptyBin' : 'common.recycleBin.deletePermanently')}</button>
            <button type="button" className="btn btn-primary" disabled={busy} onClick={() => confirm(true)}><Download size={14} />{t(confirmation.kind === 'empty' ? 'common.recycleBin.exportAndEmpty' : 'common.recycleBin.exportAndDelete')}</button>
          </> : <>
            <button type="button" className="btn btn-secondary" disabled={busy || loading} onClick={() => void reload()}><RefreshCw size={14} />{t('common.refresh')}</button>
            <button type="button" className="btn btn-danger" disabled={busy || loading || accounts.length === 0} onClick={() => requestConfirmation({ kind: 'empty', ids: accounts.map((account) => account.id) })}><Trash2 size={14} />{t('common.recycleBin.emptyBin')}</button>
          </>}
          {!confirmation && <button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.close')}</button>}
        </div>
      </div>
    </div>, document.body,
  );
}
