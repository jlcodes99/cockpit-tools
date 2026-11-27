import { useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Check, RefreshCw, ShieldCheck, X } from 'lucide-react';
import type { Account } from '../../types/account';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useAccountStore } from '../../stores/useAccountStore';
import '../../styles/pages/codex-proxy-accounts.css';

const SUPPORTED_PROXY_PREFIXES = ['http://', 'https://', 'socks5://', 'socks5h://'];

function isSupportedProxyUrl(value: string): boolean {
  const lowered = value.trim().toLowerCase();
  return SUPPORTED_PROXY_PREFIXES.some((prefix) => lowered.startsWith(prefix));
}

/**
 * Antigravity 账号代理弹层：交互与视觉对齐 CodexProxyAccountDialog（跟随统一设置 / 独立代理
 * 两种模式），出口数据模型为单值 URL（账号代理优先，未配置回落全局代理）。
 */
export function AntigravityProxyAccountDialog({ account, onClose }: {
  account: Account;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const updateAccountEgressProxy = useAccountStore((state) => state.updateAccountEgressProxy);
  const displayName = account.name?.trim() || account.email;
  const savedUrl = account.egress_proxy_url?.trim() ?? '';
  const [mode, setMode] = useState<'follow' | 'independent'>(() => (savedUrl ? 'independent' : 'follow'));
  const [draftUrl, setDraftUrl] = useState(savedUrl);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [savedNotice, setSavedNotice] = useState(false);
  const submitted = useRef(false);

  useEscCloseTopmost(true, () => { if (!busy) onClose(); }, dialog);
  useModalScrollLock(true);
  useModalFocusTrap(dialog, true);

  const effectiveLabel = savedUrl || t('codex.proxy.modeDefault');
  const trimmedDraft = draftUrl.trim();
  const canSave = !busy && (mode === 'follow' ? Boolean(savedUrl) : (trimmedDraft.length > 0 && trimmedDraft !== savedUrl));

  const chooseMode = (next: 'follow' | 'independent') => {
    setMode(next);
    setSavedNotice(false);
    setError('');
    if (next === 'independent') setDraftUrl(savedUrl);
    submitted.current = false;
  };

  const submit = async () => {
    if (busy) return;
    const target = mode === 'follow' ? null : draftUrl.trim();
    if (mode === 'independent' && (target === null || !isSupportedProxyUrl(target))) {
      setError(t('antigravity.proxy.invalidUrl'));
      return;
    }
    setBusy(true);
    setError('');
    setSavedNotice(false);
    submitted.current = true;
    try {
      const updated = await updateAccountEgressProxy(account.id, target);
      setSavedNotice(true);
      onClose();
      return updated;
    } catch (caught) {
      const message = String(caught);
      setError(message.includes('ANTIGRAVITY_EGRESS_PROXY_INVALID_URL')
        ? t('antigravity.proxy.invalidUrl')
        : message);
    } finally {
      setBusy(false);
    }
  };

  return createPortal(<div className="modal-overlay codex-proxy-account-dialog-overlay">
    <div className="modal codex-proxy-account-dialog" ref={dialog} role="dialog" aria-modal="true"
      aria-labelledby="antigravity-proxy-account-dialog-title" tabIndex={-1}>
      <header className="modal-header"><div><h2 id="antigravity-proxy-account-dialog-title">{displayName}</h2>
        <p>{t('codex.proxy.management')} · {effectiveLabel}</p></div>
        <button type="button" className="btn btn-secondary compact" disabled={busy} onClick={onClose}
          aria-label={t('common.close')}><X size={18} /></button>
      </header>
      <div className="modal-body">
        <ModalErrorMessage message={error} />
        {savedNotice && <p className="codex-proxy-account-saved" role="status"><Check size={16} />{t('codex.proxy.saved')}</p>}
        <div className="codex-proxy-account-mode-options" role="radiogroup" aria-label={t('codex.proxy.managerAccounts.mode')}>
          <button type="button" role="radio" aria-checked={mode === 'follow'}
            className={`btn codex-proxy-account-mode-option${mode === 'follow' ? ' active' : ''}`}
            disabled={busy} onClick={() => chooseMode('follow')}>
            <span className="codex-proxy-account-mode-dot">{mode === 'follow' && <Check size={12} />}</span>
            <span><strong>{t('codex.proxy.managerAccounts.following')}</strong>
              <small>{t('antigravity.proxy.followHint')}</small></span>
          </button>
          <button type="button" role="radio" aria-checked={mode === 'independent'}
            className={`btn codex-proxy-account-mode-option${mode === 'independent' ? ' active' : ''}`}
            disabled={busy} onClick={() => chooseMode('independent')}>
            <span className="codex-proxy-account-mode-dot">{mode === 'independent' && <Check size={12} />}</span>
            <span><strong>{t('codex.proxy.modeIndependent')}</strong>
              <small>{t('antigravity.proxy.independentHint')}</small></span>
          </button>
        </div>
        {mode === 'follow' ? <div className="codex-proxy-account-follow-preview"><ShieldCheck size={21} /><div>
          <span>{t('codex.proxy.managerAccounts.proxy')}</span><strong>{t('codex.proxy.modeDefault')}</strong>
        </div></div> : <label className="codex-proxy-field"><span>{t('codex.proxy.managerAccounts.proxy')}</span>
          <input value={draftUrl} disabled={busy} spellCheck={false}
            placeholder={t('settings.network.proxyUrlPlaceholder')}
            onChange={(event) => { setDraftUrl(event.target.value); setSavedNotice(false); setError(''); }} />
        </label>}
        <p className="codex-proxy-page-note">{t('antigravity.proxy.restartHint')}</p>
      </div>
      <footer className="modal-footer">
        <button type="button" className="btn btn-secondary" disabled={busy} onClick={onClose}>{t('common.close')}</button>
        <button type="button" className="btn btn-primary" disabled={!canSave} onClick={() => void submit()}>
          {busy ? <RefreshCw size={15} className="loading-spinner" /> : <Check size={15} />}{t('common.save')}
        </button>
      </footer>
    </div>
  </div>, document.body);
}
