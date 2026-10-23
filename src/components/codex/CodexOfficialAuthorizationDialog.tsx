import { useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Check, Copy, ExternalLink, Link2, Loader2, ShieldCheck } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import type { CodexAccount } from '../../types/codex';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { scrollElementIntoView } from '../../utils/reducedMotion';
import {
  bindChatGptPlanAccount, cancelChatGptPlanLogin, chatGptPlanAuthorizationUrl, chatGptPlanErrorKey,
  openChatGptPlanIncognitoWindow, pollChatGptPlanLogin,
  startChatGptPlanLogin, submitChatGptPlanCallbackUrl,
  type ChatGptPlanAccount, type ChatGptPlanLogin,
} from '../../services/chatgptPlanService';
import './CodexOfficialAuthorizationDialog.css';

export interface CodexOfficialAuthorizationDialogProps {
  sourceAccount: CodexAccount;
  registrations: ChatGptPlanAccount[];
  reauthorize?: boolean;
  completionMode?: 'bind' | 'draft' | 'service';
  onClose: () => void;
  onComplete: (grant: ChatGptPlanAccount) => void;
  onChanged?: () => void | Promise<void>;
}

/** Timeout only the wait; callers also guard each subsequent mutation with an epoch. */
function bounded<T>(request: Promise<T>, milliseconds = 30_000): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('CHATGPT_PLAN_TIMEOUT')), milliseconds);
    request.then((value) => { clearTimeout(timer); resolve(value); }, (failure) => { clearTimeout(timer); reject(failure); });
  });
}
function cancel(loginId: string) {
  void cancelChatGptPlanLogin(loginId).catch(() => {
    // Never log callback links, authorization links, or upstream error text.
    console.warn('[Official authorization] cancellation failed; login will expire');
  });
}

