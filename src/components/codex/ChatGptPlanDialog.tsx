import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Check, CheckCircle2, Copy, ExternalLink, Globe, Info, Loader2, RefreshCw, ShieldCheck, Unplug, X } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { scrollElementIntoView } from '../../utils/reducedMotion';
import { announceCodexServiceAuthorizationApplied } from '../../utils/codexAccountServiceNavigation';
import type { CodexAccount } from '../../types/codex';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import {
  cancelChatGptPlanLogin, chatGptPlanAuthorizationUrl, chatGptPlanErrorKey, chatGptPlanStatusKey,
  disconnectChatGptPlan, joinChatGptPlan, listChatGptPlanAccounts, listChatGptPlanModels,
  pollChatGptPlanLogin, removeChatGptPlanFromService, startChatGptPlanLogin, verifyChatGptPlan,
  withChatGptPlanMembership, submitChatGptPlanCallbackUrl, openChatGptPlanIncognitoWindow, bindChatGptPlanAccount, unlinkChatGptPlanAccount,
  type ChatGptPlanAccount, type ChatGptPlanLogin, type ChatGptPlanModel,
} from '../../services/chatgptPlanService';

type Phase = 'connect' | 'waiting' | 'identity' | 'models' | 'disconnect' | 'unlink';
interface Props { sourceAccount?: CodexAccount; onBound?: (account: ChatGptPlanAccount) => void; onJoined?: () => void; embedded?: boolean; account?: ChatGptPlanAccount; memberAccountIds?: readonly string[]; expectedEmail?: string; discoveryError?: string; onLoginStart?: () => void; onClose: () => void; onChanged: () => void | Promise<void> }

function cancelLoginSafely(loginId: string): Promise<void> {
  return cancelChatGptPlanLogin(loginId).catch(() => {
    // Never log the authorization URL: it can contain an id_token_hint.
    console.warn('[ChatGPT Plan] login cancellation failed; the session will expire');
  });
}

