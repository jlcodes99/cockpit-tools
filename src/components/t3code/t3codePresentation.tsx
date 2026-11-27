import type { ReactNode } from 'react';
import type { TFunction } from 'i18next';
import { Database } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { CodexAccountPlanBadge } from '../codex/CodexAccountPlanBadge';
import type { Account } from '../../types/account';
import { getClaudePlanBadge, getClaudePlanBadgeClass, type ClaudeAccount } from '../../types/claude';
import type { CodexAccount } from '../../types/codex';
import { resolveQoderPlanBadgeClass } from '../../types/qoder';
import type {
  T3CodeAntigravityQuotaGroup,
  T3CodeBindableAccountView,
  T3CodeDriverKind,
  T3CodeProviderInstanceView,
  T3CodeProviderRuntimeView,
  T3CodeQuotaWindowView,
} from '../../types/t3code';
import { formatResetTimeDisplay, getAntigravityTierBadge } from '../../utils/account';
import {
  formatCodexProviderQuota,
  type CodexProviderQuotaDisplay,
} from '../../utils/codexProviderPoolQuota';
import { getT3codePlatformSpec } from './t3codePlatformRegistry';
import { PLATFORM_LAUNCH_PATHS, type PlatformLaunchPathTarget } from '../../utils/platformLaunchPath';

export type T3codeStatusTone = 'ready' | 'pending' | 'warning' | 'muted';
export type T3codeQuotaTone = 'high' | 'medium' | 'low' | 'critical';

export interface T3codeResolvedRowStatus {
  tone: T3codeStatusTone;
  text: string;
  title?: string;
  needsRestart: boolean;
  pathTarget?: PlatformLaunchPathTarget;
}

export function getT3codeRuntimePathTarget(driver: T3CodeDriverKind): PlatformLaunchPathTarget | undefined {
  if (driver === 'antigravity') return 'antigravity_cli';
  if (driver === 'claudeAgent') return 'claude_cli';
  if (driver === 'codebuddy' || driver === 'codebuddy_cn') return 'codebuddy_cli';
}

export function getQuotaTone(percent: number): T3codeQuotaTone {
  if (percent >= 70) return 'high';
  if (percent >= 35) return 'medium';
  if (percent >= 10) return 'low';
  return 'critical';
}

export function normalizeWindowResetText(
  resetTime: string | undefined,
  t: TFunction,
): string {
  if (!resetTime) return '';
  const trimmed = resetTime.trim();
  if (!trimmed) return '';
  if (/^\d+$/.test(trimmed)) {
    const num = Number(trimmed);
    const ms = num > 1e12 ? num : num * 1000;
    const date = new Date(ms);
    if (!Number.isNaN(date.getTime())) {
      return formatResetTimeDisplay(date.toISOString(), t);
    }
  }
  return formatResetTimeDisplay(trimmed, t) || trimmed;
}

function formatGroupWindowLabel(group: string, period: string): string {
  return period ? `${group} ${period === 'weekly' ? '7d' : period}` : group;
}

function getAntigravityGroupWindows(
  windows: T3CodeQuotaWindowView[],
  groups?: T3CodeAntigravityQuotaGroup[],
): T3CodeQuotaWindowView[] {
  if (groups) {
    const resetTimes = new Map(windows.map(({ id, resetTime }) => [id, resetTime]));
    return groups.flatMap((group) => group.windows.map((window) => ({
      id: `${group.id}:${window.id}`,
      label: formatGroupWindowLabel(group.name, window.period),
      remainingPercent: window.percentage,
      resetTime: resetTimes.get(window.id),
    })));
  }

  // Older records can still carry real group windows in the flat quota snapshot.
  const knownGroups = [
    { name: 'Gemini', prefix: /^gemini[-:]/ },
    { name: 'Claude', prefix: /^(?:3p|claude)[-:]/ },
    { name: 'Gemini 4 Carbon', prefix: /^(?:gemini-carbon(?:-wtu)?|carbon)[-:]/ },
  ];
  return knownGroups.flatMap(({ name, prefix }) => ['5h', 'weekly'].flatMap((period) => {
    const window = windows.find((item) =>
      item.remainingPercent != null && item.id.replace(prefix, '') === period && prefix.test(item.id));
    return window ? [{ ...window, label: formatGroupWindowLabel(name, period) }] : [];
  }));
}

export function pickSummaryWindows(
  driver: T3CodeDriverKind,
  windows: T3CodeQuotaWindowView[],
  maxItems: number | undefined,
  quotaSummaryGroups?: T3CodeAntigravityQuotaGroup[],
): T3CodeQuotaWindowView[] {
  const summary = driver === 'antigravity' ? getAntigravityGroupWindows(windows, quotaSummaryGroups) : windows;
  return summary.slice(0, maxItems);
}

