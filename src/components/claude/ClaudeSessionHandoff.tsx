import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { ArrowRight, ArrowRightLeft, CircleCheck, RefreshCw, RotateCcw, X } from 'lucide-react';
import { confirm as nativeConfirmDialog } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { getClaudeAccountDisplayEmail, isClaudeDesktopOAuthAccount, type ClaudeAccount } from '../../types/claude';
import {
  canPreviewClaudeHandoff,
  canApplyClaudeHandoffPreview,
  canRollbackClaudeHandoff,
  getClaudeHandoffErrorCode,
  getClaudeHandoffRunAction,
  type ClaudeHandoffIdentity,
  type ClaudeHandoffMutationResult,
  type ClaudeHandoffPreview,
  type ClaudeHandoffRunSummary,
  type ClaudeHandoffStatus,
} from '../../types/claudeHandoff';
import * as handoffService from '../../services/claudeHandoffService';
import { isPrivacyModeEnabledByDefault, maskSensitiveValue } from '../../utils/privacy';
import './ClaudeSessionHandoff.css';

export interface ClaudeSessionHandoffProps {
  accounts?: ClaudeAccount[];
  currentAccountId?: string | null;
  switchRequest?: { targetAccountId: string; sequence: number } | null;
  onSwitchAccount?: (accountId: string) => Promise<boolean>;
  onSwitchCompleted?: (accountId: string) => void;
  privacyModeEnabled?: boolean;
  disabled?: boolean;
  onBusyChange?: (busy: boolean) => void;
}