export function ChatGptPlanDialog({ sourceAccount, onBound, onJoined, embedded = false, account: initialAccount, memberAccountIds, expectedEmail, discoveryError, onLoginStart, onClose, onChanged }: Props) {
  const { t } = useTranslation();
  const titleId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const active = useRef(true);
  const epoch = useRef(0);
  const busyRef = useRef(false);
  const loginRef = useRef<ChatGptPlanLogin | null>(null);
  const modelAttempt = useRef('');
  const autoPrepared = useRef(false);
  const callbackRef = useRef<HTMLInputElement>(null);
  const callbackId = useId();
  const changedRef = useRef(onChanged);
  const membersRef = useRef(memberAccountIds);
  membersRef.current = memberAccountIds;
  changedRef.current = onChanged;
  const [phase, setPhase] = useState<Phase>(initialAccount ? (sourceAccount && (initialAccount.source_account_id !== sourceAccount.id || initialAccount.source_binding_valid !== true) ? 'identity' : 'models') : 'connect');
  const [account, setAccount] = useState(initialAccount);
  const [login, setLogin] = useState<ChatGptPlanLogin | null>(null);
  const [models, setModels] = useState<ChatGptPlanModel[]>([]);
  const [model, setModel] = useState(initialAccount?.verified_model ?? '');
  const [modelsLoaded, setModelsLoaded] = useState(false);
  const [busy, setBusy] = useState('');
  const [notice, setNotice] = useState('');
  const [copied, setCopied] = useState(false);
  const [callbackInput, setCallbackInput] = useState('');
  const [callbackError, setCallbackError] = useState('');
  const [callbackAccepted, setCallbackAccepted] = useState(false);
  const error = useModalErrorState();
  const canCall = Boolean(account?.connected && !account.needs_reauth);
  const targetEmail = sourceAccount?.email || expectedEmail;
  const identityMismatch = Boolean(targetEmail && account && account.email?.trim().toLowerCase() !== targetEmail.trim().toLowerCase());
  const bindingRequired = Boolean(sourceAccount && (account?.source_account_id !== sourceAccount.id || account.source_binding_valid !== true));
  const bindingConflict = Boolean(sourceAccount && account?.source_account_id && account.source_account_id !== sourceAccount.id);

  useEffect(() => {
    if (!memberAccountIds) return;
    setAccount((previous) => {
      if (!previous) return previous;
      const next = withChatGptPlanMembership(previous, memberAccountIds);
      return next.in_service === previous.in_service ? previous : next;
    });
  }, [memberAccountIds, account?.id]);

  const close = () => {
    epoch.current++;
    const pending = loginRef.current;
    loginRef.current = null;
    if (pending) void cancelLoginSafely(pending.login_id);
    onClose();
  };
  useEscCloseTopmost(!embedded, close, dialogRef);
  useModalFocusTrap(dialogRef, !embedded);
  useModalScrollLock(!embedded);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
      epoch.current++;
      const pending = loginRef.current;
      loginRef.current = null;
      if (pending) void cancelLoginSafely(pending.login_id);
    };
  }, []);

  useEffect(() => {
    if (!callbackError || !callbackRef.current) return;
    callbackRef.current.focus();
    scrollElementIntoView(callbackRef.current, { block: 'center' });
  }, [callbackError]);

  const run = async (operation: string, action: () => Promise<void>) => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(operation);
    error.clear();
    setCallbackError('');
    setNotice('');
    try { await action(); }
    catch (failure) {
      if (active.current) {
        let key = chatGptPlanErrorKey(failure);
        if (key === 'chatgptPlan.error.reauthorize' || key === 'chatgptPlan.error.permission') {
          setAccount((previous) => previous ? { ...previous, needs_reauth: true } : previous);
          try { await changedRef.current(); }
          catch (refreshFailure) { key = chatGptPlanErrorKey(refreshFailure); }
        }
        if (operation === 'callback' && ['chatgptPlan.error.callback', 'chatgptPlan.error.mismatch', 'chatgptPlan.error.denied'].includes(key)) {
          setCallbackError(t(key));
        } else error.report(t(key));
      }
    }
    finally {
      busyRef.current = false;
      if (active.current) setBusy('');
    }
  };

  const changedInBackground = () => {
    void Promise.resolve().then(() => changedRef.current()).catch((failure) => {
      if (active.current) error.report(t(chatGptPlanErrorKey(failure)));
    });
  };

  const reloadModels = useCallback(async (currentAccount: ChatGptPlanAccount) => {
    const revision = epoch.current;
    const available = await listChatGptPlanModels(currentAccount.id);
    if (!active.current || revision !== epoch.current) return;
    setModels(available);
    setModelsLoaded(true);
    setModel((current) => available.some((item) => item.id === current) ? current : available[0]?.id ?? '');
  }, []);

  useEffect(() => {
    if (phase !== 'models' || !canCall || identityMismatch || bindingRequired || modelsLoaded || busyRef.current || !account || modelAttempt.current === account.id) return;
    modelAttempt.current = account.id;
    void run('models', () => reloadModels(account));
  }, [phase, canCall, identityMismatch, bindingRequired, modelsLoaded, account?.id, reloadModels, busy]);

  useEffect(() => {
    if (!login || phase !== 'waiting') return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      try {
        const expiry = login.expires_at > 1e12 ? login.expires_at : login.expires_at * 1000;
        if (Date.now() >= expiry) {
          void cancelLoginSafely(login.login_id);
          if (stopped || !active.current) return;
          loginRef.current = null;
          setLogin(null);
          setPhase('connect');
          error.report(t('chatgptPlan.error.expired'));
          return;
        }
        const status = await pollChatGptPlanLogin(login.login_id);
        if (stopped || !active.current) return;
        if (status.status === 'complete' && status.account) {
          loginRef.current = null;
          setLogin(null);
          const authorized = status.account;
          setAccount((previous) => withChatGptPlanMembership(authorized, membersRef.current, previous));
          setModelsLoaded(false);
          modelAttempt.current = '';
          setPhase(sourceAccount && (authorized.source_account_id !== sourceAccount.id || authorized.source_binding_valid !== true) ? 'identity' : initialAccount ? 'models' : 'identity');
          error.clear();
          setCallbackError('');
          setCallbackInput('');
          changedInBackground();
          return;
        }
        if (['error', 'cancelled', 'expired'].includes(status.status)) {
          if (status.account) {
            const draft = status.account;
            setAccount((previous) => withChatGptPlanMembership(draft, membersRef.current, previous));
            changedInBackground();
          }
          loginRef.current = null;
          setLogin(null);
          setPhase('connect');
          error.report(t(chatGptPlanErrorKey(status.error ?? `CHATGPT_PLAN_LOGIN_${status.status.toUpperCase()}`)));
          return;
        }
        // The server owns expiration and session cleanup; polling never overlaps.
        timer = setTimeout(() => void poll(), 1500);
      } catch (failure) {
        if (stopped || !active.current) return;
        error.report(t(chatGptPlanErrorKey(failure)));
        timer = setTimeout(() => void poll(), 5000);
      }
    };
    void poll();
    return () => { stopped = true; if (timer) clearTimeout(timer); };
  }, [login, phase, t]);

  const startLogin = () => {
    onLoginStart?.();
    return run('login', async () => {
    const revision = epoch.current;
    const next = await startChatGptPlanLogin(account?.id);
    if (!active.current || revision !== epoch.current) {
      await cancelLoginSafely(next.login_id);
      return;
    }
    loginRef.current = next;
    setLogin(next);
    setPhase('waiting');
    setCopied(false);
    setCallbackInput('');
    setCallbackAccepted(false);
    });
  };

  // Preparing a link never opens a browser or grants access. Existing-account
  // discovery is allowed to finish before an email-specific shortcut starts.
  useEffect(() => {
    if (autoPrepared.current || initialAccount || (expectedEmail && !sourceAccount) || discoveryError || phase !== 'connect') return;
    autoPrepared.current = true;
    void startLogin();
  }, [initialAccount, expectedEmail, sourceAccount, discoveryError, phase]);

  const submitCallback = () => run('callback', async () => {
    const current = loginRef.current;
    if (!current || !callbackInput.trim() || callbackAccepted) return;
    await submitChatGptPlanCallbackUrl(current.login_id, callbackInput.trim());
    if (active.current && loginRef.current?.login_id === current.login_id) {
      setCallbackAccepted(true);
      setCallbackInput('');
    }
  });

  const refreshAccount = async (id: string) => {
    const next = (await listChatGptPlanAccounts()).find((item) => item.id === id);
    if (active.current && next) setAccount((previous) => withChatGptPlanMembership(next, membersRef.current, previous));
  };

  const verify = (join: boolean) => run(join ? 'join' : 'verify', async () => {
    if (!account || !model || identityMismatch) return;
    if (bindingRequired) throw new Error('CHATGPT_PLAN_BINDING_REQUIRED');
    if (join) {
      await joinChatGptPlan(account.id, model);
      if (active.current) setAccount((previous) => previous ? { ...previous, in_service: true } : previous);
    }
    else {
      const next = await verifyChatGptPlan(account.id, model);
      if (active.current) setAccount((previous) => withChatGptPlanMembership(next, membersRef.current, previous));
    }
    await changedRef.current();
    if (join && sourceAccount) announceCodexServiceAuthorizationApplied(
      [sourceAccount.id, `chatgpt_plan_${account.id}`], [`chatgpt_plan_${account.id}`],
    );
    if (active.current) setNotice(join ? 'chatgptPlan.joinedNotice' : 'chatgptPlan.verifiedNotice');
    if (join && onJoined) { if (active.current) onJoined(); return; }
    await refreshAccount(account.id);
  });

  const bind = () => run('bind', async () => {
    if (!sourceAccount || !account || identityMismatch || bindingConflict || !bindingRequired) return;
    const result = await bindChatGptPlanAccount(account.id, sourceAccount.id);
    if (result.source_account_id !== sourceAccount.id || result.source_binding_valid !== true) throw new Error('CHATGPT_PLAN_BINDING_REQUIRED');
    const bound = withChatGptPlanMembership(result, membersRef.current, account);
    if (active.current) {
      setAccount(bound);
      setPhase('models');
      setNotice('chatgptPlan.serviceMode.linkedNotice');
      onBound?.(bound);
    }
    await changedRef.current();
  });

  const remove = () => run('remove', async () => {
    if (!account) return;
    await removeChatGptPlanFromService(account.id);
    if (active.current) setAccount((previous) => previous ? { ...previous, in_service: false } : previous);
    await changedRef.current();
    if (active.current) setNotice('chatgptPlan.removedNotice');
  });

  const disconnect = () => run('disconnect', async () => {
    if (!account) return;
    const result = await disconnectChatGptPlan(account.id);
    if (active.current) {
      setAccount((previous) => previous ? { ...previous, connected: false, in_service: false } : previous);
      setModels([]);
      setModelsLoaded(false);
      setPhase('models');
    }
    await changedRef.current();
    if (active.current) setNotice(result.remote_revoked ? 'chatgptPlan.disconnectedNotice' : 'chatgptPlan.localDisconnectedNotice');
  });

  const unlink = () => run('unlink', async () => {
    if (!account?.source_account_id) return;
    const result = await unlinkChatGptPlanAccount(account.id);
    if (active.current) {
      setAccount({ ...withChatGptPlanMembership(result), in_service: false });
      setPhase(sourceAccount ? 'identity' : 'models');
    }
    await changedRef.current();
    if (active.current) close();
  });

  const changePhase = (next: Phase) => { error.clear(); setCallbackError(''); setNotice(''); setPhase(next); };

  const content = <>
        {!embedded && <div className="modal-header">
          <h2 id={titleId}>{t(initialAccount ? 'chatgptPlan.manageTitle' : 'chatgptPlan.add')}</h2>
          <button type="button" className="btn btn-secondary chatgpt-plan-icon-button" onClick={close} aria-label={t('common.close')}><X size={18} /></button>
        </div>}
        <div className="modal-body">
          <ModalErrorMessage message={error.message || (discoveryError ? t(discoveryError) : '')} scrollKey={error.scrollKey} />
          {identityMismatch ? <ModalErrorMessage message={t('chatgptPlan.error.mismatch')} /> : null}
          {bindingConflict ? <ModalErrorMessage message={t('chatgptPlan.serviceMode.bindConflict')} /> : null}
          {sourceAccount && account?.source_binding_error && bindingRequired && !bindingConflict ? <ModalErrorMessage message={t(chatGptPlanErrorKey(account.source_binding_error))} /> : null}
          {notice ? <div className="chatgpt-plan-notice" role="status"><CheckCircle2 size={16} />{t(notice)}</div> : null}
          {(phase === 'connect' || phase === 'waiting') ? (
            <div className="chatgpt-plan-connect oauth-section">
              <p className="chatgpt-plan-oauth-summary"><ShieldCheck size={15} />{t('chatgptPlan.oauthSummary')}</p>
              <p className="chatgpt-plan-description">{t('chatgptPlan.oauthLimits')}</p>
              {expectedEmail ? <p className="chatgpt-plan-description">{expectedEmail}</p> : null}
              {login ? <div className="oauth-url-section">
                <div className="oauth-link">
                  <label htmlFor={`${callbackId}-authorization`}>{t('accounts.oauth.linkLabel')}</label>
                  <div className="oauth-url-box">
                    <input id={`${callbackId}-authorization`} type="text" value={login.authorize_url} readOnly spellCheck={false} />
                    <button type="button" className="btn btn-secondary" disabled={Boolean(busy)}
                      aria-label={t(copied ? 'common.copied' : 'common.copy')}
                      onClick={() => void run('copy', async () => {
                        await navigator.clipboard.writeText(chatGptPlanAuthorizationUrl(login.authorize_url));
                        if (active.current) setCopied(true);
                      })}>{copied ? <Check size={16} /> : <Copy size={16} />}</button>
                  </div>
                </div>
                <button type="button" className="btn btn-primary btn-full" disabled={Boolean(busy)} onClick={() => void run('browser', async () => {
                  try { await openUrl(chatGptPlanAuthorizationUrl(login.authorize_url)); }
                  catch { throw new Error('CHATGPT_PLAN_OPEN_BROWSER'); }
                })}><Globe size={16} />{t('common.shared.oauth.openBrowser')}</button>
                <button type="button" className="btn btn-secondary btn-full" disabled={Boolean(busy)}
                  onClick={() => void run('browser', () => openChatGptPlanIncognitoWindow(login.login_id))}>
                  <ShieldCheck size={16} />{t('common.shared.oauth.incognitoWindow')}
                </button>
                <div className="oauth-link">
                  <label htmlFor={callbackId}>{t('common.shared.oauth.manualCallbackLabel')}</label>
                  <div className="oauth-url-box oauth-manual-input">
                    <input ref={callbackRef} id={callbackId} type="text" value={callbackInput} spellCheck={false} autoComplete="off"
                      aria-invalid={Boolean(callbackError)} aria-describedby={callbackError ? `${callbackId}-error` : undefined}
                      disabled={busy === 'callback' || callbackAccepted}
                      placeholder={t('chatgptPlan.callbackPlaceholder')}
                      onChange={(event) => { setCallbackInput(event.target.value); setCallbackError(''); error.clear(); }} />
                    <button type="button" className="btn btn-secondary oauth-copy-button"
                      disabled={Boolean(busy) || callbackAccepted || !callbackInput.trim()} onClick={() => void submitCallback()}>
                      {busy === 'callback' ? <Loader2 size={16} className="chatgpt-plan-spin" /> : <Check size={16} />}
                      <span className="oauth-copy-button-label">{t('accounts.oauth.continue')}</span>
                    </button>
                  </div>
                  {callbackError && <p id={`${callbackId}-error`} className="chatgpt-plan-callback-error" role="alert">{callbackError}</p>}
                </div>
                <div className="codex-oauth-auto-update-note" role="status"><Info size={14} />{t('common.shared.oauth.hint')}</div>
              </div> : <div className="oauth-loading" role="status">
                {busy === 'login' ? <><Loader2 size={18} className="chatgpt-plan-spin" />{t('chatgptPlan.progress.login')}</> :
                  <button type="button" className="btn btn-primary btn-full" disabled={Boolean(busy)} onClick={() => void startLogin()}>
                    <RefreshCw size={16} />{t('codex.oauth.timeoutRetry')}
                  </button>}
              </div>}
            </div>
          ) : null}
          {account && (phase === 'identity' || phase === 'models' || phase === 'disconnect' || phase === 'unlink') ? (
            <div className="chatgpt-plan-identity">
              <strong>{account.label || account.email}</strong>
              {account.email && account.email !== account.label ? <span>{account.email}</span> : null}
              <span className="chatgpt-plan-badge">{t(chatGptPlanStatusKey(account))}</span>
            </div>
          ) : null}
          {phase === 'identity' ? sourceAccount && bindingRequired ? <div className="chatgpt-plan-binding-confirmation">
            <h3>{t('chatgptPlan.serviceMode.bindTitle')}</h3>
            <p>{t('chatgptPlan.serviceMode.bindHint', { account: sourceAccount.account_name || sourceAccount.email || sourceAccount.id })}</p>
          </div> : <p>{t('chatgptPlan.identityHint')}</p> : null}
          {phase === 'identity' && bindingRequired && !canCall ? <button type="button" className="btn btn-primary" disabled={Boolean(busy)} onClick={() => void startLogin()}>{t('common.reauthorize')}</button> : null}
          {phase === 'identity' || phase === 'models' ? <p className="chatgpt-plan-description">{t('chatgptPlan.apiOnlyHint')}</p> : null}
          {phase === 'models' && account ? (
            <div className="chatgpt-plan-models">
              {canCall ? <>
                <div className="chatgpt-plan-model-heading">
                  <label>{t('chatgptPlan.model')}</label>
                  <button type="button" className="btn btn-secondary btn-sm" disabled={Boolean(busy)} onClick={() => void run('models', () => reloadModels(account))}><RefreshCw size={14} className={busy === 'models' ? 'chatgpt-plan-spin' : ''} />{t('common.refresh')}</button>
                </div>
                <SingleSelectDropdown value={model} options={models.map((item) => ({ value: item.id, label: item.display_name || item.id }))} onChange={(value) => { setModel(value); error.clear(); setNotice(''); }} disabled={Boolean(busy) || models.length === 0} ariaLabel={t('chatgptPlan.model')} placeholder={t(busy === 'models' ? 'common.loading' : 'chatgptPlan.selectModel')} />
                {modelsLoaded && models.length === 0 ? <p className="chatgpt-plan-description">{t('chatgptPlan.error.noModels')}</p> : null}
                <p className="chatgpt-plan-description">{t('chatgptPlan.verifyHint')}</p>
                <p className="chatgpt-plan-description">{t('chatgptPlan.serviceHint')}</p>
                <div className="chatgpt-plan-actions">
                  <button type="button" className="btn btn-secondary" disabled={Boolean(busy) || !model || identityMismatch} onClick={() => void verify(false)}>{t('chatgptPlan.verify')}</button>
                  {account.in_service ? <button type="button" className="btn btn-secondary" disabled={Boolean(busy)} onClick={() => void remove()}>{t('chatgptPlan.remove')}</button> : null}
                </div>
              </> : <>
                <p className="chatgpt-plan-description">{t('chatgptPlan.reauthorizeHint')}</p>
                <button type="button" className="btn btn-primary" disabled={Boolean(busy)} onClick={() => void startLogin()}>{t('common.reauthorize')}</button>
              </>}
              <div className="chatgpt-plan-account-tools">
                <button type="button" className="btn btn-secondary" disabled={Boolean(busy)} onClick={() => void run('usage', () => openUrl('https://chatgpt.com/settings/usage'))}><ExternalLink size={14} />{t('chatgptPlan.manageUsage')}</button>
                {canCall ? <button type="button" className="btn btn-secondary" disabled={Boolean(busy)} onClick={() => void startLogin()}>{t('common.reauthorize')}</button> : null}
                {account.connected ? <button type="button" className="btn btn-danger" disabled={Boolean(busy)} onClick={() => changePhase('disconnect')}><Unplug size={14} />{t('chatgptPlan.disconnect')}</button> : null}
              </div>
            </div>
          ) : null}
          {(phase === 'models' || phase === 'identity') && account?.source_account_id ? <div className="chatgpt-plan-actions"><button type="button" className="btn btn-secondary" disabled={Boolean(busy)} onClick={() => changePhase('unlink')}>{t('chatgptPlan.serviceMode.unlink')}</button></div> : null}
          {phase === 'unlink' ? <div className="chatgpt-plan-disconnect"><h3>{t('chatgptPlan.serviceMode.unlinkTitle')}</h3><p>{t('chatgptPlan.serviceMode.unlinkHint')}</p></div> : null}
          {phase === 'disconnect' ? <div className="chatgpt-plan-disconnect"><h3>{t('chatgptPlan.disconnectTitle')}</h3><p>{t('chatgptPlan.disconnectHint')}</p></div> : null}
        </div>
        <div className="modal-footer">
          {busy ? <span className="chatgpt-plan-progress" role="status"><Loader2 size={14} className="chatgpt-plan-spin" />{t(busy === 'callback' || busy === 'bind' || busy === 'unlink' ? 'common.processing' : `chatgptPlan.progress.${busy}`)}</span> : null}
          <button type="button" className="btn btn-secondary" onClick={phase === 'disconnect' || phase === 'unlink' ? () => changePhase(bindingRequired ? 'identity' : 'models') : close}>{t(phase === 'models' ? 'common.close' : 'common.cancel')}</button>
          {phase === 'identity' ? <>
            <button type="button" className="btn btn-secondary" onClick={close}>{t(bindingRequired ? 'chatgptPlan.serviceMode.saveLinkOnly' : 'chatgptPlan.saveOnly')}</button>
            {bindingRequired ? <button type="button" className="btn btn-primary" disabled={Boolean(busy) || identityMismatch || bindingConflict || !canCall} onClick={() => void bind()}>{t('chatgptPlan.serviceMode.bind')}</button> :
              <button type="button" className="btn btn-primary" disabled={identityMismatch} onClick={() => { if (!identityMismatch) changePhase('models'); }}>{t('chatgptPlan.continue')}</button>}
          </> : null}
          {phase === 'models' && canCall && !bindingRequired && !account?.in_service ? <button type="button" className="btn btn-primary" disabled={Boolean(busy) || !model || identityMismatch} onClick={() => void verify(true)}>{t('chatgptPlan.verifyAndJoin')}</button> : null}
          {phase === 'unlink' ? <button type="button" className="btn btn-danger" disabled={Boolean(busy)} onClick={() => void unlink()}>{t('chatgptPlan.serviceMode.unlink')}</button> : null}
          {phase === 'disconnect' ? <button type="button" className="btn btn-danger" disabled={Boolean(busy)} onClick={() => void disconnect()}>{t('chatgptPlan.disconnect')}</button> : null}
        </div>
      </>;
  if (embedded) return content;
  return createPortal(
    <div className="modal-overlay chatgpt-plan-overlay">
      <div ref={dialogRef} className="modal chatgpt-plan-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1}>
        {content}
      </div>
    </div>, document.body,
  );
}
