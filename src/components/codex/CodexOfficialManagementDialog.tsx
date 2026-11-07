import { useEffect, useRef, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Loader2, ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { codexOfficialAuthorizationState, codexOfficialAuthorizationStateKey } from '../../utils/codexOfficialAuthorization';
import { CodexOfficialBadge } from './CodexOfficialBadge';
import type { CodexOfficialAuthorizationRow } from '../../utils/codexOfficialAuthorization';
import type { ChatGptPlanReleaseDisposition } from '../../services/chatgptPlanService';
import { chatGptPlanErrorKey } from '../../services/chatgptPlanService';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { scrollElementIntoView } from '../../utils/reducedMotion';
import './CodexOfficialManagementDialog.css';

export function CodexOfficialManagementDialog({ row, busy, selectedMode, confirmation, children, onClose, onMode, onApply, onAuthorize, onCheck, onRelease, onConfirmation }: {
  row: CodexOfficialAuthorizationRow; busy: boolean; selectedMode: 'codex' | 'official';
  confirmation: ChatGptPlanReleaseDisposition | null; children: ReactNode;
  onClose: () => void; onMode: (mode: 'codex' | 'official') => void; onApply: () => void;
  onAuthorize: () => void; onCheck: () => void; onRelease: () => void;
  onConfirmation: (disposition: ChatGptPlanReleaseDisposition | null) => void;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null), confirmationArea = useRef<HTMLElement>(null);
  useEffect(() => { if (confirmation && confirmationArea.current) { confirmationArea.current.focus({ preventScroll: true }); scrollElementIntoView(confirmationArea.current, { block: 'nearest' }); } }, [confirmation]);
  useModalFocusTrap(dialog, true); useModalScrollLock(true);
  useEscCloseTopmost(true, () => confirmation ? onConfirmation(null) : onClose(), dialog);
  const state = codexOfficialAuthorizationState(row.grant, row.official && !row.paused);
  const checkedAt = row.grant?.last_checked_at;
  const checkedTime = checkedAt && Number.isFinite(checkedAt) ? new Date(checkedAt > 1e12 ? checkedAt : checkedAt * 1000).toLocaleString() : null;
  return createPortal(<div className="modal-overlay codex-official-management-overlay">
    <div ref={dialog} className="modal codex-official-management-dialog" role="dialog" aria-modal="true" aria-labelledby="codex-official-management-title" tabIndex={-1}>
      <div className="modal-header"><h2 id="codex-official-management-title">{t('chatgptPlan.officialFlow.manageAccount')}</h2></div>
      <div className="modal-body">
        <div className="codex-official-management-account"><ShieldCheck size={24} /><div><strong>{row.account.account_name || row.account.email}</strong>{row.account.account_name && <small>{row.account.email}</small>}<span>{t(row.paused ? 'codex.localAccess.accountPoolHealth.paused' : row.joined ? 'chatgptPlan.officialFlow.inService' : 'chatgptPlan.officialFlow.notInService')}</span><div className="codex-official-management-status">{row.grant && state !== 'disconnected' && <CodexOfficialBadge state={state} active={state === 'used'} />}{t(codexOfficialAuthorizationStateKey(state))}</div></div></div>
        {children}
        {row.joined ? <section><h3>{t('chatgptPlan.officialFlow.accessMode')}</h3><div className="codex-official-management-mode">
          <SingleSelectDropdown value={selectedMode} options={['codex', 'official'].map((value) => ({ value, label: t(value === 'official' ? 'chatgptPlan.officialFlow.rowOfficial' : 'chatgptPlan.officialFlow.rowLocal') }))}
            ariaLabel={t('chatgptPlan.officialFlow.accessMode')} disabled={busy || Boolean(confirmation)} onChange={(value) => onMode(value as 'codex' | 'official')} />
          <button type="button" className="btn btn-primary" disabled={busy || Boolean(confirmation) || (!row.paused && selectedMode === (row.official ? 'official' : 'codex'))} onClick={onApply}>{t(row.paused ? 'chatgptPlan.officialFlow.resumeAccess' : 'chatgptPlan.officialFlow.switchAccessMode')}</button>
        </div></section> : <p>{t('chatgptPlan.officialFlow.notMemberHint')}</p>}
        <section><h3>{t('chatgptPlan.officialFlow.authorizationDetails')}</h3><p>{t('chatgptPlan.officialFlow.checkHint')}</p><small className="codex-official-checked-at">{checkedTime ? t('chatgptPlan.officialFlow.lastChecked', { time: checkedTime }) : t('chatgptPlan.officialFlow.neverChecked')}</small>
          {row.grant?.check_error && <p className="codex-official-check-failure">{t('chatgptPlan.officialFlow.checkFailed', { error: t(chatGptPlanErrorKey(row.grant.check_error)) })}</p>}
          <div className="codex-official-detail-actions">
            <button type="button" className="btn btn-secondary" disabled={busy || Boolean(confirmation) || !row.grant || (!row.grant.connected && !row.grant.needs_reauth)} onClick={onCheck}>{t('chatgptPlan.officialFlow.checkAuthorization')}</button>
            <button type="button" className="btn btn-secondary" disabled={busy || Boolean(confirmation)} onClick={onAuthorize}>{t('chatgptPlan.officialFlow.authorizeAgain')}</button>
            {row.grant && <button type="button" className="btn btn-danger" disabled={busy || Boolean(confirmation)} onClick={() => onConfirmation(row.official ? 'local_gateway' : 'pause')}>{t('chatgptPlan.officialFlow.releaseAuthorization')}</button>}
          </div>
        </section>
        {confirmation && <section ref={confirmationArea} tabIndex={-1} className="codex-official-confirm"><h3>{t('chatgptPlan.officialFlow.releaseTitle')}</h3><p>{t('chatgptPlan.officialFlow.releaseHint')}</p>
          {row.joined && row.official ? <div className="codex-official-release-options">
            {(['local_gateway', 'pause'] as const).map((disposition) => <button type="button" key={disposition} className={`btn btn-secondary codex-official-release-option${confirmation === disposition ? ' is-active' : ''}`} disabled={busy} aria-pressed={confirmation === disposition} onClick={() => onConfirmation(disposition)}>
              <strong>{t(disposition === 'pause' ? 'chatgptPlan.officialFlow.releasePause' : 'chatgptPlan.officialFlow.releaseLocal')}</strong><span>{t(disposition === 'pause' ? 'chatgptPlan.officialFlow.releasePauseHint' : 'chatgptPlan.officialFlow.releaseLocalHint')}</span>
            </button>)}
          </div> : <p>{t('chatgptPlan.officialFlow.releaseUnchangedHint')}</p>}
          <div><button type="button" className="btn btn-secondary" disabled={busy} onClick={() => onConfirmation(null)}>{t('common.cancel')}</button><button type="button" className="btn btn-danger" disabled={busy} onClick={onRelease}>{t('chatgptPlan.officialFlow.releaseAuthorization')}</button></div>
        </section>}
        {busy && <p className="codex-official-loading" role="status"><Loader2 size={16} className="codex-official-spin" />{t('common.processing')}</p>}
      </div>
      <div className="modal-footer"><button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.close')}</button></div>
    </div>
  </div>, document.body);
}