export function ClaudeSessionHandoff({
  accounts,
  currentAccountId = null,
  switchRequest = null,
  onSwitchAccount,
  onSwitchCompleted,
  privacyModeEnabled = isPrivacyModeEnabledByDefault(),
  disabled = false,
  onBusyChange,
}: ClaudeSessionHandoffProps) {
  const { t, i18n } = useTranslation();
  const titleId = useId();
  const descriptionId = useId();
  const successTitleId = useId();
  const [open, setOpen] = useState(false);
  const [switchMode, setSwitchMode] = useState(false);
  const [switchCompleted, setSwitchCompleted] = useState(false);
  const [status, setStatus] = useState<ClaudeHandoffStatus | null>(null);
  const [statusLoading, setStatusLoading] = useState(false);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [sourceAccountId, setSourceAccountId] = useState('');
  const [targetAccountId, setTargetAccountId] = useState('');
  const [preview, setPreview] = useState<ClaudeHandoffPreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [autoPreviewPaused, setAutoPreviewPaused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<handoffService.ClaudeHandoffProgress | null>(null);
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  const [result, setResult] = useState<ClaudeHandoffMutationResult | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const successHeadingRef = useRef<HTMLHeadingElement>(null);
  const mountedRef = useRef(true);
  const openRef = useRef(false);
  const busyRef = useRef(false);
  const previewGeneration = useRef(0);
  const statusGeneration = useRef(0);
  const busyCallback = useRef(onBusyChange);
  busyCallback.current = onBusyChange;

  useEffect(() => {
    if (!open) return;
    let active = true;
    let stop: (() => void) | undefined;
    void handoffService.watchClaudeHandoffProgress(next => {
      if (active && busyRef.current) setProgress(next);
    }).then(unlisten => { if (active) stop = unlisten; else unlisten(); }).catch(() => {});
    return () => { active = false; stop?.(); };
  }, [open]);

  useEffect(() => {
    if (!busy) return;
    const started = Date.now();
    setElapsedSeconds(0);
    const timer = window.setInterval(() => setElapsedSeconds(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => window.clearInterval(timer);
  }, [busy]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      openRef.current = false;
      previewGeneration.current += 1;
      statusGeneration.current += 1;
      // An in-flight write still owns the busy lock until its promise settles.
    };
  }, []);

  const explainError = (value: unknown, fallback = 'UNKNOWN') => {
    const code = getClaudeHandoffErrorCode(value);
    return t(`claude.handoff.errors.${code ?? fallback}`, {
      defaultValue: t(`claude.handoff.errors.${fallback}`),
    });
  };

  const loadStatus = async (defaultTarget?: string | null, switchTarget?: string) => {
    const request = ++statusGeneration.current;
    setStatusLoading(true);
    setStatusError(null);
    try {
      const next = await handoffService.getClaudeHandoffStatus();
      if (!mountedRef.current || !openRef.current || request !== statusGeneration.current) return;
      setStatus(next);
      const confirmedSource = next.currentIdentity && accounts
        ? accounts.find(account => isClaudeDesktopOAuthAccount(account)
          && account.account_uuid === next.currentIdentity?.account
          && account.organization_uuid === next.currentIdentity?.org)?.id
        : next.currentAccountId;
      if (switchTarget && confirmedSource && confirmedSource !== switchTarget
        && next.accounts.some(account => account.id === confirmedSource && account.eligible)
        && (!accounts || accounts.some(account => account.id === confirmedSource && isClaudeDesktopOAuthAccount(account)))) {
        setSourceAccountId(confirmedSource);
      }
      // A later successful refresh supersedes an earlier receipt for that run.
      // A failed refresh never erases an already confirmed mutation outcome.
      setResult((previous) => {
        if (!previous) return previous;
        const latest = next.runs.find((run) => run.id === previous.run.id);
        if (!latest) return previous;
        return latest.state === previous.run.state ? { ...previous, run: latest } : {
          run: latest,
          reopened: false,
          warning: latest.state === 'rolled_back' ? 'RECOVERED_APP_CLOSED' : null,
        };
      });
      if (defaultTarget !== undefined) {
        setTargetAccountId(!next.usesSavedAccountIndex && next.accounts.some((account) => account.id === defaultTarget && account.eligible)
          ? defaultTarget ?? '' : '');
      }
    } catch (failure) {
      if (mountedRef.current && openRef.current && request === statusGeneration.current) {
        setStatusError(explainError(failure, 'STATUS_FAILED'));
      }
    } finally {
      if (mountedRef.current && openRef.current && request === statusGeneration.current) setStatusLoading(false);
    }
  };

  const close = useCallback(() => {
    if (busyRef.current) return;
    openRef.current = false;
    previewGeneration.current += 1;
    statusGeneration.current += 1;
    setOpen(false);
  }, []);

  // SingleSelectDropdown portals its menu to body. Include that menu in the
  // focus boundary and keyboard navigation without changing the shared control.
  useEffect(() => {
    if (!open) return;
    const dialog = dialogRef.current;
    const previousFocus = document.activeElement;
    const menuSelector = '.claude-handoff-select-menu';
    const focusableSelector = 'button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), summary, [tabindex]:not([tabindex="-1"])';
    const visible = (element: HTMLElement) => element.getClientRects().length > 0;
    const focusables = () => Array.from(dialog?.querySelectorAll<HTMLElement>(focusableSelector) ?? []).filter(visible);
    const expandedTrigger = () => dialog?.querySelector<HTMLButtonElement>('[aria-haspopup="listbox"][aria-expanded="true"]');
    const dismissMenu = () => {
      const trigger = expandedTrigger();
      trigger?.click();
      return trigger;
    };
    const onKeyDown = (event: KeyboardEvent) => {
      const menu = document.querySelector<HTMLElement>(menuSelector);
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopImmediatePropagation();
        if (busyRef.current) return;
        if (menu) dismissMenu()?.focus();
        else close();
        return;
      }
      if (menu && ['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
        event.preventDefault();
        event.stopImmediatePropagation();
        const items = Array.from(menu.querySelectorAll<HTMLButtonElement>('[role="option"]'));
        const index = items.findIndex((item) => item === document.activeElement);
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1
          : event.key === 'ArrowDown' ? (index + 1) % items.length
            : (index < 0 ? items.length - 1 : (index - 1 + items.length) % items.length);
        items[next]?.focus();
        return;
      }
      if (event.key !== 'Tab') return;
      event.preventDefault();
      event.stopImmediatePropagation();
      const active = menu ? dismissMenu() : document.activeElement;
      const items = focusables();
      const index = items.findIndex((item) => item === active);
      const next = index < 0 ? (event.shiftKey ? items.length - 1 : 0)
        : (index + (event.shiftKey ? -1 : 1) + items.length) % items.length;
      (items[next] ?? dialog)?.focus();
    };
    const onFocusIn = (event: FocusEvent) => {
      const target = event.target;
      if (target instanceof Node && !dialog?.contains(target)
        && !document.querySelector(menuSelector)?.contains(target)) {
        (focusables()[0] ?? dialog)?.focus();
      }
    };
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    dialog?.focus();
    window.addEventListener('keydown', onKeyDown, true);
    document.addEventListener('focusin', onFocusIn);
    return () => {
      window.removeEventListener('keydown', onKeyDown, true);
      document.removeEventListener('focusin', onFocusIn);
      document.body.style.overflow = previousOverflow;
      // WebKit does not always focus buttons on pointer activation.
      if (triggerRef.current?.isConnected) triggerRef.current.focus();
      else if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
    };
  }, [close, open]);

  const openDialog = (switchTarget?: string) => {
    if (disabled || busyRef.current) return;
    const choosingSwitch = Boolean(switchTarget && onSwitchAccount);
    openRef.current = true;
    setSwitchMode(choosingSwitch);
    setSwitchCompleted(false);
    setStatus(null);
    // Cockpit's last-used marker may be stale if Desktop was switched elsewhere.
    setSourceAccountId('');
    setTargetAccountId(choosingSwitch ? switchTarget! : '');
    setPreview(null);
    setPreviewLoading(false);
    setAutoPreviewPaused(false);
    setResult(null);
    setError(null);
    setOpen(true);
    void loadStatus(choosingSwitch ? undefined : currentAccountId, choosingSwitch ? switchTarget : undefined);
  };

  useEffect(() => {
    if (switchRequest) openDialog(switchRequest.targetAccountId);
    // A new sequence is the user's explicit request to open the switch review.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [switchRequest?.sequence]);

  const switchOnly = async () => {
    if (!switchMode || !onSwitchAccount || busyRef.current || !targetAccountId
      || !status || statusError || statusLoading || needsRecovery) return;
    busyRef.current = true;
    setBusy(true);
    setProgress({ stage: 'switching', completed: 0, total: 0 });
    busyCallback.current?.(true);
    setError(null);
    try {
      if (await onSwitchAccount(targetAccountId)) {
        openRef.current = false;
        setOpen(false);
      } else {
        setError(t('claude.handoff.switchFailed'));
      }
    } catch {
      setError(t('claude.handoff.switchFailed'));
    } finally {
      busyRef.current = false;
      if (mountedRef.current) setBusy(false);
      busyCallback.current?.(false);
    }
  };

  const changeAccount = (side: 'source' | 'target', id: string) => {
    if (busyRef.current) return;
    if (id === (side === 'source' ? sourceAccountId : targetAccountId)) return;
    previewGeneration.current += 1;
    setPreview(null);
    setPreviewLoading(false);
    setAutoPreviewPaused(false);
    setError(null);
    if (side === 'source') setSourceAccountId(id);
    else setTargetAccountId(id);
    // An option click unmounts the portal; restore focus to its owning control.
    dialogRef.current?.querySelector<HTMLButtonElement>(`[data-handoff-account="${side}"] button`)?.focus();
  };

  const desktopAccounts = accounts?.filter(isClaudeDesktopOAuthAccount) ?? [];
  const labelForAccount = (id: string) => {
    const account = desktopAccounts.find((item) => item.id === id);
    const saved = status?.accounts.find((item) => item.id === id);
    const label = status?.usesSavedAccountIndex ? saved?.label : account ? getClaudeAccountDisplayEmail(account) : saved?.label;
    return label && label !== id ? maskSensitiveValue(label, privacyModeEnabled) : t('claude.handoff.savedAccount');
  };
  const labelForIdentity = (identity: ClaudeHandoffIdentity) => {
    if (status?.usesSavedAccountIndex) {
      const saved = status.accounts.find((item) => item.identity?.account === identity.account
        && item.identity?.org === identity.org);
      return saved ? labelForAccount(saved.id) : t('claude.handoff.savedAccount');
    }
    const account = desktopAccounts.find((item) => item.account_uuid === identity.account && item.organization_uuid === identity.org);
    return account ? labelForAccount(account.id) : t('claude.handoff.savedAccount');
  };
  const eligibleAccounts = (status?.accounts ?? []).filter((account) => account.eligible
    && (status?.usesSavedAccountIndex || !accounts || desktopAccounts.some((saved) => saved.id === account.id)));
  const options = eligibleAccounts.map((account) => ({ value: account.id, label: labelForAccount(account.id) }));
  const pair = { sourceAccountId, targetAccountId };
  const validPair = !statusError && canPreviewClaudeHandoff(status, pair)
    && eligibleAccounts.some((account) => account.id === sourceAccountId)
    && eligibleAccounts.some((account) => account.id === targetAccountId);
  const pairLabel = `${labelForAccount(sourceAccountId)} → ${labelForAccount(targetAccountId)}`;
  const runs = result
    ? [result.run, ...(status?.runs ?? []).filter((run) => run.id !== result.run.id)]
    : status?.runs ?? [];
  const needsRecovery = runs.some((run) => getClaudeHandoffRunAction(run.state) === 'recover');
  // Progress events and a copied catalog alone cannot confirm the account switch.
  // Keep warning/incomplete receipts in the existing review and recovery flow.
  const completedSuccess = switchCompleted && result?.run.state === 'applied'
    && result.accountSwitched === true && result.reopened && !result.warning
    && !result.run.skippedMissing && !result.run.skippedStale && !result.run.replacedBranches;
  const ReceiptContainer = completedSuccess ? 'details' : 'div';

  useEffect(() => {
    if (!open || !completedSuccess || busy) return;
    // Instant positioning also respects reduced motion and a deeply scrolled preflight.
    bodyRef.current?.scrollTo({ top: 0, behavior: 'instant' });
    successHeadingRef.current?.focus({ preventScroll: true });
  }, [open, completedSuccess, busy]);

  const refreshStatus = () => {
    if (busyRef.current) return;
    previewGeneration.current += 1;
    setPreview(null);
    setPreviewLoading(false);
    setAutoPreviewPaused(false);
    setError(null);
    void loadStatus();
  };

  const createPreview = async () => {
    if (!validPair || busyRef.current || statusLoading || needsRecovery || disabled) return;
    const request = ++previewGeneration.current;
    setPreview(null);
    setError(null);
    setPreviewLoading(true);
    try {
      const next = await handoffService.previewClaudeHandoff(pair);
      if (mountedRef.current && openRef.current && request === previewGeneration.current) setPreview(next);
    } catch (failure) {
      if (mountedRef.current && openRef.current && request === previewGeneration.current) setError(explainError(failure));
    } finally {
      if (mountedRef.current && openRef.current && request === previewGeneration.current) setPreviewLoading(false);
    }
  };

  // A switch has a fixed target. Once the user identifies the actual source,
  // keep its read-only preview current without making them request it twice.
  useEffect(() => {
    if (!open || !switchMode || !validPair || busy || disabled || statusLoading || needsRecovery || result || switchCompleted || autoPreviewPaused) return;
    const request = ++previewGeneration.current;
    setPreview(null);
    setPreviewLoading(true);
    void handoffService.previewClaudeHandoff({ sourceAccountId, targetAccountId })
      .then((next) => {
        if (mountedRef.current && openRef.current && request === previewGeneration.current) setPreview(next);
      })
      .catch((failure) => {
        if (mountedRef.current && openRef.current && request === previewGeneration.current) {
          const code = getClaudeHandoffErrorCode(failure);
          setError(t(`claude.handoff.errors.${code ?? 'UNKNOWN'}`, {
            defaultValue: t('claude.handoff.errors.UNKNOWN'),
          }));
        }
      })
      .finally(() => {
        if (mountedRef.current && openRef.current && request === previewGeneration.current) setPreviewLoading(false);
      });
    return () => {
      if (previewGeneration.current === request) previewGeneration.current += 1;
    };
  }, [open, switchMode, validPair, busy, disabled, statusLoading, needsRecovery, result, switchCompleted, autoPreviewPaused,
    sourceAccountId, targetAccountId, status?.desktopVersion, t]);

  const mutate = async (run?: ClaudeHandoffRunSummary) => {
    if (busyRef.current || disabled || statusLoading) return;
    if (run ? !canRollbackClaudeHandoff(status) : !status?.supported) return;
    const approvedPreview = preview;
    if (!run && (!validPair || needsRecovery || !canApplyClaudeHandoffPreview(approvedPreview))) return;
    // Lock before native confirmation so repeated clicks cannot submit twice.
    busyRef.current = true;
    setBusy(true);
    setProgress({ stage: run ? 'recovering' : 'checking', completed: 0, total: 0 });
    busyCallback.current?.(true);
    setError(null);
    const confirmedPair = run ? `${labelForIdentity(run.source)} → ${labelForIdentity(run.target)}` : pairLabel;
    let attemptedMutation = false;
    try {
      if (run || !switchMode) {
        const confirmed = await nativeConfirmDialog(
          `${confirmedPair}\n\n${t(run ? 'claude.handoff.rollbackConfirm' : 'claude.handoff.applyConfirm')}`,
          {
            title: t(run ? 'claude.handoff.rollbackTitle' : 'claude.handoff.applyTitle'),
            kind: 'warning',
            okLabel: t(run ? 'claude.handoff.rollback' : 'claude.handoff.apply'),
            cancelLabel: t('claude.handoff.cancel'),
          },
        );
        if (!confirmed || !mountedRef.current) return;
      }
      attemptedMutation = true;
      if (switchMode && !run) setAutoPreviewPaused(true);
      setResult(null);
      previewGeneration.current += 1;
      setPreview(null);
      setPreviewLoading(false);
      const outcome = run
        ? await handoffService.rollbackClaudeHandoff(run.id)
        : await (switchMode ? handoffService.applyAndSwitchClaudeHandoff : handoffService.applyClaudeHandoff)({
          ...pair,
          fingerprint: approvedPreview!.fingerprint,
          desktopVersion: approvedPreview!.desktopVersion,
        });
      if (!mountedRef.current) return;
      // Persist the authoritative mutation result before the fallible refresh.
      setResult(outcome);
      if (!run && switchMode) {
        if (outcome.accountSwitched) {
          setSwitchCompleted(true);
          onSwitchCompleted?.(targetAccountId);
        } else if (outcome.warning !== 'ACCOUNT_SWITCH_UNCERTAIN') {
          setError(t('claude.handoff.switchFailedAfterCopy'));
        }
      }
      await loadStatus();
    } catch (failure) {
      if (!mountedRef.current) return;
      setPreview(null);
      setError(explainError(failure));
      // A partially completed operation invalidates the old run inventory.
      // If refresh also fails, require a fresh status instead of showing success.
      if (attemptedMutation) setStatus(null);
      // Partial writes may have left a recovery run even when apply rejected.
      await loadStatus();
    } finally {
      busyRef.current = false;
      if (mountedRef.current) setBusy(false);
      busyCallback.current?.(false);
      if (mountedRef.current) dialogRef.current?.focus();
    }
  };

  const formatDate = (createdAt: number) => {
    const date = new Date(createdAt < 1e12 ? createdAt * 1000 : createdAt);
    return Number.isNaN(date.getTime()) ? t('claude.handoff.unknownDate') : date.toLocaleString(i18n.language);
  };

  return (
    <>
      <button ref={triggerRef} type="button" className="btn btn-secondary icon-only"
        data-testid="claude-handoff-open" aria-label={t('claude.handoff.title')} title={t('claude.handoff.title')}
        disabled={disabled || busy} onClick={() => openDialog()}>
        <ArrowRightLeft size={14} aria-hidden="true" />
      </button>
      {open && createPortal(
        <div className="modal-overlay claude-handoff-overlay" onClick={(event) => { if (event.target === event.currentTarget) close(); }}>
          <div ref={dialogRef} className="modal claude-handoff-modal" role="dialog" aria-modal="true"
            aria-labelledby={completedSuccess ? successTitleId : titleId} aria-describedby={descriptionId} aria-busy={busy} tabIndex={-1} data-testid="claude-handoff-dialog">
            <div className="modal-header">
              <h2 id={titleId}>{t(switchMode ? 'claude.handoff.switchTitle' : 'claude.handoff.title')}</h2>
              <button type="button" className="modal-close" aria-label={t('claude.handoff.close')} disabled={busy} onClick={close}>
                <X size={20} aria-hidden="true" />
              </button>
            </div>
            <div ref={bodyRef} className="modal-body claude-handoff-body">
              <div role="status" aria-live="polite" aria-atomic="true"
                className={completedSuccess ? 'claude-handoff-success' : 'claude-handoff-announcement'}>
                {completedSuccess && <>
                  <div className="claude-handoff-success-heading">
                    <CircleCheck size={36} strokeWidth={2} aria-hidden="true" />
                    <h3 ref={successHeadingRef} id={successTitleId} tabIndex={-1} aria-describedby={descriptionId}
                      data-testid="claude-handoff-success-title">{t('claude.handoff.successTitle')}</h3>
                  </div>
                  <p id={descriptionId} className="claude-handoff-success-account">
                    <span>{t('claude.handoff.successAccount')}</span>
                    <strong>{labelForIdentity(result.run.target)}</strong>
                  </p>
                  <p className="claude-handoff-pair-summary">{labelForIdentity(result.run.source)} → {labelForIdentity(result.run.target)}</p>
                  <p>{t('claude.handoff.resultCounts', { created: result.run.created, updated: result.run.updated })}</p>
                  <p>{t('claude.handoff.switchDone')}</p>
                </>}
              </div>
              {!completedSuccess && <>
              <p id={descriptionId}>{t(switchMode ? 'claude.handoff.switchDescription' : 'claude.handoff.description')}</p>
              {status?.usesSavedAccountIndex && <p className="claude-handoff-notice" data-testid="claude-handoff-saved-index">
                {t('claude.handoff.savedIndexNotice')}
              </p>}
              <details className="claude-handoff-details">
                <summary>{t('claude.handoff.scopeTitle')}</summary>
                <p>{t('claude.handoff.scope')}</p>
                <p>{t(switchMode ? 'claude.handoff.switchStepSignIn' : 'claude.handoff.safety')}</p>
                {status?.desktopVersion && <p>{t('claude.handoff.version', { version: status.desktopVersion })}</p>}
              </details>
              </>}
              <div className="claude-handoff-status-row">
                <span aria-live="polite">{t(statusLoading ? 'claude.handoff.loading'
                  : completedSuccess ? 'claude.handoff.applySuccess' : 'claude.handoff.readOnly')}</span>
                <button type="button" className="btn btn-secondary btn-sm" onClick={refreshStatus} disabled={busy || statusLoading}>
                  <RefreshCw size={14} className={statusLoading ? 'loading-spinner' : ''} aria-hidden="true" />
                  {t('claude.handoff.refresh')}
                </button>
              </div>
              {statusError && <p role="alert" className="claude-handoff-notice">{statusError}</p>}
              {status && (!status.supported || !status.desktopVersion) && (
                <p role="alert" className="claude-handoff-notice">{explainError(status.reason, 'UNSUPPORTED_VERSION')}</p>
              )}
              {!completedSuccess && <>
              {status && eligibleAccounts.length < 2 && <p className="claude-handoff-notice">{t('claude.handoff.needAccounts')}</p>}
              <div className="claude-handoff-pair">
                <div data-handoff-account="source">
                  <span className="claude-handoff-label">{t('claude.handoff.source')}</span>
                  <SingleSelectDropdown value={sourceAccountId} options={options.filter((option) => !switchMode || option.value !== targetAccountId)} onChange={(id) => changeAccount('source', id)}
                      disabled={busy || statusLoading} ariaLabel={t('claude.handoff.source')} placeholder={t('claude.handoff.selectSource')}
                      menuClassName="claude-handoff-select-menu" />
                </div>
                <ArrowRight className="claude-handoff-pair-arrow" size={18} aria-hidden="true" />
                <div data-handoff-account="target">
                  <span className="claude-handoff-label">{t('claude.handoff.target')}</span>
                  {switchMode ? <div className="claude-handoff-account-static">{labelForAccount(targetAccountId)}</div>
                    : <SingleSelectDropdown value={targetAccountId} options={options} onChange={(id) => changeAccount('target', id)}
                      disabled={busy || statusLoading} ariaLabel={t('claude.handoff.target')} placeholder={t('claude.handoff.selectTarget')}
                      menuClassName="claude-handoff-select-menu" />}
                </div>
              </div>
              {!switchMode && <p className="claude-handoff-hint">{t(status?.usesSavedAccountIndex
                ? 'claude.handoff.savedIndexTargetHint' : 'claude.handoff.targetHint')}</p>
              }
              {switchMode && !sourceAccountId && <p className="claude-handoff-notice">{t('claude.handoff.sourceUnknown')}</p>}
              {sourceAccountId && sourceAccountId === targetAccountId && <p role="alert" className="claude-handoff-notice">{t('claude.handoff.errors.SAME_ACCOUNT')}</p>}
              {status?.accounts.some((account) => !account.eligible) && (
                <details className="claude-handoff-details">
                  <summary>{t('claude.handoff.unavailableAccounts')}</summary>
                  <ul>{status.accounts.filter((account) => !account.eligible).map((account) => (
                    <li key={account.id}>{labelForAccount(account.id)}: {explainError(account.reason, 'ACCOUNT_NOT_ELIGIBLE')}</li>
                  ))}</ul>
                </details>
              )}
              </>}
              {needsRecovery && <p role="alert" className="claude-handoff-notice">{t('claude.handoff.errors.RECOVERY_REQUIRED')}</p>}
              {error && <p role="alert" className="claude-handoff-notice" data-testid="claude-handoff-error">{error}</p>}
              {previewLoading && <p role="status">{t('claude.handoff.previewLoading')}</p>}
              {preview && (
                <section className="claude-handoff-preview" data-testid="claude-handoff-preview" aria-label={t('claude.handoff.previewTitle')}>
                  <h3>{t('claude.handoff.previewTitle')}</h3>
                  {(preview.preservedBranches ?? 0) > 0 && <p data-testid="claude-handoff-preserved">
                    {t('claude.handoff.preservedBranches', { count: preview.preservedBranches })}
                  </p>}
                  <p className="claude-handoff-pair-summary">{pairLabel}</p>
                  <dl className="claude-handoff-counts">
                    {(['created', 'updated', 'unchanged'] as const).map((key) => (
                      <div key={key}><dt>{t(`claude.handoff.${key}`)}</dt><dd>{preview[key]}</dd></div>
                    ))}
                  </dl>
                  {preview.missing > 0 && <p role="alert" className="claude-handoff-notice" data-testid="claude-handoff-missing">
                    {t('claude.handoff.missingBeforeApply', { count: preview.missing })}
                  </p>}
                  {preview.missing > 0 && <p role="alert" className="claude-handoff-notice" data-testid="claude-handoff-missing-blocked">
                    {t('claude.handoff.unresolvedMissingBeforeApply')}
                  </p>}
                  {preview.stale > 0 && <p role="alert" className="claude-handoff-notice" data-testid="claude-handoff-stale">
                    {t('claude.handoff.staleBeforeApply', { count: preview.stale })}
                  </p>}
                  {(preview.stale > 0 || preview.replacedBranches > 0) && <p role="alert" className="claude-handoff-notice" data-testid="claude-handoff-replaced">
                    {t('claude.handoff.unresolvedBranchesBeforeApply', { count: preview.stale + preview.replacedBranches })}
                  </p>}
                  {preview.created + preview.updated === 0 && <p>{t(preview.baselineChanged
                    ? 'claude.handoff.rememberSharedState' : 'claude.handoff.noChanges')}</p>}
                  <p>{t('claude.handoff.quotaPauses', { count: preview.quotaPausesCleared })}</p>
                  {(preview.issues.length > 0 || preview.warnings.length > 0) && (
                    <details className="claude-handoff-details">
                      <summary>{t('claude.handoff.attention', { issues: preview.issues.length, warnings: preview.warnings.length })}</summary>
                      <ul>{[...preview.issues, ...preview.warnings].map((issue, index) => (
                        <li key={`${index}-${issue.sessionId}`}>
                          <strong>{!privacyModeEnabled && issue.title
                            ? issue.title : t('claude.handoff.sessionLabel', { number: index + 1 })}</strong>
                          <p>{explainError(issue.reason, 'SESSION_SKIPPED')}</p>
                        </li>
                      ))}</ul>
                    </details>
                  )}
                  <p className="claude-handoff-hint">{t(switchMode ? 'claude.handoff.switchCopyHint' : 'claude.handoff.applyHint')}</p>
                </section>
              )}
              {busy && <p role="status" data-testid="claude-handoff-busy">
                {t(`claude.handoff.progress.${progress?.stage ?? 'checking'}`, { defaultValue: t('claude.handoff.busy') })}
                {progress?.stage === 'writing' && progress.total > 0 && ` (${progress.completed}/${progress.total})`}
                {' · '}{t('claude.handoff.elapsed', { seconds: elapsedSeconds })}
              </p>}
              <ReceiptContainer className={completedSuccess ? 'claude-handoff-details claude-handoff-receipt' : 'claude-handoff-receipts'}>
              {completedSuccess && <summary>{t('claude.handoff.successDetails')}</summary>}
              {result && (
                <section role="status" className="claude-handoff-result" data-testid="claude-handoff-result">
                  <strong>{t(result.run.state === 'rolled_back' ? 'claude.handoff.rollbackSuccess'
                    : result.run.state === 'applied' ? 'claude.handoff.applySuccess' : 'claude.handoff.recoveryNeeded')}</strong>
                  <p>{labelForIdentity(result.run.source)} → {labelForIdentity(result.run.target)}</p>
                  <p>{t('claude.handoff.resultCounts', { created: result.run.created, updated: result.run.updated })}</p>
                  {result.run.state === 'applied' && Boolean(result.run.skippedMissing) &&
                    <p role="alert" className="claude-handoff-notice">{t('claude.handoff.missingAfterApply', { count: result.run.skippedMissing })}</p>}
                  {result.run.state === 'applied' && Boolean(result.run.skippedStale) &&
                    <p role="alert" className="claude-handoff-notice">{t('claude.handoff.staleAfterApply', { count: result.run.skippedStale })}</p>}
                  {result.run.state === 'applied' && Boolean(result.run.replacedBranches) &&
                    <p role="status" className="claude-handoff-notice">{t('claude.handoff.replacedAfterApply', { count: result.run.replacedBranches })}</p>}
                  <p>{t(completedSuccess ? 'claude.handoff.switchDone'
                    : result.reopened ? 'claude.handoff.reopened' : 'claude.handoff.notReopened')}</p>
                  {result.run.state === 'applied' && !switchMode && <p>{t('claude.handoff.viewTarget')}</p>}
                  {result.warning && <p className={result.warning === 'RECOVERED_APP_CLOSED' ? 'claude-handoff-hint' : 'claude-handoff-notice'}>
                    {explainError(result.warning, 'OPERATION_WARNING')}
                  </p>}
                </section>
              )}
              {runs.length > 0 && (
                <section className="claude-handoff-history" aria-label={t('claude.handoff.history')}>
                  <h3>{t('claude.handoff.history')}</h3>
                  {runs.map((run) => {
                    const action = getClaudeHandoffRunAction(run.state);
                    return (
                      <div className="claude-handoff-run" key={run.id} data-testid="claude-handoff-run" data-state={run.state}>
                        <div>
                          <strong>{labelForIdentity(run.source)} → {labelForIdentity(run.target)}</strong>
                          <p>{formatDate(run.createdAt)} · {t(`claude.handoff.states.${run.state}`, { defaultValue: t('claude.handoff.states.pending') })}</p>
                          <p>{t('claude.handoff.resultCounts', { created: run.created, updated: run.updated })}</p>
                          {run.state === 'applied' && Boolean(run.skippedMissing) &&
                            <p className="claude-handoff-notice">{t('claude.handoff.missingAfterApply', { count: run.skippedMissing })}</p>}
                          {run.state === 'applied' && Boolean(run.skippedStale) &&
                            <p className="claude-handoff-notice">{t('claude.handoff.staleAfterApply', { count: run.skippedStale })}</p>}
                          {run.state === 'applied' && Boolean(run.replacedBranches) &&
                            <p className="claude-handoff-notice">{t('claude.handoff.replacedAfterApply', { count: run.replacedBranches })}</p>}
                          {run.lastError && <p role="status">{t(`claude.handoff.errors.${run.lastError}`, {
                            defaultValue: t('claude.handoff.errors.UNKNOWN'),
                          })}</p>}
                        </div>
                        {action && <button type="button" className="btn btn-secondary btn-sm" data-testid={`claude-handoff-${action}`}
                          disabled={busy || disabled || statusLoading || !canRollbackClaudeHandoff(status)} onClick={() => void mutate(run)}>
                          <RotateCcw size={14} aria-hidden="true" />{t(action === 'rollback' ? 'claude.handoff.rollback' : 'claude.handoff.recover')}
                        </button>}
                      </div>
                    );
                  })}
                </section>
              )}
              </ReceiptContainer>
            </div>
            <div className="modal-footer">
              {completedSuccess ? <button type="button" className="btn btn-primary" data-testid="claude-handoff-done"
                onClick={close} disabled={busy}>{t('claude.handoff.successDone')}</button> : <>
              <button type="button" className="btn btn-secondary" onClick={close} disabled={busy}>{t('claude.handoff.close')}</button>
              {switchMode && !completedSuccess && <button type="button" className="btn btn-secondary" data-testid="claude-switch-only"
                disabled={busy || disabled || !targetAccountId || !status || Boolean(statusError)
                  || statusLoading || needsRecovery} onClick={() => void switchOnly()}>
                {t('claude.handoff.switchOnly')}
              </button>}
              {!switchMode && <button type="button" className="btn btn-secondary" data-testid="claude-handoff-preview-button"
                disabled={busy || disabled || statusLoading || previewLoading || !validPair || needsRecovery} onClick={() => void createPreview()}>
                {previewLoading && <RefreshCw size={14} className="loading-spinner" aria-hidden="true" />}{t('claude.handoff.preview')}
              </button>}
              <button type="button" className="btn btn-primary" data-testid="claude-handoff-apply"
                disabled={switchCompleted || busy || disabled || statusLoading || previewLoading || !validPair || needsRecovery
                  || !canApplyClaudeHandoffPreview(preview)} onClick={() => void mutate()}>
                {t(switchMode ? 'claude.handoff.switchAndCopy' : 'claude.handoff.apply')}
              </button>
              </>}
            </div>
          </div>
        </div>, document.body,
      )}
    </>
  );
}