export function CodexOfficialAuthorizationDialog({ sourceAccount, registrations, reauthorize = false, completionMode = 'bind', onClose, onComplete, onChanged }: CodexOfficialAuthorizationDialogProps) {
  const { t } = useTranslation();
  const tr = (key: string) => t(`chatgptPlan.officialFlow.${key}`);
  const titleId = useId();
  const fieldId = useId();
  const dialogRef = useRef<HTMLDivElement>(null);
  const fieldRef = useRef<HTMLInputElement>(null);
  const alive = useRef(true);
  const epoch = useRef(0);
  const running = useRef(false);
  const finished = useRef(false);
  const loginRef = useRef<ChatGptPlanLogin | null>(null);
  const callbacks = useRef({ onChanged, onComplete });
  callbacks.current = { onChanged, onComplete };
  const inFlight = useRef(new Map<string, Promise<unknown>>());
  const [grant, setGrant] = useState(() => {
    const linked = registrations.filter((item) => item.source_account_id === sourceAccount.id);
    return linked.length === 1 ? linked[0] : undefined;
  });
  const [login, setLogin] = useState<ChatGptPlanLogin | null>(null);
  const [busy, setBusy] = useState('');
  const [copied, setCopied] = useState(false);
  const [callback, setCallback] = useState('');
  const [callbackError, setCallbackError] = useState('');
  const [callbackAccepted, setCallbackAccepted] = useState(false);
  const error = useModalErrorState();
  const available = registrations.filter((item) => !item.source_account_id || item.source_account_id === sourceAccount.id);
  const usable = Boolean(!reauthorize && grant?.connected && !grant.needs_reauth);
  const live = (revision: number) => alive.current && epoch.current === revision;
  const clear = () => { error.clear(); setCallbackError(''); };
  const notifyChanged = () => {
    void Promise.resolve().then(() => callbacks.current.onChanged?.()).catch(() => {
      if (alive.current) error.report(tr('refreshFailed'));
    });
  };
  const close = () => {
    alive.current = false;
    epoch.current++;
    const pending = loginRef.current;
    loginRef.current = null;
    if (pending) cancel(pending.login_id);
    onClose();
  };
  useEscCloseTopmost(true, close, dialogRef);
  useModalFocusTrap(dialogRef, true);
  useModalScrollLock(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      epoch.current++;
      const pending = loginRef.current;
      loginRef.current = null;
      if (pending) cancel(pending.login_id);
    };
  }, []);
  useEffect(() => {
    if (callbackError && fieldRef.current) {
      fieldRef.current.focus();
      scrollElementIntoView(fieldRef.current, { block: 'center' });
    }
  }, [callbackError]);

  const run = async (operation: string, action: (revision: number) => Promise<void>) => {
    if (running.current || !alive.current) return;
    const revision = epoch.current;
    running.current = true;
    setBusy(operation);
    clear();
    try { await action(revision); }
    catch (failure) {
      if (live(revision)) {
        const key = chatGptPlanErrorKey(failure);
        if (['chatgptPlan.error.reauthorize', 'chatgptPlan.error.permission'].includes(key)) {
          setGrant((previous) => previous ? { ...previous, needs_reauth: true } : previous);
        }
        if (operation === 'callback' && ['chatgptPlan.error.callback', 'chatgptPlan.error.mismatch', 'chatgptPlan.error.denied'].includes(key)) setCallbackError(t(key));
        else error.report(t(key));
      }
    } finally {
      running.current = false;
      if (live(revision)) setBusy('');
    }
  };

  // A UI wait timeout does not cancel a backend operation. Explicit retries reuse
  // that operation until it settles. Authorization never generates model output.
  const once = <T,>(key: string, request: () => Promise<T>): Promise<T> => {
    const pending = inFlight.current.get(key);
    if (pending) return pending as Promise<T>;
    const created = request();
    inFlight.current.set(key, created);
    const release = () => { if (inFlight.current.get(key) === created) inFlight.current.delete(key); };
    void created.then(release, release);
    return created;
  };

  const finish = async (candidate: ChatGptPlanAccount, revision: number) => {
    if (!live(revision)) return;
    if (candidate.source_account_id && candidate.source_account_id !== sourceAccount.id) throw new Error('CHATGPT_PLAN_BINDING_CONFLICT');
    if (!candidate.connected || candidate.needs_reauth) throw new Error('CHATGPT_PLAN_REAUTH_REQUIRED');
    let current = candidate;
    // Even a matching email is never sufficient: the backend checks source identity and revision.
    if (current.source_account_id !== sourceAccount.id || current.source_binding_valid !== true || current.source_binding_error) {
      const bound = await bounded(once(`bind:${current.id}`, () => bindChatGptPlanAccount(current.id, sourceAccount.id).then((value) => { notifyChanged(); return value; })));
      if (!live(revision)) return;
      if (bound.source_account_id !== sourceAccount.id || bound.source_binding_valid !== true || bound.source_binding_error) throw new Error('CHATGPT_PLAN_BINDING_REQUIRED');
      current = { ...bound, in_service: candidate.in_service };
    }
    setGrant(current);
    setBusy('validating');
    setGrant(current);
    if (live(revision)) { finished.current = true; callbacks.current.onComplete(current); }
  };

  const ensureLogin = async (revision: number, fresh = false) => {
    if (loginRef.current) return loginRef.current;
    let expired = false;
    const request = startChatGptPlanLogin(fresh ? undefined : grant?.id);
    void request.then((next) => { if (expired || !live(revision)) cancel(next.login_id); }, () => {});
    let next: ChatGptPlanLogin;
    try { next = await bounded(request); }
    catch (failure) { expired = true; throw failure; }
    if (!live(revision)) return null;
    try { chatGptPlanAuthorizationUrl(next.authorize_url); }
    catch (failure) { cancel(next.login_id); throw failure; }
    loginRef.current = next;
    setLogin(next);
    setCallbackAccepted(false);
    setCallback('');
    return next;
  };
  const browserAction = (kind: 'browser' | 'copy' | 'incognito', fresh = false) => run('preparing', async (revision) => {
    const next = await ensureLogin(revision, fresh);
    if (!next || !live(revision)) return;
    if (kind === 'copy') {
      await bounded(navigator.clipboard.writeText(chatGptPlanAuthorizationUrl(next.authorize_url)));
      if (live(revision)) setCopied(true);
    } else if (kind === 'incognito') await bounded(openChatGptPlanIncognitoWindow(next.login_id));
    else {
      try { await bounded(openUrl(chatGptPlanAuthorizationUrl(next.authorize_url))); }
      catch { throw new Error('CHATGPT_PLAN_OPEN_BROWSER'); }
    }
  });

  // Prepare the address on open without opening a browser or generating output.
  // Deferring one microtask avoids orphan starts during React StrictMode remounts.
  useEffect(() => {
    let stopped = false;
    void Promise.resolve().then(() => { if (!stopped && !finished.current && !usable) void run('preparing', async (revision) => { await ensureLogin(revision); }); });
    return () => { stopped = true; };
  }, [grant?.id, usable]);

  useEffect(() => {
    if (!login) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const revision = epoch.current;
    const poll = async () => {
      if (stopped || !live(revision)) return;
      try {
        const expiry = login.expires_at > 1e12 ? login.expires_at : login.expires_at * 1000;
        if (Date.now() >= expiry) throw new Error('CHATGPT_PLAN_LOGIN_EXPIRED');
        const status = await bounded(pollChatGptPlanLogin(login.login_id), 15_000);
        if (stopped || !live(revision)) return;
        if (status.status === 'complete' && status.account) {
          if (running.current) { timer = setTimeout(() => void poll(), 250); return; }
          loginRef.current = null;
          setLogin(null);
          const completed = { ...status.account, in_service: grant?.id === status.account.id ? grant.in_service : false };
          setGrant(completed);
          setCallback('');
          setCallbackError('');
          notifyChanged();
          void run('validating', (currentRevision) => finish(completed, currentRevision));
          return;
        }
        if (['error', 'cancelled', 'expired'].includes(status.status)) {
          if (status.account) { setGrant({ ...status.account, in_service: false }); notifyChanged(); }
          throw new Error(status.error ?? `CHATGPT_PLAN_LOGIN_${status.status.toUpperCase()}`);
        }
        timer = setTimeout(() => void poll(), 1500);
      } catch (failure) {
        if (stopped || !live(revision)) return;
        loginRef.current = null;
        setLogin(null);
        cancel(login.login_id);
        error.report(t(chatGptPlanErrorKey(failure)));
      }
    };
    void poll();
    return () => { stopped = true; if (timer) clearTimeout(timer); };
  }, [login, t]);

  const submit = () => run('callback', async (revision) => {
    const pending = loginRef.current;
    if (!pending || !callback.trim() || callbackAccepted) return;
    await bounded(submitChatGptPlanCallbackUrl(pending.login_id, callback.trim()));
    if (live(revision) && loginRef.current?.login_id === pending.login_id) { setCallbackAccepted(true); setCallback(''); }
  });

  return createPortal(<div className="modal-overlay codex-official-auth-overlay">
    <div ref={dialogRef} className="modal codex-official-auth-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1}>
      <div className="modal-header"><h2 id={titleId}>{tr('title')}</h2></div>
      <div className="modal-body">
        <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />
        <div className="official-auth-account"><span className="official-auth-emblem"><ShieldCheck size={23} /></span><div><span className="official-auth-caption">{tr('targetAccount')}</span><strong>{sourceAccount.account_name || sourceAccount.email || sourceAccount.id}</strong>{sourceAccount.account_name && sourceAccount.email ? <small>{sourceAccount.email}</small> : null}</div></div>
        <section className="official-auth-intro"><h3>{tr('benefitTitle')}</h3><p>{tr('benefit')}</p><p className="official-auth-limits">{tr('limits')}</p></section>
        <div className="official-auth-main">
          {(!usable || login) && <div className="official-auth-address"><label className="official-auth-caption" htmlFor={`${fieldId}-link`}>{tr('authorizationLink')}</label><input className="official-auth-input official-auth-link" id={`${fieldId}-link`} readOnly value={login?.authorize_url ?? ''} placeholder={tr('preparing')} spellCheck={false} />{login && <div className="official-auth-status" role="status"><Loader2 size={14} className="official-auth-spin" />{tr('waiting')}</div>}</div>}
          <button type="button" className="btn btn-primary official-auth-primary" disabled={Boolean(busy)} onClick={() => void (usable && !login ? run('validating', (revision) => finish(grant!, revision)) : browserAction('browser'))}>
            {busy ? <Loader2 size={17} className="official-auth-spin" /> : usable && !login ? <Check size={17} /> : <ExternalLink size={17} />}
            {tr(busy === 'validating' ? 'validating' : busy ? 'preparing' : usable && !login ? 'verifyAndBind' : 'openAuthorization')}
          </button>
          {!usable || login ? <div className="official-auth-secondary"><button type="button" className="btn btn-secondary" disabled={Boolean(busy)} onClick={() => void browserAction('copy')}><Copy size={15} />{t(copied ? 'common.copied' : 'chatgptPlan.copyLink')}</button><button type="button" className="btn btn-secondary" disabled={Boolean(busy)} onClick={() => void browserAction('incognito')}><ShieldCheck size={15} />{tr('incognito')}</button></div> : null}
          {usable && !login ? <button type="button" className="btn btn-secondary" disabled={Boolean(busy)} onClick={() => void browserAction('browser', !grant?.source_account_id)}>{tr('authorizeAgain')}</button> : null}

          <p className="official-auth-footnote">{tr(completionMode === 'draft' ? 'draftSelectionHint' : completionMode === 'service' ? 'joinHint' : 'bindHint')}</p>
          <p className="official-auth-footnote">{tr('verificationHint')}</p>
        </div>
        {!grant && available.length > 0 && !login ? <details className="official-auth-existing"><summary>{tr('existingAuthorization')}</summary><p className="official-auth-footnote">{tr('existingHint')}</p>{available.map((candidate) => <div className="official-auth-existing-row" key={candidate.id}><span><strong>{candidate.email || candidate.label}</strong><small>{candidate.id.slice(0, 8)}</small></span><button type="button" className="btn btn-secondary" disabled={Boolean(busy)} onClick={() => { if (candidate.connected && !candidate.needs_reauth) void run('validating', (revision) => finish(candidate, revision)); else { clear(); epoch.current++; if (loginRef.current) cancel(loginRef.current.login_id); loginRef.current = null; setLogin(null); running.current = false; setBusy(''); setGrant(candidate); } }}><Link2 size={14} />{tr('associate')}</button></div>)}</details> : null}
        {(!usable || login) && <section className="official-auth-manual"><h3>{tr('manualCallback')}</h3><label htmlFor={fieldId}>{tr('callbackLabel')}</label><div className="official-auth-callback-row"><input ref={fieldRef} id={fieldId} className="official-auth-input" value={callback} placeholder={tr('callbackPlaceholder')} autoComplete="off" spellCheck={false} disabled={!login || busy === 'callback' || callbackAccepted} aria-invalid={Boolean(callbackError)} aria-describedby={callbackError ? `${fieldId}-error` : undefined} onChange={(event) => { setCallback(event.target.value); clear(); }} /><button type="button" className="btn btn-secondary" disabled={!login || Boolean(busy) || !callback.trim() || callbackAccepted} onClick={() => void submit()}>{tr('submitCallback')}</button></div>{callbackError ? <p id={`${fieldId}-error`} className="official-auth-field-error" role="alert">{callbackError}</p> : null}{callbackAccepted ? <p role="status" className="official-auth-footnote">{tr('callbackAccepted')}</p> : null}</section>}

      </div>
      <div className="modal-footer"><span>{tr('footerHint')}</span><button type="button" className="btn btn-secondary" onClick={close}>{t('common.cancel')}</button></div>
    </div>
  </div>, document.body);
}
