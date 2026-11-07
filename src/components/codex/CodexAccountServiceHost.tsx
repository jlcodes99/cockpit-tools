import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { createPortal } from 'react-dom';
import { CheckCircle2, Loader2, Network, ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import { getCodexLocalAccessState } from '../../services/codexLocalAccessService';
import { chatGptPlanErrorKey, listChatGptPlanAccounts, setChatGptPlanServiceMode, isChatGptPlanServiceModeApplied, checkChatGptPlanAuthorization,
  releaseChatGptPlanAuthorization, type ChatGptPlanReleaseDisposition, type ChatGptPlanReleaseResult, type ChatGptPlanAccount } from '../../services/chatgptPlanService';
import { CODEX_ACCOUNT_SERVICE_REQUEST, takeCodexServiceAuthorizationRequests, announceCodexServiceAuthorizationApplied, type CodexAccountServiceRequest } from '../../utils/codexAccountServiceNavigation';
import { codexOfficialAuthorizationRows, codexOfficialAuthorizationDeadline,
  codexOfficialAuthorizationState, codexOfficialAuthorizationStateKey, type CodexOfficialAuthorizationRow } from '../../utils/codexOfficialAuthorization';
import { CODEX_ACCOUNT_SERVICE_DRAFT_CANCEL, announceCodexServiceDraftResult, cancelCodexServiceDraftAuthorizations, isCodexServiceDraftRequestActive, requeueCodexServiceAuthorizationRequests } from '../../utils/codexAccountServiceNavigation';
import type { CodexServiceMemberAccess } from '../../types/codexLocalAccess';
import type { CodexAccount } from '../../types/codex';
import { CodexOfficialAuthorizationDialog } from './CodexOfficialAuthorizationDialog';
import { CodexOfficialBadge } from './CodexOfficialBadge';
import { CODEX_OFFICIAL_AUTHORIZATION_MANAGE, requestCodexOfficialAuthorizationManagement, takeCodexOfficialAuthorizationManagement } from '../../utils/codexAccountServiceNavigation';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { CodexOfficialManagementDialog } from './CodexOfficialManagementDialog';
import './CodexAccountServiceHost.css';

interface Snapshot { running: boolean; accounts: CodexAccount[]; members: string[]; paused: string[]; serviceMembers?: CodexServiceMemberAccess[]; grants: ChatGptPlanAccount[] }
interface Request { kind: 'join' | 'draft' | 'manage'; items: CodexAccountServiceRequest[]; draftRequestId?: string }
type AfterAuthorization = 'none' | 'joinOfficial' | 'switchOfficial' | 'draftOfficial';
interface Selection { source: CodexAccount; after: AfterAuthorization; reauthorize?: boolean; draftRequestId?: string }
interface Confirmation { row: CodexOfficialAuthorizationRow; disposition: ChatGptPlanReleaseDisposition }
async function readSnapshot(): Promise<Snapshot> {
  return codexOfficialAuthorizationDeadline((async () => {
    if (!useCodexAccountStore.getState().accounts.length) await useCodexAccountStore.getState().fetchAccounts();
    const [state, grants] = await Promise.all([getCodexLocalAccessState(), listChatGptPlanAccounts()]);
    return { running: Boolean(state.running && state.collection?.enabled), accounts: useCodexAccountStore.getState().accounts, members: state.collection?.accountIds ?? [], paused: state.collection?.pausedAccountIds ?? [], serviceMembers: state.serviceMembers, grants };
  })());
}
function changed() {
  void useCodexAccountStore.getState().fetchAccounts().catch(() => {
    console.warn('[Official authorization] account list refresh failed');
  });
  window.dispatchEvent(new Event('chatgpt-plan-accounts-changed'));
  window.dispatchEvent(new Event('codex-local-access-state-updated'));
}

/** Handles explicit authorization actions only; service startup never opens this dialog. */
export function CodexAccountServiceHost({ activePage = true }: { activePage?: boolean }) {
  const { t } = useTranslation();
  const [request, setRequest] = useState<Request | null>(null);
  const requestRef = useRef(request); requestRef.current = request;
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [selection, setSelection] = useState<Selection | null>(null);
  const selectionRef = useRef(selection); selectionRef.current = selection;
  const deferredRequests = useRef<CodexAccountServiceRequest[]>([]);
  const deferredManage = useRef(false);
  const resumeDeferredRef = useRef(() => {});
  const [done, setDone] = useState<Set<string>>(new Set());
  const doneRef = useRef(done); doneRef.current = done;
  const [busy, setBusy] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [selectedMode, setSelectedMode] = useState<'codex' | 'official'>('codex');
  const [managementAction, setManagementAction] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ key: string; warning?: boolean } | null>(null);
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const confirmationRef = useRef(confirmation); confirmationRef.current = confirmation;
  const managementFlights = useRef(new Map<string, Promise<unknown>>());
  const [mode, setMode] = useState<'codex' | 'official'>('official');
  const busyRef = useRef(false);
  const epoch = useRef(0);
  const validationReads = useRef(0);
  const [retry, setRetry] = useState(0);
  const error = useModalErrorState();
  const dialog = useRef<HTMLDivElement>(null);
  const close = useCallback(() => {
    epoch.current++;
    const previous = requestRef.current;
    requestRef.current = null; selectionRef.current = null;
    if (previous?.draftRequestId) cancelCodexServiceDraftAuthorizations(previous.draftRequestId);
    void Promise.resolve().then(() => resumeDeferredRef.current());
    setRequest(null); setSnapshot(null); setSelection(null); setDone(new Set()); setBusy(false); setMode('official'); setConfirmation(null); setNotice(null); setManagementAction(null); setExpandedId(null);
  }, []);
  const visible = Boolean(request && !selection && !expandedId);
  const dismissGuide = useCallback(() => {
    if (confirmationRef.current) { setConfirmation(null); error.clear(); return; }
    close();
  }, [close, error.clear]);
  useModalFocusTrap(dialog, visible);
  useModalScrollLock(visible);
  useEscCloseTopmost(visible, dismissGuide, dialog);
  useEffect(() => () => {
    epoch.current++;
    const ids = new Set([requestRef.current?.draftRequestId, ...deferredRequests.current.map((item) => item.draftRequestId)]);
    requestRef.current = null;
    for (const id of ids) if (id) cancelCodexServiceDraftAuthorizations(id);
  }, []);
  const markDone = (id: string) => { doneRef.current = new Set(doneRef.current).add(id); setDone(doneRef.current); };
  const resumeDeferred = () => {
    if (busyRef.current || selectionRef.current || requestRef.current) return;
    if (deferredManage.current) { deferredManage.current = false; requestCodexOfficialAuthorizationManagement(); return; }
    if (!deferredRequests.current.length) return;
    const items = deferredRequests.current; deferredRequests.current = [];
    requeueCodexServiceAuthorizationRequests(items);
  };

  resumeDeferredRef.current = resumeDeferred;

  useEffect(() => {
    const manage = () => {
      if (!takeCodexOfficialAuthorizationManagement()) return;
      if (busyRef.current || selectionRef.current || requestRef.current?.kind === 'draft') { deferredManage.current = true; return; }
      epoch.current++;
      const next: Request = { kind: 'manage', items: [] };
      requestRef.current = next;
      setRequest(next); setSelection(null); setSnapshot(null); setDone(new Set()); setMode('official'); setConfirmation(null); setNotice(null); error.clear();
    };
    const receive = () => {
      const items = takeCodexServiceAuthorizationRequests().filter((item) => !item.draftRequestId || isCodexServiceDraftRequestActive(item.draftRequestId, item.accountId));
      if (!items.length) return;
      const current = requestRef.current;
      if (busyRef.current || selectionRef.current || (current && (current.kind === 'draft' || items[0].draftRequestId) && current.draftRequestId !== items[0].draftRequestId)) {
        deferredRequests.current.push(...items); return;
      }
      const draftRequestId = items[0].draftRequestId;
      const incoming = items.filter((item) => item.draftRequestId === draftRequestId);
      deferredRequests.current.push(...items.filter((item) => item.draftRequestId !== draftRequestId));
      epoch.current++;
      const previous = current?.kind === (draftRequestId ? 'draft' : 'join') && current.draftRequestId === draftRequestId ? current.items.filter((item) => !doneRef.current.has(item.accountId)) : [];
      const combined = new Map([...previous, ...incoming].map((item) => [item.accountId, item]));
      const next: Request = { kind: draftRequestId ? 'draft' : 'join', items: [...combined.values()], draftRequestId };
      requestRef.current = next; doneRef.current = new Set();
      setRequest(next); setSelection(null); setSnapshot(null); setDone(new Set()); setMode(incoming[0]?.initialMode ?? 'official'); setConfirmation(null); setNotice(null); error.clear();
    };
    const cancelDraft = (event: Event) => {
      const requestId = (event as CustomEvent<{ requestId: string }>).detail?.requestId;
      if (!requestId) return;
      deferredRequests.current = deferredRequests.current.filter((item) => item.draftRequestId !== requestId);
      if (requestRef.current?.draftRequestId === requestId) close();
    };

    manage(); receive();
    window.addEventListener(CODEX_OFFICIAL_AUTHORIZATION_MANAGE, manage);
    window.addEventListener(CODEX_ACCOUNT_SERVICE_REQUEST, receive);
    window.addEventListener(CODEX_ACCOUNT_SERVICE_DRAFT_CANCEL, cancelDraft);
    return () => {
      window.removeEventListener(CODEX_OFFICIAL_AUTHORIZATION_MANAGE, manage);
      window.removeEventListener(CODEX_ACCOUNT_SERVICE_REQUEST, receive);
      window.removeEventListener(CODEX_ACCOUNT_SERVICE_DRAFT_CANCEL, cancelDraft);
    };
  }, [activePage]);

  const applyLocal = async (sourceId: string) => {
    const state = await setChatGptPlanServiceMode(sourceId, 'codex', undefined, { intent: requestRef.current?.kind === 'join' ? 'join' : 'switch' });
    const related = [sourceId];
    announceCodexServiceAuthorizationApplied(related, (state.collection?.accountIds ?? []).filter((id) => related.includes(id)));
    changed();
    if (!state.collection?.accountIds.includes(sourceId)) throw new Error('CHATGPT_PLAN_SERVICE_CHANGED');
    return state;
  };

  useEffect(() => {
    if (!request) return;
    const revision = ++epoch.current;
    setSnapshot(null); error.clear();
    void (async () => {
      const next = await readSnapshot();
      if (revision !== epoch.current) return;
      const ids = new Set(request.items.map((item) => item.accountId));
      const rows = codexOfficialAuthorizationRows(next.accounts, next.grants, next.members, next.paused, next.serviceMembers).filter((row) => ids.has(row.account.id));
      if ((request.kind === 'join' || request.kind === 'draft') && rows.length !== ids.size) throw new Error('CHATGPT_PLAN_NOT_ELIGIBLE');
      setSnapshot(next);
      if (request.kind === 'draft' && rows[0]) {
        const nextSelection: Selection = { source: rows[0].account, after: 'draftOfficial', draftRequestId: request.draftRequestId,
          reauthorize: Boolean(rows[0].grant?.needs_reauth || rows[0].grant?.source_binding_error) };
        selectionRef.current = nextSelection; setSelection(nextSelection);
      }
    })().catch((failure) => { if (revision === epoch.current) error.report(t(chatGptPlanErrorKey(failure))); });
  }, [request, retry]);

  useEffect(() => {
    const pending = snapshot?.grants.some((grant) => grant.source_account_id && grant.source_binding_valid !== true && !grant.source_binding_error);
    if (!request || !pending) { validationReads.current = 0; return; }
    if (selection || busy) return;
    if (validationReads.current >= 4) {
      if (validationReads.current++ === 4) error.report(t('chatgptPlan.error.timeout'));
      return;
    }
    let cancelled = false;
    const revision = epoch.current;
    const timer = setTimeout(() => {
      validationReads.current++;
      void readSnapshot().then((next) => {
        if (!cancelled && revision === epoch.current) setSnapshot(next);
      }).catch((failure) => {
        if (!cancelled && revision === epoch.current) error.report(t(chatGptPlanErrorKey(failure)));
      });
    }, 500 * 2 ** validationReads.current);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [request, snapshot, selection, busy, t]);

  const rows = snapshot ? codexOfficialAuthorizationRows(snapshot.accounts, snapshot.grants, snapshot.members, snapshot.paused, snapshot.serviceMembers)
    .filter((row) => request?.kind === 'join' || request?.kind === 'draft'
      ? request.items.some((item) => item.accountId === row.account.id)
      : request?.kind === 'manage') : [];
  const completeDraft = (sourceAccountId: string, nextMode: 'codex' | 'official', registrationId?: string, draftRequestId = requestRef.current?.draftRequestId) => {
    if (!draftRequestId || !isCodexServiceDraftRequestActive(draftRequestId, sourceAccountId)) return;
    if (nextMode === 'official' && !registrationId) return;
    const result = nextMode === 'official'
      ? { requestId: draftRequestId, sourceAccountId, status: 'selected' as const, mode: 'official' as const, registrationId: registrationId! }
      : { requestId: draftRequestId, sourceAccountId, status: 'selected' as const, mode: 'codex' as const };
    if (!announceCodexServiceDraftResult(result)) return;
    markDone(sourceAccountId);
    if (requestRef.current?.draftRequestId === draftRequestId && requestRef.current.items.every((item) => doneRef.current.has(item.accountId))) close();
  };
  const runLocal = async (ids: string[]) => {
    if (busyRef.current) return;
    if (requestRef.current?.kind === 'draft') { ids.forEach((id) => completeDraft(id, 'codex')); return; }
    busyRef.current = true; setBusy(true); error.clear(); setNotice(null);
    const revision = epoch.current;
    try {
      for (const id of ids) {
        if (revision !== epoch.current) return;
        const state = await codexOfficialAuthorizationDeadline(applyLocal(id), 90_000);
        const members = state.collection?.accountIds ?? [];
        if (revision !== epoch.current) return;
        setSnapshot((previous) => previous ? { ...previous, members, paused: state.collection?.pausedAccountIds ?? [], serviceMembers: state.serviceMembers } : previous);
        markDone(id);
      }
      if (revision === epoch.current && rows.every((row) => doneRef.current.has(row.account.id))) close();
    } catch (failure) { if (revision === epoch.current) error.report(t(chatGptPlanErrorKey(failure))); }
    finally { busyRef.current = false; if (revision === epoch.current) setBusy(false); resumeDeferred(); }
  };
  const openAuthorization = (row: CodexOfficialAuthorizationRow, reauthorize = false, after: AfterAuthorization = 'none') => {
    if (busyRef.current) return;
    error.clear(); setNotice(null);
    const next: Selection = { source: row.account, after, reauthorize, draftRequestId: requestRef.current?.draftRequestId };
    selectionRef.current = next; setSelection(next);
  };
  const expand = (row: CodexOfficialAuthorizationRow) => {
    if (busyRef.current) return;
    error.clear(); setNotice(null); setConfirmation(null);
    setExpandedId((previous) => previous === row.account.id ? null : row.account.id);
    setSelectedMode(row.official ? 'official' : 'codex');
  };
  const applyMode = async (row: CodexOfficialAuthorizationRow, nextMode: 'codex' | 'official', intent: 'join' | 'switch') => {
    if (busyRef.current) return;
    if (intent === 'switch' && !row.joined) { error.report(t('chatgptPlan.error.serviceChanged')); return; }
    if (nextMode === 'official' && (!row.grant || !row.grant.connected || row.grant.needs_reauth || row.grant.source_binding_error)) {
      openAuthorization(row, Boolean(row.grant?.needs_reauth || row.grant?.source_binding_error), intent === 'join' ? 'joinOfficial' : 'switchOfficial'); return;
    }
    busyRef.current = true; setBusy(true); error.clear(); setNotice(null);
    const revision = epoch.current;
    try {
      const related = [row.account.id];
      const state = await codexOfficialAuthorizationDeadline(setChatGptPlanServiceMode(row.account.id, nextMode, row.grant?.id, { intent }).then((value) => {
        announceCodexServiceAuthorizationApplied(related, (value.collection?.accountIds ?? []).filter((id) => related.includes(id)));
        changed(); return value;
      }), 90_000);
      if (revision !== epoch.current) return;
      const members = state.collection?.accountIds ?? [];
      if (!isChatGptPlanServiceModeApplied(state, row.account.id, nextMode, row.grant?.id)) throw new Error('CHATGPT_PLAN_SERVICE_CHANGED');
      setSnapshot((previous) => previous ? { ...previous, members, paused: state.collection?.pausedAccountIds ?? [], serviceMembers: state.serviceMembers, grants: previous.grants.map((grant) =>
        grant.id === row.grant?.id ? { ...grant, in_service: nextMode === 'official', last_service_mode: nextMode } : grant) } : previous);
      if (request?.kind !== 'manage') markDone(row.account.id);
      if (request?.kind === 'join' && rows.every((item) => doneRef.current.has(item.account.id))) close();
    } catch (failure) {
      if (revision === epoch.current) {
        const key = chatGptPlanErrorKey(failure);
        if (key === 'chatgptPlan.error.reauthorize' || key === 'chatgptPlan.error.permission') {
          setSnapshot((previous) => previous ? { ...previous, grants: previous.grants.map((grant) =>
            grant.id === row.grant?.id ? { ...grant, needs_reauth: true } : grant) } : previous);
        }
        error.report(t(key));
      }
    }
    finally { busyRef.current = false; if (revision === epoch.current) setBusy(false); resumeDeferred(); }
  };
  const runOfficial = (row: CodexOfficialAuthorizationRow) => {
    if (requestRef.current?.kind === 'draft') {
      if (row.grant?.connected && !row.grant.needs_reauth && row.grant.source_binding_valid === true && !row.grant.source_binding_error) completeDraft(row.account.id, 'official', row.grant.id);
      else openAuthorization(row, Boolean(row.grant?.needs_reauth || row.grant?.source_binding_error), 'draftOfficial');
      return;
    }
    if (request?.kind === 'join') return applyMode(row, 'official', 'join');
    openAuthorization(row, Boolean(row.grant?.needs_reauth || row.grant?.source_binding_error));
  };

  const manageAuthorization = async (row: CodexOfficialAuthorizationRow, action: 'check' | 'release', disposition?: ChatGptPlanReleaseDisposition) => {
    const grant = row.grant;
    if (!grant || busyRef.current) return;
    busyRef.current = true; setBusy(true); error.clear(); setNotice(null); setManagementAction(action);
    const revision = epoch.current;
    const key = `${action}:${grant.id}:${disposition ?? ''}`;
    let operation = managementFlights.current.get(key);
    // A timed-out release cannot be replaced with another disposition while its
    // original transaction is still in flight.
    if (!operation && [...managementFlights.current.keys()].some((item) => item.startsWith(`release:${grant.id}:`))) {
      busyRef.current = false; setBusy(false); setManagementAction(null); error.report(t('chatgptPlan.error.busy')); return;
    }
    if (!operation) {
      operation = (action === 'check' ? checkChatGptPlanAuthorization(grant.id)
        : releaseChatGptPlanAuthorization(grant.id, disposition ?? 'pause')).then((value) => { changed(); return value; });
      managementFlights.current.set(key, operation);
      const release = () => { if (managementFlights.current.get(key) === operation) managementFlights.current.delete(key); };
      void operation.then(release, release);
    }
    try {
      const result = await codexOfficialAuthorizationDeadline(operation, 45_000);
      if (revision !== epoch.current) return;
      if (action === 'check') {
        const checked = result as ChatGptPlanAccount;
        setSnapshot((previous) => previous ? { ...previous, grants: previous.grants.map((item) => item.id === grant.id
          ? { ...checked, in_service: previous.serviceMembers?.some((member) => member.sourceAccountId === row.account.id && member.mode === 'official') ?? Boolean(checked.in_service && previous.members.includes(row.account.id)) } : item) } : previous);
        const failure = checked.check_error || ((!checked.connected || checked.needs_reauth) ? 'CHATGPT_PLAN_REAUTH_REQUIRED'
          : checked.source_binding_error || (checked.source_binding_valid !== true ? 'CHATGPT_PLAN_BINDING_REQUIRED' : null));
        if (failure) error.report(t('chatgptPlan.officialFlow.checkFailed', { error: t(chatGptPlanErrorKey(failure)) }));
        else setNotice({ key: 'chatgptPlan.officialFlow.checkPassed' });
      } else {
        const released = result as ChatGptPlanReleaseResult;
        const members = released.state.collection?.accountIds ?? [];
        const related = [row.account.id];
        announceCodexServiceAuthorizationApplied(related, members.filter((id) => related.includes(id)));
        setSnapshot((previous) => previous ? { ...previous, members, paused: released.state.collection?.pausedAccountIds ?? [], serviceMembers: released.state.serviceMembers,
          grants: previous.grants.map((item) => item.id === grant.id ? { ...released.authorization, in_service: Boolean(released.authorization.in_service && members.includes(row.account.id)) } : item) } : previous);
        setConfirmation(null);
        setNotice({ key: released.remote_revoked ? 'chatgptPlan.disconnectedNotice' : 'chatgptPlan.localDisconnectedNotice', warning: !released.remote_revoked });
        setSelectedMode('codex');
      }
    } catch (failure) { if (revision === epoch.current) error.report(t(chatGptPlanErrorKey(failure))); }
    finally { busyRef.current = false; setBusy(false); if (revision === epoch.current) setManagementAction(null); resumeDeferred(); }
  };
  const completed = async (grant: ChatGptPlanAccount) => {
    const current = selectionRef.current;
    if (!current) return;
    // A metadata refresh triggered during OAuth must not overwrite the newly
    // bound authorization or its explicit follow-up transaction.
    epoch.current++;
    selectionRef.current = null; setSelection(null);
    const allGrants = [...(snapshot?.grants ?? []).filter((item) => item.id !== grant.id), grant];
    setSnapshot((previous) => previous ? { ...previous, grants: allGrants } : previous);
    changed();
    if (current.after === 'draftOfficial') {
      completeDraft(current.source.id, 'official', grant.id, current.draftRequestId);
    } else if (current.after !== 'none') {
      const row = codexOfficialAuthorizationRows([current.source], allGrants, snapshot?.members ?? [], snapshot?.paused ?? [], snapshot?.serviceMembers)[0];
      if (row) await applyMode(row, 'official', current.after === 'joinOfficial' ? 'join' : 'switch');
    } else {
      setExpandedId(current.source.id);
      setSelectedMode(snapshot?.serviceMembers?.some((member) => member.sourceAccountId === current.source.id && member.mode === 'official') ? 'official' : 'codex');
    }
    resumeDeferred();
  };

  const refreshGuide = async () => {
    changed();
    const revision = epoch.current;
    const next = await readSnapshot();
    if (revision === epoch.current && requestRef.current) setSnapshot(next);
  };
  const returnToGuide = () => {
    if (requestRef.current?.kind === 'draft') { close(); return; }
    selectionRef.current = null; setSelection(null);
    const revision = epoch.current;
    void refreshGuide().catch((failure) => { if (revision === epoch.current && requestRef.current) error.report(t(chatGptPlanErrorKey(failure))); });
    resumeDeferred();
  };
  const selectMode = (value: 'codex' | 'official') => {
    if (busyRef.current) return;
    error.clear(); setMode(value);
  };
  const navigateTabs = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key) || busyRef.current) return;
    event.preventDefault();
    const next = event.key === 'Home' ? 'codex' : event.key === 'End' ? 'official' : mode === 'codex' ? 'official' : 'codex';
    selectMode(next);
    event.currentTarget.parentElement?.querySelector<HTMLButtonElement>(`#codex-service-tab-${next}`)?.focus();
  };

  if (!request) return null;
  if (selection && snapshot) return <CodexOfficialAuthorizationDialog key={`${selection.source.id}:${selection.after}`}
    sourceAccount={selection.source} registrations={snapshot.grants} reauthorize={selection.reauthorize}
    completionMode={selection.after === 'draftOfficial' ? 'draft' : selection.after === 'none' ? 'bind' : 'service'}
    onChanged={refreshGuide} onClose={returnToGuide} onComplete={(grant) => { void completed(grant); }} />;
  const managedRow = rows.find((row) => row.account.id === expandedId);
  if (managedRow) return <CodexOfficialManagementDialog row={managedRow} busy={busy} selectedMode={selectedMode} confirmation={confirmation?.disposition ?? null}
    onClose={close}
    onMode={(value) => { setSelectedMode(value); error.clear(); }} onApply={() => void applyMode(managedRow, selectedMode, 'switch')}
    onAuthorize={() => openAuthorization(managedRow, true)} onCheck={() => void manageAuthorization(managedRow, 'check')}
    onRelease={() => { if (confirmation) void manageAuthorization(managedRow, 'release', confirmation.disposition); }}
    onConfirmation={(disposition) => { error.clear(); setNotice(null); setConfirmation(disposition ? { row: managedRow, disposition } : null); }}>
    <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />
    {notice && <div className={`codex-official-notice${notice.warning ? ' is-warning' : ''}`} role="status">{t(notice.key)}</div>}
  </CodexOfficialManagementDialog>;
  const choosing = request.kind === 'join' || request.kind === 'draft';
  const pendingIds = rows.filter((row) => !done.has(row.account.id)).map((row) => row.account.id);
  return createPortal(<div className="modal-overlay codex-official-guide-overlay">
    <div ref={dialog} className={`modal codex-official-guide${request.kind === 'manage' ? ' is-management' : ''}`} role="dialog" aria-modal="true" aria-labelledby="codex-official-guide-title" tabIndex={-1}>
      <div className="modal-header"><div><h2 id="codex-official-guide-title">{t(request.kind === 'manage' ? 'chatgptPlan.officialFlow.manageTitle' : 'chatgptPlan.officialFlow.guideTitle')}</h2><p>{t(request.kind === 'manage' ? 'chatgptPlan.officialFlow.manageHint' : 'chatgptPlan.officialFlow.beforeJoin')}</p></div>
      </div>
      {choosing && <div className="codex-official-mode-tabs" role="tablist" aria-label={t('chatgptPlan.serviceMode.title')}>
        {(['codex', 'official'] as const).map((value) => <button key={value} type="button" role="tab"
          id={`codex-service-tab-${value}`} aria-controls="codex-service-mode-panel" aria-selected={mode === value} tabIndex={mode === value ? 0 : -1}
          className={`btn btn-secondary${mode === value ? ' is-active' : ''}`} disabled={busy}
          onClick={() => selectMode(value)} onKeyDown={navigateTabs}>
          {value === 'official' ? <ShieldCheck size={16} /> : <Network size={16} />}
          {t(value === 'official' ? 'chatgptPlan.officialFlow.officialTab' : 'chatgptPlan.officialFlow.localTab')}
        </button>)}
      </div>}
      <div className="modal-body" id="codex-service-mode-panel" role={choosing ? 'tabpanel' : undefined} aria-labelledby={choosing ? `codex-service-tab-${mode}` : 'codex-official-guide-title'}>
        <div className="codex-official-benefit">{mode === 'official' ? <ShieldCheck size={22} /> : <Network size={22} />}<div>
          <strong>{t(mode === 'official' ? 'chatgptPlan.officialFlow.benefitTitle' : 'chatgptPlan.officialFlow.localBenefitTitle')}</strong>
          <p>{t(mode === 'official' ? 'chatgptPlan.officialFlow.benefit' : 'chatgptPlan.officialFlow.localBenefit')}</p>
          <small>{t(mode === 'official' ? 'chatgptPlan.officialFlow.limits' : 'chatgptPlan.officialFlow.localLimits')}</small>
          {mode === 'official' && <small>{t('chatgptPlan.officialFlow.sourceDataHint')}</small>}
        </div></div>
        {request.kind === 'draft' && <p className="codex-official-management-hint">{t('chatgptPlan.officialFlow.draftSelectionHint')}</p>}
        <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />
        {notice && <div className={`codex-official-notice${notice.warning ? ' is-warning' : ''}`} role="status">{t(notice.key)}</div>}
        {snapshot && <p className="codex-official-source-summary">{t('chatgptPlan.officialFlow.originalAccountHint')}<strong>{t('chatgptPlan.officialFlow.sourceAccountsSummary', {
          count: codexOfficialAuthorizationRows(snapshot.accounts, snapshot.grants, snapshot.members, snapshot.paused, snapshot.serviceMembers).length,
          members: codexOfficialAuthorizationRows(snapshot.accounts, snapshot.grants, snapshot.members, snapshot.paused, snapshot.serviceMembers).filter((row) => row.joined).length,
        })}</strong></p>}
        {!snapshot ? <p role="status" className="codex-official-loading"><Loader2 size={18} className="codex-official-spin" />{t('common.loading')}</p> : <ul className="codex-official-rows">
          {rows.map((row) => {
            const state = codexOfficialAuthorizationState(row.grant, row.official && !row.paused);
            const disabled = busy || Boolean(confirmation);
            return <li key={row.account.id} className="codex-official-row">
              <div className="codex-official-row-heading">
                <div className="codex-official-account"><strong title={row.account.email}>{row.account.account_name || row.account.email || row.account.id}</strong>
                  {row.account.account_name && row.account.email && <small>{row.account.email}</small>}
                  <div><span className={row.joined ? 'codex-official-member' : ''}>{t(!row.joined ? 'chatgptPlan.officialFlow.notInService' : row.paused ? 'codex.localAccess.accountPoolHealth.paused'
                    : row.official ? 'chatgptPlan.officialFlow.officialTab' : 'chatgptPlan.officialFlow.localTab')}</span>
                    {mode === 'official' && <>{row.grant && state !== 'disconnected' && <CodexOfficialBadge active={state === 'used'} state={state} />}
                      <span className="codex-official-authorization-state" data-state={state}>{t(codexOfficialAuthorizationStateKey(state))}</span></>}
                  </div>
                </div>
                <div className="codex-official-row-actions">
                  {choosing ? done.has(row.account.id) ? <span className="codex-official-complete"><CheckCircle2 size={16} />{t('chatgptPlan.officialFlow.completed')}</span>
                    : mode === 'official' ? <button type="button" className="btn btn-primary btn-sm" disabled={disabled} onClick={() => void runOfficial(row)}>{t(row.authorized ? 'chatgptPlan.officialFlow.useOfficial' : request.kind === 'draft' ? 'chatgptPlan.officialFlow.authorize' : 'chatgptPlan.officialFlow.authorizeJoin')}</button>
                      : <button type="button" className="btn btn-primary btn-sm" disabled={disabled} onClick={() => void runLocal([row.account.id])}>{t('chatgptPlan.officialFlow.useLocal')}</button>
                    : row.grant || row.paused ? <button type="button" className="btn btn-secondary btn-sm" disabled={disabled} onClick={() => expand(row)}>{t('chatgptPlan.officialFlow.manageAccount')}</button>
                      : <button type="button" className="btn btn-primary btn-sm" disabled={disabled} onClick={() => openAuthorization(row)}>{t('chatgptPlan.officialFlow.authorize')}</button>}
                </div>
              </div>
            </li>;
          })}
        </ul>}
        {snapshot && rows.length === 0 && request.kind === 'manage' && <p className="codex-official-loading">{t('chatgptPlan.officialFlow.noAccounts')}</p>}
        {busy && <p role="status" className="codex-official-loading"><Loader2 size={16} className="codex-official-spin" />{t(managementAction === 'check' ? 'chatgptPlan.officialFlow.checkingAuthorization' : 'chatgptPlan.officialFlow.applying')}</p>}
      </div>
      <div className="modal-footer codex-official-guide-footer">
        <div className="codex-official-guide-footer-actions">
        <button type="button" className="btn btn-secondary" onClick={dismissGuide}>{t(request.kind === 'manage' ? 'common.close' : 'common.cancel')}</button>
        {error.message && !snapshot && <button type="button" className="btn btn-primary" onClick={() => { error.clear(); setRetry((value) => value + 1); }}>{t('common.refresh')}</button>}
        {choosing && mode === 'codex' && rows.length > 1 && pendingIds.length > 0 && <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void runLocal(pendingIds)}>{t('chatgptPlan.officialFlow.continueLocal')}</button>}
        </div>
      </div>
    </div>
  </div>, document.body);
}
