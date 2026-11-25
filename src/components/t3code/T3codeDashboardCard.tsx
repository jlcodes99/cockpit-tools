import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';
import { CheckCircle2, Layers, RotateCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { T3CodeIcon } from '../icons/T3CodeIcon';
import * as service from '../../services/t3codeService';
import { useAccountStore } from '../../stores/useAccountStore';
import { useClaudeAccountStore } from '../../stores/useClaudeAccountStore';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import type { Page } from '../../types/navigation';
import type { T3CodeProviderInstanceView, T3CodeProviderWorkspaceView } from '../../types/t3code';
import { formatCodexProviderQuota } from '../../utils/codexProviderPoolQuota';
import { getPlatformLabel, renderPlatformIcon } from '../../utils/platformMeta';
import { getT3codePlatformSpec } from './t3codePlatformRegistry';
import {
  T3codeAccountPlanBadge,
  formatT3codeQuotaWindow,
  getQuotaTone,
  normalizeWindowResetText,
  pickSummaryWindows,
} from './t3codePresentation';

interface T3codeDashboardCardProps {
  maskAccountText: (value?: string | null) => string;
  hideCardButton: ReactNode;
  onNavigate: (page: Page) => void;
}

export function T3codeDashboardCard({
  maskAccountText,
  hideCardButton,
  onNavigate,
}: T3codeDashboardCardProps) {
  const { t } = useTranslation();
  const [workspace, setWorkspace] = useState<T3CodeProviderWorkspaceView | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const antigravityAccounts = useAccountStore((state) => state.accounts);
  const codexAccounts = useCodexAccountStore((state) => state.accounts);
  const claudeAccounts = useClaudeAccountStore((state) => state.accounts);

  const loadWorkspace = useCallback(async () => {
    try {
      const next = await service.getT3codeProviderWorkspace(null);
      setWorkspace(next);
    } catch {
      // Non-blocking background read for dashboard summary
    }
  }, []);

  useEffect(() => {
    void loadWorkspace();
    let active = true;
    const stop = listen<string>('t3code-provider-binding-changed', () => {
      if (active) void loadWorkspace();
    }).catch(() => undefined);
    return () => {
      active = false;
      void stop.then((unlisten) => unlisten?.());
    };
  }, [loadWorkspace]);

  const handleRefreshCard = async () => {
    if (refreshing) return;
    setRefreshing(true);
    try {
      const fresh = await service.getT3codeProviderWorkspace(null);
      setWorkspace(fresh);
      const targets = fresh.providerInstances.filter(
        (item) => item.enabled && item.boundAccountId && !item.boundIsApiService,
      );
      for (const item of targets) {
        try {
          const updated = await service.refreshT3codeProviderAccountQuota({
            t3InstanceId: null,
            providerInstanceId: item.instanceId,
          });
          setWorkspace(updated);
        } catch {
          // Continue refreshing remaining bound providers
        }
      }
    } finally {
      setRefreshing(false);
    }
  };

  const gatewayName = t('common.apiBudget.gatewayName');
  const activeProviders = (workspace?.providerInstances ?? []).filter(
    (item) => item.enabled && (item.boundAccountId || item.boundAccountEmail),
  );
  const primarySlot = activeProviders[0] ?? null;
  const secondarySlot = activeProviders[1] ?? null;

  const renderSlotContent = (instance: T3CodeProviderInstanceView | null, emptyText: string) => {
    if (!instance) {
      return <div className="empty-slot">{emptyText}</div>;
    }
    const spec = getT3codePlatformSpec(instance.driver);
    const displayEmail = instance.boundIsApiService
      ? gatewayName
      : maskAccountText(instance.boundAccountEmail || instance.displayName);
    const windows = pickSummaryWindows(instance.driver, instance.windows, 2, instance.quotaSummaryGroups);
    const providerQuota = instance.providerQuota
      ? formatCodexProviderQuota(instance.providerQuota, t)
      : '';

    const agAccount =
      instance.driver === 'antigravity' && instance.boundAccountId
        ? antigravityAccounts.find((item) => item.id === instance.boundAccountId)
        : undefined;
    const codexAccount =
      instance.driver === 'codex' && instance.boundAccountId
        ? codexAccounts.find((item) => item.id === instance.boundAccountId)
        : undefined;
    const claudeAccount =
      instance.driver === 'claudeAgent' && instance.boundAccountId
        ? claudeAccounts.find((item) => item.id === instance.boundAccountId)
        : undefined;

    return (
      <div className="account-mini-card">
        <div className="account-mini-header">
          <div className="account-info-row">
            <span style={{ display: 'inline-flex', flexShrink: 0 }} aria-hidden="true">
              {renderPlatformIcon(spec.platformId, 14)}
            </span>
            <span className="account-email" title={`${spec.label} · ${displayEmail}`}>
              {displayEmail}
            </span>
            <T3codeAccountPlanBadge
              driver={instance.driver}
              plan={instance.boundAccountPlan}
              apiService={instance.boundIsApiService}
              antigravity={agAccount}
              codex={codexAccount}
              claude={claudeAccount}
            />
          </div>
        </div>

        <div className="account-mini-quotas">
          {windows.length > 0 ? (
            windows.map((win) => {
              const { label, value, percent } = formatT3codeQuotaWindow(instance.driver, win, t);
              const tone = percent == null ? '' : getQuotaTone(percent);
              const quotaClass = tone === 'critical' ? 'low' : tone;
              const resetText = normalizeWindowResetText(win.resetTime, t);
              return (
                <div key={win.id} className="mini-quota-row-stacked">
                  <div className="mini-quota-header">
                    <span className="model-name">{label}</span>
                    <span className={`model-pct ${quotaClass}`}>{value}</span>
                  </div>
                  {percent != null && (
                    <div className="mini-progress-track">
                      <div className={`mini-progress-bar ${quotaClass}`} style={{ width: `${percent}%` }} />
                    </div>
                  )}
                  {resetText && <div className="mini-reset-time">{resetText}</div>}
                </div>
              );
            })
          ) : providerQuota ? (
            <div className="mini-quota-row-stacked">
              <div className="mini-quota-header">
                <span className="model-name">{spec.label}</span>
                <span className="model-pct high">{providerQuota}</span>
              </div>
            </div>
          ) : (
            <div className="empty-slot-text">{t('t3code.quotaQuery.empty')}</div>
          )}
        </div>
      </div>
    );
  };

  return (
    <div className="main-card windsurf-card">
      <div className="main-card-header">
        <div className="header-title">
          <T3CodeIcon size={18} />
          <h3>{getPlatformLabel('t3code', t)}</h3>
        </div>
        <div className="header-action-group">
          <button
            type="button"
            className="header-action-btn"
            onClick={() => void handleRefreshCard()}
            disabled={refreshing}
            title={t('common.refresh')}
          >
            <RotateCw size={14} className={refreshing ? 'loading-spinner' : ''} />
            <span>{t('common.refresh')}</span>
          </button>
          {hideCardButton}
        </div>
      </div>

      <div className="split-content">
        <div className="split-half current-half">
          <span className="half-label">
            <CheckCircle2 size={12} /> {t('t3code.accountConfig.providers')}
          </span>
          {renderSlotContent(primarySlot, t('t3code.providerWorkspace.unbound'))}
        </div>

        <div className="split-divider" />

        <div className="split-half recommend-half">
          <span className="half-label">
            <Layers size={12} />{' '}
            {activeProviders.length > 2
              ? `${t('t3code.providerWorkspace.providerInstance')} (+${activeProviders.length - 1})`
              : t('t3code.providerWorkspace.providerInstance')}
          </span>
          {secondarySlot ? (
            renderSlotContent(secondarySlot, '')
          ) : (
            <div className="empty-slot-text">{t('t3code.providerWorkspace.unbound')}</div>
          )}
        </div>
      </div>

      <button type="button" className="card-footer-action" onClick={() => onNavigate('t3code')}>
        {t('dashboard.viewAllAccounts')}
      </button>
    </div>
  );
}