const quotaAmountFormat = new Intl.NumberFormat(undefined, { maximumFractionDigits: 6 });

export function formatT3codeQuotaWindow(
  driver: T3CodeDriverKind,
  window: T3CodeQuotaWindowView,
  t: TFunction,
  allowQuotaOverflow = false,
) {
  let label = window.label;
  if (window.id === 'credits') label = t('accounts.credits.label');
  if (driver === 'codex' && (window.id === 'weekly' || window.label === '7d')) label = 'Weekly';
  if (window.remainingAmount != null) {
    return { label, value: quotaAmountFormat.format(window.remainingAmount), percent: undefined };
  }
  const remaining = Math.max(0, Math.round(window.remainingPercent));
  const percent = allowQuotaOverflow ? remaining : Math.min(100, remaining);
  return { label, value: `${percent}%`, percent };
}

export function comparePrimaryQuota(left: T3CodeQuotaWindowView[], right: T3CodeQuotaWindowView[]) {
  const [first] = left;
  const [second] = right;
  if (!first || !second) return Number(Boolean(first)) - Number(Boolean(second));
  if (first.remainingAmount != null && second.remainingAmount != null) {
    return first.remainingAmount - second.remainingAmount;
  }
  if (first.remainingPercent != null && second.remainingPercent != null) {
    return first.remainingPercent - second.remainingPercent;
  }
  return 0;
}

export function formatAccountLabel(
  account: Pick<T3CodeBindableAccountView, 'email' | 'displayName' | 'isApiService'>,
  gatewayLabel: string,
  maskText?: (value: string) => string,
): string {
  if (account.isApiService) return gatewayLabel;
  const applyMask = maskText ?? ((value: string) => value);
  const maskedEmail = account.email ? applyMask(account.email) : '';
  const maskedName = account.displayName?.trim() ? applyMask(account.displayName.trim()) : '';
  if (maskedName && maskedName !== maskedEmail) {
    return maskedEmail ? `${maskedName} · ${maskedEmail}` : maskedName;
  }
  return maskedEmail;
}

export function suggestInstanceName(
  driver: T3CodeDriverKind,
  account: T3CodeBindableAccountView | undefined,
  gatewayLabel: string,
): string {
  const base = getT3codePlatformSpec(driver).label;
  if (!account) return base;
  if (account.isApiService) return `${base} · ${gatewayLabel}`;
  const raw = account.displayName?.trim() || account.email.split('@')[0]?.trim() || account.email;
  const short = raw.length > 20 ? `${raw.slice(0, 20)}...` : raw;
  return `${base} · ${short}`;
}

export function makeProviderSlug(driver: T3CodeDriverKind): string {
  const prefix = driver === 'claudeAgent' ? 'claude' : driver.replace(/-/g, '_');
  const rand = globalThis.crypto.randomUUID().replace(/-/g, '').slice(0, 8);
  return `${prefix}_${rand}`;
}

export function isRuntimeOlderThanBinding(
  runtime: T3CodeProviderRuntimeView,
  boundUpdatedAt?: number,
): boolean {
  if (!boundUpdatedAt || !runtime.checkedAt) return false;
  const checkedMs = Date.parse(runtime.checkedAt);
  return Number.isFinite(checkedMs) && Math.floor(checkedMs / 1000) + 1 < boundUpdatedAt;
}

export function doesRuntimeMatchBound(
  instance: T3CodeProviderInstanceView,
  runtime?: T3CodeProviderRuntimeView,
): boolean {
  if (!runtime) return false;
  if (instance.boundIsApiService) return runtime.matchesGateway;
  if (instance.driver === 'antigravity' && (runtime.authUsesKeyring || !runtime.accountEmail)) return false;
  if (!instance.boundAccountEmail || !runtime.accountEmail) {
    return Boolean(
      instance.boundAccountId &&
        runtime.authStatus === 'authenticated' &&
        runtime.availability === 'available' &&
        runtime.modelCount > 0,
    );
  }
  return (
    instance.boundAccountEmail.toLowerCase() === runtime.accountEmail.toLowerCase() &&
    (!instance.boundAccountWorkspaceId ||
      instance.boundAccountWorkspaceId === runtime.workspaceId ||
      (instance.driver === 'grok' && !runtime.workspaceId))
  );
}

