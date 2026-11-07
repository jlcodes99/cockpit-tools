import { useEffect, useId, useRef, useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import { Settings2, X, LoaderCircle, ChevronRight } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import {
  defaultResetRule, getResetAutomationSnapshot, loadResetAutomation, subscribeResetAutomation,
  saveResetAccountRule, saveResetPublicRule, validateResetRule, prepareResetRuleDraft, resolveResetAutomation,
  type ResetAccountRule, type ResetAutomationState, type ResetRule,
} from '../../services/codexResetAutomationService';
import './CodexResetAutomation.css';

function timeUnit(seconds: number) { return seconds % 86400 === 0 ? 86400 : seconds % 3600 === 0 ? 3600 : 60; }
function ruleDescriptions(rule: ResetRule, t: TFunction) {
  const unit = timeUnit(rule.before_seconds);
  return [
    rule.expiry_enabled ? t('codex.resetAutomation.previewExpiry', {
      value: Number.isFinite(rule.before_seconds) ? rule.before_seconds / unit : '—',
      unit: t(`codex.resetAutomation.${unit === 86400 ? 'days' : unit === 3600 ? 'hours' : 'minutes'}`),
    }) : null,
    rule.quota_enabled ? t('codex.resetAutomation.previewWeekly', { percent: Number.isFinite(rule.remaining_percent) ? rule.remaining_percent : '—' }) : null,
  ].filter((value): value is string => value !== null);
}
function useAutomation() {
  const state = useSyncExternalStore(subscribeResetAutomation, getResetAutomationSnapshot, getResetAutomationSnapshot);
  useEffect(() => { void loadResetAutomation().catch(error => console.warn('[Reset Automation]', error)); }, []);
  return state;
}
function sourceTitle(state: ResetAutomationState | null, accountId: string, t: TFunction) {
  const { mode, rule, enabled } = resolveResetAutomation(state, accountId);
  if (!state) return t('codex.resetAutomation.entryDefault');
  return [t('codex.resetAutomation.title'), t(enabled ? 'common.enabled' : 'common.disabled'),
    ...(rule && mode !== 'off' ? ruleDescriptions(rule, t) : [])].join(' · ');
}

export function CodexResetAutomationStatus({ accountId }: { accountId: string }) {
  const { t } = useTranslation();
  const state = useAutomation();
  if (!state) return null;
  const { enabled } = resolveResetAutomation(state, accountId);
  return <span className={`reset-automation-pill-status ${enabled ? 'is-on' : ''}`} title={sourceTitle(state, accountId, t)}>
    {' · '}{t(`codex.resetAutomation.${enabled ? 'cardOn' : 'cardOff'}`)}
  </span>;
}

export function CodexResetAutomationSection({ accountId, onSettings, disabled = false }: { accountId: string; onSettings: () => void; disabled?: boolean }) {
  const { t } = useTranslation();
  const state = useAutomation();
  const { mode, rule, enabled } = resolveResetAutomation(state, accountId);
  const descriptions = rule && mode !== 'off' ? ruleDescriptions(rule, t) : [];
  return <section className="reset-automation-summary">
    <div className="reset-automation-summary-heading"><strong>{t('codex.resetAutomation.masterLabel')}</strong>
      <button type="button" className="btn btn-secondary reset-automation-settings" onClick={onSettings} disabled={disabled}>
        <Settings2 size={14} />{t('nav.settings')}<ChevronRight size={14} />
      </button>
    </div>
    <span className={`reset-automation-summary-state ${enabled ? 'is-on' : ''}`}>
      {state ? t(enabled ? 'common.enabled' : 'common.disabled') : t('codex.resetAutomation.entryDefault')}
    </span>
    {descriptions.length > 0 && <p className={`reset-automation-summary-rules ${enabled ? '' : 'is-off'}`} title={enabled ? undefined : t('codex.resetAutomation.previewOff')}>{descriptions.join(t('codex.resetAutomation.orSeparator'))}</p>}
  </section>;
}

function RulePreview({ rule, t }: { rule: ResetRule; t: TFunction }) {
  const descriptions = ruleDescriptions(rule, t);
  return <section className={`reset-automation-preview ${rule.enabled ? '' : 'is-off'}`} aria-live="polite">
    <strong>{t('codex.resetAutomation.previewTitle')}</strong>
    <p>{t(!rule.enabled ? 'codex.resetAutomation.previewOff' : descriptions.length ? 'codex.resetAutomation.previewAny' : 'codex.resetAutomation.previewEmpty')}</p>
    {descriptions.length > 0 && <ul>{descriptions.map((description, index) => <li key={index}>{description}</li>)}</ul>}
  </section>;
}

function RuleEditor({ rule, onChange, t, fieldError, validationAttempt }: {
  rule: ResetRule; onChange: (rule: ResetRule) => void; t: TFunction; fieldError: string | null; validationAttempt: number;
}) {
  const [unit, setUnit] = useState(() => timeUnit(rule.before_seconds));
  const timeRef = useRef<HTMLInputElement>(null), percentRef = useRef<HTMLInputElement>(null);
  const id = useId();
  useEffect(() => {
    const node = fieldError === 'invalidTime' ? timeRef.current : fieldError === 'invalidPercent' ? percentRef.current : null;
    node?.focus(); node?.scrollIntoView({ block: 'nearest' });
  }, [fieldError, validationAttempt]);
  const toggle = (key: 'enabled' | 'expiry_enabled' | 'quota_enabled', label: string) => <div className={`reset-automation-setting-row ${key === 'enabled' ? 'is-master' : ''}`}>
    <label htmlFor={`${id}-${key}`}>{label}</label>
    <button type="button" id={`${id}-${key}`} className={`btn reset-automation-switch ${rule[key] ? 'is-on' : ''}`}
      role="switch" aria-label={label} aria-checked={rule[key]} disabled={key !== 'enabled' && !rule.enabled}
      onClick={() => onChange({ ...rule, [key]: !rule[key] })}><span /></button>
  </div>;
  const timeDisabled = !rule.enabled || !rule.expiry_enabled;
  const quotaDisabled = !rule.enabled || !rule.quota_enabled;
  return <div className="reset-automation-fields">
    {toggle('enabled', t('codex.resetAutomation.masterLabel'))}
    <div className="reset-automation-condition">
      {toggle('expiry_enabled', t('codex.resetAutomation.expiry'))}
      <div className="reset-automation-field">
        <label htmlFor={`${id}-time`}>{t('codex.resetAutomation.before')}</label>
        <input ref={timeRef} id={`${id}-time`} className="reset-automation-number" type="number" min="0" step="any" disabled={timeDisabled}
          value={Number.isFinite(rule.before_seconds) ? rule.before_seconds / unit : ''}
          aria-invalid={fieldError === 'invalidTime'} aria-describedby={fieldError === 'invalidTime' ? `${id}-error` : undefined}
          onChange={e => onChange({ ...rule, before_seconds: e.target.value === '' ? NaN : Number(e.target.value) * unit })} />
        <SingleSelectDropdown value={String(unit)} onChange={value => setUnit(Number(value))} disabled={timeDisabled}
          ariaLabel={t('codex.resetAutomation.before')} options={[60, 3600, 86400].map((value, i) => ({
            value: String(value), label: t(`codex.resetAutomation.${['minutes', 'hours', 'days'][i]}`),
          }))} />
      </div>
      {fieldError === 'invalidTime' && <p id={`${id}-error`} className="reset-automation-field-error" role="alert">{t('codex.resetAutomation.invalidTime')}</p>}
    </div>
    <div className="reset-automation-condition">
      {toggle('quota_enabled', t('codex.resetAutomation.weeklyTrigger'))}
      <div className="reset-automation-field is-quota">
        <label htmlFor={`${id}-percent`}>{t('codex.resetAutomation.weeklyRemaining')}</label>
        <input ref={percentRef} id={`${id}-percent`} className="reset-automation-number" type="number" min="0" max="100" step="1" disabled={quotaDisabled}
          value={Number.isFinite(rule.remaining_percent) ? rule.remaining_percent : ''}
          aria-invalid={fieldError === 'invalidPercent'} aria-describedby={fieldError === 'invalidPercent' ? `${id}-error` : undefined}
          onChange={e => onChange({ ...rule, remaining_percent: e.target.value === '' ? NaN : Number(e.target.value) })} /><span>%</span>
      </div>
      {fieldError === 'invalidPercent' && <p id={`${id}-error`} className="reset-automation-field-error" role="alert">{t('codex.resetAutomation.invalidPercent')}</p>}
    </div>
    <RulePreview rule={rule} t={t} />
  </div>;
}

function initialCustom(account: ResetAccountRule | undefined, publicRule: ResetRule): ResetRule {
  return account?.mode === 'custom' ? account.rule : { ...publicRule, enabled: account?.mode === 'off' ? false : publicRule.enabled };
}
export function CodexResetAutomationDialog({ accountId, accountLabel, onClose }: { accountId: string; accountLabel?: string; onClose: () => void }) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<'account' | 'public'>('account');
  const [snapshot, setSnapshot] = useState<ResetAutomationState | null>(getResetAutomationSnapshot);
  const [publicDraft, setPublicDraft] = useState<ResetRule>(snapshot?.public_rule ?? defaultResetRule);
  const initial = snapshot?.accounts[accountId];
  const [applyToAll, setApplyToAll] = useState(false);
  const expectedAccount = useRef<ResetAccountRule>(initial ?? { mode: 'inherit' });
  const expectedPublic = useRef<ResetRule>(snapshot?.public_rule ?? defaultResetRule);
  const expectedAccounts = useRef(snapshot?.accounts ?? {});
  const [customDraft, setCustomDraft] = useState<ResetRule>(() => initialCustom(initial, snapshot?.public_rule ?? defaultResetRule));
  const [loading, setLoading] = useState(!snapshot), [busy, setBusy] = useState(false);
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [validationAttempt, setValidationAttempt] = useState(0);
  const error = useModalErrorState();
  const dialogRef = useRef<HTMLDivElement>(null), alive = useRef(true), submitting = useRef(false);
  const titleId = useId();
  useEscCloseTopmost(true, onClose, dialogRef);
  useModalFocusTrap(dialogRef, true);
  useModalScrollLock(true);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const load = async () => {
    error.clear(); setLoading(true);
    try {
      const next = await loadResetAutomation(true);
      if (!alive.current) return;
      setSnapshot(next); setPublicDraft(next.public_rule);
      const account = next.accounts[accountId];
      expectedAccount.current = account ?? { mode: 'inherit' }; expectedPublic.current = next.public_rule;
      expectedAccounts.current = next.accounts; setApplyToAll(false);
      setCustomDraft(initialCustom(account, next.public_rule));
    } catch (e) { if (alive.current) error.report(t('codex.resetAutomation.loadFailed', { error: String(e) })); }
    finally { if (alive.current) setLoading(false); }
  };
  useEffect(() => { if (!snapshot) void load(); }, []);
  const clear = () => { error.clear(); setFieldError(null); };
  const save = async () => {
    if (submitting.current || !snapshot) return;
    clear();
    const previous = tab === 'public' ? expectedPublic.current : expectedAccount.current.mode === 'custom' ? expectedAccount.current.rule : snapshot.public_rule;
    const rule = prepareResetRuleDraft(tab === 'public' ? publicDraft : customDraft, previous);
    const invalid = validateResetRule(rule);
    if (invalid) { setFieldError(invalid); setValidationAttempt(value => value + 1); return; }
    submitting.current = true; setBusy(true);
    try {
      const next = tab === 'public' ? await saveResetPublicRule(rule, expectedPublic.current, applyToAll, applyToAll ? expectedAccounts.current : undefined)
        : await saveResetAccountRule(accountId, { mode: 'custom', rule }, expectedAccount.current);
      if (!alive.current) return;
      setSnapshot(next); setApplyToAll(false); onClose();
    } catch (e) { if (alive.current) error.report(t('codex.resetAutomation.saveFailed', { error: String(e) })); }
    finally { submitting.current = false; if (alive.current) setBusy(false); }
  };
  const live = useSyncExternalStore(subscribeResetAutomation, getResetAutomationSnapshot, getResetAutomationSnapshot);
  const editorRule = tab === 'public' ? publicDraft : customDraft;
  const records = (live ?? snapshot)?.records.filter(r => tab === 'public' || r.account_id === accountId).slice(0, 10) ?? [];
  return createPortal(<div className="modal-overlay reset-automation-overlay">
    <div ref={dialogRef} tabIndex={-1} className="modal reset-automation-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId}>
      <div className="modal-header"><h2 id={titleId}>{t('codex.resetAutomation.title')}</h2>
        <button type="button" className="btn btn-secondary" onClick={onClose} aria-label={t('common.close')}><X size={18} /></button></div>
      <div className="modal-body">
        <div className="reset-automation-tabs" role="tablist">
          {(['account', 'public'] as const).map(value => <button type="button" role="tab" aria-selected={tab === value}
            className={`btn ${tab === value ? 'is-active' : ''}`} disabled={busy} key={value}
            onClick={() => { setTab(value); clear(); }}>{t(`codex.resetAutomation.${value}Tab`)}</button>)}
        </div>
        {accountLabel && tab === 'account' && <p className="reset-automation-account" title={accountLabel}>{accountLabel}</p>}
        {loading ? <p role="status"><LoaderCircle size={16} className="loading-spinner" /> {t('common.loading')}</p> : snapshot && <>
          <p className="reset-automation-hint">{t(`codex.resetAutomation.${tab === 'public' ? 'publicHint' : 'accountHint'}`)}</p>
          <fieldset disabled={busy} className="reset-automation-editor">
            <RuleEditor key={tab} rule={editorRule} t={t} fieldError={fieldError} validationAttempt={validationAttempt}
              onChange={rule => {
                if (tab === 'account') setCustomDraft(rule);
                else setPublicDraft(rule);
                clear();
              }} />
          </fieldset>
          {tab === 'public' && <section className="reset-automation-apply-all">
            <div className="reset-automation-setting-row">
              <label htmlFor={`${titleId}-apply-all`}>{t('codex.resetAutomation.applyAll')}</label>
              <button type="button" id={`${titleId}-apply-all`} className={`btn reset-automation-switch ${applyToAll ? 'is-on' : ''}`}
                role="switch" aria-label={t('codex.resetAutomation.applyAll')} aria-checked={applyToAll} disabled={busy}
                aria-describedby={`${titleId}-apply-all-hint`} onClick={() => { setApplyToAll(value => !value); clear(); }}><span /></button>
            </div>
            <p id={`${titleId}-apply-all-hint`} className="reset-automation-hint">{t('codex.resetAutomation.applyAllHint')}</p>
          </section>}
          <p className="reset-automation-hint">{t('codex.resetAutomation.hostHint')}</p>
          {records.length > 0 && <details className="reset-automation-history"><summary>{t('codex.resetAutomation.history')}</summary>
            <ol>{records.map(record => <li key={record.id}>
              <time>{new Date(record.at * 1000).toLocaleString()}</time>
              <span>{tab === 'public' && record.account_id !== accountId ? `${record.account_id.slice(0, 8)} · ` : ''}{t(`codex.resetAutomation.reason${record.reason[0].toUpperCase()}${record.reason.slice(1)}`)} · {t(record.status === 'pending' ? 'codex.resetAutomation.pending' : `common.${record.status}`)}</span>
              {record.detail && <p>{record.detail}</p>}
            </li>)}</ol>
          </details>}
        </>}
        <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} position="bottom" />
        {!loading && error.message && <button type="button" className="btn btn-secondary" onClick={() => void load()}>{t('common.retry')}</button>}
      </div>
      <div className="modal-footer"><button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.cancel')}</button>
        <button type="button" className="btn btn-primary" disabled={loading || busy || !snapshot} onClick={() => void save()}>
          {busy && <LoaderCircle size={15} className="loading-spinner" />}{t('common.save')}</button></div>
    </div>
  </div>, document.body);
}
