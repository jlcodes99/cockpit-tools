import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Loader2, ShieldCheck, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { announceCodexServiceAuthorizationApplied } from '../../utils/codexAccountServiceNavigation';
import type { CodexAccount } from '../../types/codex';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { ChatGptPlanDialog } from './ChatGptPlanDialog';
import {
  chatGptPlanErrorKey, setChatGptPlanServiceMode, withChatGptPlanMembership,
  type ChatGptPlanAccount,
} from '../../services/chatgptPlanService';
import './ChatGptPlan.css';
import './CodexAccountServiceDialog.css';

interface Props {
  sourceAccount: CodexAccount;
  memberAccountIds: readonly string[];
  registrations: readonly ChatGptPlanAccount[];
  initialMode?: 'codex' | 'official';
  onChanged: () => void | Promise<void>;
  onClose: () => void;
  onComplete?: () => void;
}

/** One explicit choice of service credentials; client credentials are never changed here. */
export function CodexAccountServiceDialog({ sourceAccount, memberAccountIds, registrations, initialMode, onChanged, onClose, onComplete }: Props) {
  const { t } = useTranslation();
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const active = useRef(true);
  const busyRef = useRef(false);
  const sourceRef = useRef(sourceAccount.id);
  sourceRef.current = sourceAccount.id;
  const changedRef = useRef(onChanged);
  changedRef.current = onChanged;
  const boundRegistrations = registrations.filter((item) => item.source_account_id === sourceAccount.id && item.source_binding_valid === true);
  const currentOfficial = boundRegistrations.find((item) => memberAccountIds.includes(`chatgpt_plan_${item.id}`));
  const currentMode = currentOfficial ? 'official' : memberAccountIds.includes(sourceAccount.id) ? 'codex' : undefined;
  const [mode, setMode] = useState<'codex' | 'official'>(initialMode ?? currentMode ?? 'codex');
  const [selected, setSelected] = useState<string | null>(currentOfficial?.id ?? (boundRegistrations.length === 1 ? boundRegistrations[0].id : null));
  const [locallyBound, setLocallyBound] = useState<ChatGptPlanAccount | null>(null);
  const [busy, setBusy] = useState(false);
  const error = useModalErrorState();
  const sourceEmail = sourceAccount.email?.trim().toLowerCase();
  const candidates = useMemo(() => registrations.filter((item) =>
    item.source_account_id === sourceAccount.id ||
    (!item.source_account_id && sourceEmail && item.email?.trim().toLowerCase() === sourceEmail)),
  [registrations, sourceAccount.id, sourceEmail]);
  const selectedAccount = locallyBound?.id === selected ? locallyBound : candidates.find((item) => item.id === selected);
  const officialFlow = mode === 'official' && (selected === 'new' || Boolean(selectedAccount));

  useEffect(() => {
    active.current = true;
    return () => { active.current = false; };
  }, []);
  useEscCloseTopmost(true, onClose, dialogRef);
  useModalFocusTrap(dialogRef, true);
  useModalScrollLock(true);

  const applyCodex = async () => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    error.clear();
    const sourceId = sourceAccount.id;
    try {
      const state = await setChatGptPlanServiceMode(sourceId, 'codex');
      await changedRef.current();
      const relatedIds = [sourceId, ...registrations.filter((item) => item.source_account_id === sourceId).map((item) => `chatgpt_plan_${item.id}`)];
      announceCodexServiceAuthorizationApplied(relatedIds, (state.collection?.accountIds ?? []).filter((id) => relatedIds.includes(id)));
      if (active.current && sourceRef.current === sourceId) (onComplete ?? onClose)();
    } catch (failure) {
      if (active.current && sourceRef.current === sourceId) error.report(t(chatGptPlanErrorKey(failure)));
    } finally {
      busyRef.current = false;
      if (active.current) setBusy(false);
    }
  };

  return createPortal(
    <div className="modal-overlay codex-account-service-overlay">
      <div ref={dialogRef} className="modal codex-account-service-dialog chatgpt-plan-embedded" role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1}>
        <div className="modal-header">
          <div><h2 id={titleId}>{t('chatgptPlan.serviceMode.title')}</h2><p>{sourceAccount.account_name || sourceAccount.email || sourceAccount.id}</p></div>
          <button type="button" className="btn btn-secondary" aria-label={t('common.close')} onClick={onClose}><X size={18} /></button>
        </div>
        <div className="codex-account-service-modes" role="tablist" aria-label={t('chatgptPlan.serviceMode.title')}>
          {(['codex', 'official'] as const).map((value) => <button key={value} type="button" role="tab" aria-selected={mode === value}
            className={`btn ${mode === value ? 'btn-primary' : 'btn-secondary'}`} disabled={busy}
            onClick={() => { error.clear(); setMode(value); }}>
            {value === 'official' && <ShieldCheck size={15} />}{t(`chatgptPlan.serviceMode.${value}`)}
          </button>)}
        </div>
        {currentMode && <p className="codex-account-service-current">{t('chatgptPlan.serviceMode.current')}: {t(`chatgptPlan.serviceMode.${currentMode}`)}</p>}
        {officialFlow ? <ChatGptPlanDialog embedded key={selectedAccount?.id ?? 'new'}
          account={selectedAccount ? withChatGptPlanMembership(selectedAccount, memberAccountIds) : undefined}
          sourceAccount={sourceAccount} memberAccountIds={memberAccountIds}
          onBound={(account) => { setLocallyBound(account); setSelected(account.id); }}
          onClose={onClose} onChanged={onChanged} onJoined={onComplete ?? onClose} /> : <>
          <div className="modal-body">
            <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />
            <p className="chatgpt-plan-description">{t(mode === 'codex' ? 'chatgptPlan.serviceMode.codexHint' : 'chatgptPlan.serviceMode.officialHint')}</p>
            {mode === 'official' && <div className="codex-account-service-source">
              {candidates.length > 0 && <>
                <label>{t('chatgptPlan.serviceMode.existingAuthorization')}</label>
                <SingleSelectDropdown value="" options={candidates.map((item) => ({ value: item.id, label: `${item.label || item.email || item.id} · ${item.id.slice(0, 8)}` }))}
                  ariaLabel={t('chatgptPlan.serviceMode.existingAuthorization')} placeholder={t('chatgptPlan.serviceMode.existingAuthorization')}
                  onChange={(value) => { error.clear(); setSelected(value); }} />
                <p className="chatgpt-plan-description">{t('chatgptPlan.serviceMode.existingHint')}</p>
              </>}
              <button type="button" className="btn btn-primary" onClick={() => { error.clear(); setSelected('new'); }}>
                <ShieldCheck size={16} />{t('chatgptPlan.serviceMode.newAuthorization')}
              </button>
            </div>}
          </div>
          <div className="modal-footer">
            <button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.cancel')}</button>
            {mode === 'codex' && <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void applyCodex()}>
              {busy && <Loader2 size={15} className="chatgpt-plan-spin" />}{t('chatgptPlan.serviceMode.applyCodex')}
            </button>}
          </div>
        </>}
      </div>
    </div>, document.body,
  );
}