export function resolveEffectiveRuntime(
  instance: T3CodeProviderInstanceView,
  live: T3CodeProviderRuntimeView | undefined,
): T3CodeProviderRuntimeView | undefined {
  if (instance.runtime?.source === 'staged') {
    if (
      live &&
      !isRuntimeOlderThanBinding(live, instance.boundUpdatedAt) &&
      (live.installed === false ||
        (live.availability === 'available' && live.modelCount > 0 && doesRuntimeMatchBound(instance, live)))
    ) {
      return live;
    }
    return instance.runtime;
  }
  const stored = instance.runtime;
  if (instance.driver === 'antigravity' && stored?.accountEmail && live && !live.accountEmail) {
    return { ...live, accountEmail: stored.accountEmail, authUsesKeyring: stored.authUsesKeyring };
  }
  if (stored?.checkedAt && (!live?.checkedAt || stored.checkedAt > live.checkedAt)) {
    return stored;
  }
  return live ?? stored;
}

export function resolveProviderRowStatus(
  instance: T3CodeProviderInstanceView,
  runtime: T3CodeProviderRuntimeView | undefined,
  t3Running: boolean,
  t: TFunction,
  t3StartedAt?: string,
): T3codeResolvedRowStatus {
  if (instance.pendingDefault?.state === 'waiting') {
    return { tone: 'pending', text: t('t3code.providerWorkspace.defaultWaiting'), needsRestart: false };
  }
  if (instance.pendingDefault?.state === 'restartRequired') {
    return {
      tone: 'pending',
      text: t(t3Running ? 't3code.quotaQuery.restartToApply' : 't3code.providerWorkspace.readyOnLaunch'),
      title: instance.pendingDefault.error,
      needsRestart: t3Running,
    };
  }
  if (instance.pendingDefault?.state === 'failed') {
    return {
      tone: 'warning',
      text: t('t3code.providerWorkspace.defaultApplyFailed'),
      title: instance.pendingDefault.error,
      needsRestart: false,
    };
  }
  if (!instance.enabled) {
    return { tone: 'muted', text: t('common.disabled'), needsRestart: false };
  }
  if (!instance.boundAccountId && !instance.boundAccountEmail) {
    return { tone: 'muted', text: t('t3code.providerWorkspace.unbound'), needsRestart: false };
  }
  if (t3Running && instance.modelRefreshPending) {
    return { tone: 'pending', text: t('t3code.providerWorkspace.runtimePending'), needsRestart: false };
  }
  const pathTarget = getT3codeRuntimePathTarget(instance.driver);
  if (t3Running && runtime?.installed === false && runtime.status === 'error') {
    return {
      tone: 'warning',
      text: pathTarget
        ? t('t3code.providerWorkspace.runtimeNotFound', { app: PLATFORM_LAUNCH_PATHS[pathTarget].label })
        : t('t3code.providerWorkspace.runtimeError'),
      title: runtime.message,
      needsRestart: false,
      pathTarget,
    };
  }
  const bootTime = t3StartedAt ? Date.parse(t3StartedAt) : undefined;
  const boundAt = instance.boundUpdatedAt;
  const bindingSinceBoot = Boolean(boundAt && (bootTime === undefined || !Number.isFinite(bootTime) || boundAt * 1000 >= bootTime));
  if (t3Running && instance.bindingAppliedLive &&
      (runtime?.source === 'staged' || (runtime && isRuntimeOlderThanBinding(runtime, instance.boundUpdatedAt)))) {
    return { tone: 'pending', text: t('t3code.providerWorkspace.runtimePending'), needsRestart: false };
  }
  if (t3Running && instance.driver === 'antigravity' && runtime?.authStatus === 'authenticated') {
    if (runtime.authUsesKeyring || (runtime.accountEmail && instance.boundAccountEmail &&
        runtime.accountEmail.toLowerCase() !== instance.boundAccountEmail.toLowerCase())) {
      return { tone: 'warning', text: t('t3code.overview.accountMismatch'), needsRestart: Boolean(runtime.authUsesKeyring) };
    }
    if (!runtime.accountEmail) {
      return { tone: 'pending', text: t('t3code.providerWorkspace.runtimePending'), needsRestart: false };
    }
  }
  if (instance.needsRepair) {
    return {
      tone: 'pending',
      text: t('t3code.providerWorkspace.needsSyncVerify'),
      needsRestart: false,
    };
  }
  if (runtime?.source === 'staged') {
    if (!t3Running || !bindingSinceBoot) {
      return {
        tone: 'pending',
        text: t(t3Running ? 't3code.providerWorkspace.runtimePending' : 't3code.providerWorkspace.readyOnLaunch'),
        needsRestart: false,
      };
    }
    return {
      tone: 'pending',
      text:
        runtime.modelCount > 0
          ? t('t3code.providerWorkspace.stagedReady', { count: runtime.modelCount })
          : t('t3code.quotaQuery.restartToApply'),
      title: t('t3code.quotaQuery.restartToApply'),
      needsRestart: true,
    };
  }
  const matches = doesRuntimeMatchBound(instance, runtime);
  if (runtime?.authStatus === 'unauthenticated') {
    return {
      tone: 'warning',
      text: t('t3code.providerWorkspace.authRequired'),
      title: runtime.message,
      needsRestart: false,
    };
  }
  if (t3Running && instance.driver === 'grok' && runtime?.authStatus === 'authenticated' &&
      runtime.accountEmail && instance.boundAccountEmail && !matches) {
    return { tone: 'warning', text: t('t3code.overview.accountMismatch'), needsRestart: false };
  }
  const readyWithoutAuthDetails = runtime?.authStatus === 'unknown' &&
    runtime.status === 'ready' && !runtime.accountEmail && Boolean(instance.boundAccountId) &&
    !['antigravity', 'codex', 'claudeAgent'].includes(instance.driver);
  if (
    ((runtime?.authStatus === 'authenticated' && matches) || readyWithoutAuthDetails) &&
    runtime?.availability === 'available' &&
    runtime.modelCount > 0
  ) {
    const labelKey =
      runtime.source === 'cache'
        ? 't3code.providerWorkspace.verifiedReady'
        : 't3code.providerWorkspace.modelsReady';
    return {
      tone: 'ready',
      text: t(labelKey, { count: runtime.modelCount }),
      title: runtime.message,
      needsRestart: false,
    };
  }
  if (
    instance.boundIsApiService &&
    runtime?.matchesGateway &&
    runtime.availability === 'available' &&
    runtime.modelCount > 0
  ) {
    return {
      tone: 'ready',
      text: t('t3code.providerWorkspace.modelsReady', { count: runtime.modelCount }),
      needsRestart: false,
    };
  }
  if (t3Running) {
    if (runtime?.status === 'error') {
      return {
        tone: 'warning',
        text: t('t3code.providerWorkspace.runtimeError'),
        title: runtime.message,
        needsRestart: false,
        pathTarget: pathTarget === 'codebuddy_cli' ? pathTarget : undefined,
      };
    }
    if (!instance.bindingAppliedLive && bindingSinceBoot &&
        (!runtime?.checkedAt || isRuntimeOlderThanBinding(runtime, boundAt))) {
      return {
        tone: 'pending',
        text: t('t3code.quotaQuery.restartToApply'),
        title: t('t3code.quotaQuery.restartToApply'),
        needsRestart: true,
      };
    }
    return { tone: 'pending', text: t('t3code.providerWorkspace.runtimePending'), needsRestart: false };
  }
  return {
    tone: 'muted',
    text: t('t3code.providerWorkspace.readyOnLaunch'),
    title: runtime?.message,
    needsRestart: false,
  };
}

export interface T3codeAccountPlanBadgeProps {
  driver: T3CodeDriverKind;
  plan?: string;
  apiService?: boolean;
  antigravity?: Account;
  codex?: CodexAccount;
  claude?: ClaudeAccount;
}

export function T3codeAccountPlanBadge({
  driver,
  plan,
  apiService,
  antigravity,
  codex,
  claude,
}: T3codeAccountPlanBadgeProps): ReactNode {
  const { t } = useTranslation();

  if (apiService) {
    const label = t('t3code.overview.localGatewayBadge');
    return (
      <span className="tier-badge raw-value business" title={label}>
        {label}
      </span>
    );
  }
  if (codex) {
    return <CodexAccountPlanBadge account={codex} />;
  }
  if (claude) {
    const label = getClaudePlanBadge(claude);
    return (
      <span className={`tier-badge ${getClaudePlanBadgeClass(claude)}`} title={label}>
        {label}
      </span>
    );
  }
  if (driver === 'antigravity') {
    const quotaSource = antigravity?.quota?.subscription_tier
      ? antigravity.quota
      : plan && plan !== 'UNKNOWN'
        ? { subscription_tier: plan }
        : undefined;
    const badge = getAntigravityTierBadge(quotaSource);
    const text = badge.label !== '—' ? badge.label : t('common.unknown');
    return (
      <span className={`tier-badge ${badge.className}`} title={badge.rawTier || text}>
        {text}
      </span>
    );
  }
  if (driver === 'qoder' || driver === 'qoder_cn') {
    const label = plan?.trim() || 'UNKNOWN';
    return <span className={`tier-badge raw-value ${resolveQoderPlanBadgeClass(label)}`} title={label}>{label}</span>;
  }
  if (!plan?.trim() || plan.trim().toLowerCase() === 'unknown') {
    return <span className="tier-badge t3code-plan-neutral" title={t('common.unknown')}>—</span>;
  }
  const label = plan;
  const lower = label.toLowerCase();
  const toneClass = [
    [/enterprise/, 'enterprise'],
    [/team/, 'team'],
    [/trial/, 'trial'],
    [/pro|max|ultimate|advanced|youth|vip/, 'pro'],
    [/plus/, 'plus'],
    [/free/, 'free'],
  ] as const;
  const className = toneClass.find(([pattern]) => pattern.test(lower))?.[1] ?? 't3code-plan-neutral';
  return (
    <span className={`tier-badge ${className}`} title={label}>
      {label}
    </span>
  );
}

export interface T3codeQuotaSectionProps {
  driver: T3CodeDriverKind;
  windows: T3CodeQuotaWindowView[];
  quotaSummaryGroups?: T3CodeAntigravityQuotaGroup[];
  providerQuota?: CodexProviderQuotaDisplay;
  speedTokPerSec?: number;
  cacheHitPercent?: number;
  compact?: boolean;
  allowQuotaOverflow?: boolean;
  emptyLabel?: string;
}

export function T3codeQuotaSection({
  driver,
  windows,
  quotaSummaryGroups,
  providerQuota,
  speedTokPerSec,
  cacheHitPercent,
  compact = false,
  allowQuotaOverflow = false,
  emptyLabel,
}: T3codeQuotaSectionProps) {
  const { t } = useTranslation();
  const maxItems = driver === 'antigravity' ? undefined : compact ? 2 : 4;
  const visibleWindows = pickSummaryWindows(driver, windows, maxItems, quotaSummaryGroups);
  const providerQuotaText = providerQuota ? formatCodexProviderQuota(providerQuota, t) : null;
  const hasTelemetry =
    (typeof speedTokPerSec === 'number' && speedTokPerSec > 0) ||
    (typeof cacheHitPercent === 'number' && cacheHitPercent >= 0);

  if (visibleWindows.length === 0 && !providerQuotaText) {
    return (
      <div className={`t3code-quota-empty${compact ? ' is-compact' : ''}`}>
        <Database size={compact ? 14 : 15} aria-hidden="true" />
        <span>{emptyLabel || t('t3code.quotaQuery.empty')}</span>
      </div>
    );
  }

  return (
    <div className={`t3code-quota-list${compact ? ' is-compact' : ''}`}>
      {visibleWindows.map((window) => {
        const { label: fullLabel, value, percent } = formatT3codeQuotaWindow(driver, window, t, allowQuotaOverflow);
        const weekly = t('settings.general.codexAppUiInjectionWeeklyLabel');
        const label = compact ? fullLabel.replace(/\b(?:7d|weekly)$/i, weekly) : fullLabel;
        const tone = percent == null ? null : getQuotaTone(percent);
        const resetText = normalizeWindowResetText(window.resetTime, t);
        return (
          <div key={window.id} className="t3code-quota-item">
            <div className="t3code-quota-row">
              <span className="t3code-quota-label" title={label}>
                {label}
              </span>
              <span className={`t3code-quota-percent${tone ? ` is-${tone}` : ''}`}>{value}</span>
            </div>
            {!compact && percent != null && (
              <div className="t3code-quota-track" aria-hidden="true">
                <span className={`t3code-quota-fill is-${tone}`} style={{ width: `${Math.min(100, percent)}%` }} />
              </div>
            )}
            {!compact && resetText && (
              <div className="t3code-quota-reset" title={resetText}>
                {resetText}
              </div>
            )}
          </div>
        );
      })}
      {providerQuotaText && (
        <div className="t3code-provider-quota-box" title={providerQuotaText}>
          <span className="t3code-provider-quota-value">{providerQuotaText}</span>
          {hasTelemetry && !compact && (
            <span className="t3code-provider-quota-telemetry">
              {typeof speedTokPerSec === 'number' && speedTokPerSec > 0
                ? `${speedTokPerSec.toFixed(1)} tok/s`
                : null}
              {typeof speedTokPerSec === 'number' &&
              speedTokPerSec > 0 &&
              typeof cacheHitPercent === 'number'
                ? ' · '
                : null}
              {typeof cacheHitPercent === 'number' ? `Cache ${Math.round(cacheHitPercent)}%` : null}
            </span>
          )}
        </div>
      )}
    </div>
  );
}
