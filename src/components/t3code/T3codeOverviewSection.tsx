import { T3codeCommonSettings } from './T3codeCommonSettings';
import { useMemo, useState, type ReactNode } from 'react';
import { CheckCircle2, Eye, EyeOff, Play, Plus, RefreshCw, RotateCw, Search } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { QuickSettingsPopover } from '../QuickSettingsPopover';
import { SingleSelectDropdown, type SingleSelectOption } from '../SingleSelectDropdown';
import { useDurableUiSetting } from '../../hooks/useDurableUiSetting';
import type { T3CodeDriverKind, T3CodeProviderInstanceView, T3CodeProviderWorkspaceView } from '../../types/t3code';
import { T3CODE_PLATFORM_ORDER_STORAGE_KEY } from '../../utils/durableUiSettings';
import { renderPlatformIcon } from '../../utils/platformMeta';
import { getOrderedT3codePlatforms } from './t3codePlatformRegistry';
import { T3codeQuotaSection } from './t3codePresentation';
import type { T3codeProviderRowView } from './useT3codeWorkspaceController';

interface T3codeOverviewSectionProps {
  workspace: T3CodeProviderWorkspaceView | null;
  instanceOptions: SingleSelectOption[];
  targetId: string;
  onSelectTarget: (targetId: string) => void;
  providers: T3codeProviderRowView[];
  t3Running: boolean;
  injectionConnected: boolean;
  loading: boolean;
  busyInstanceId: string | null;
  refreshingAll: boolean;
  restarting: boolean;
  gatewayName: string;
  privacyModeEnabled: boolean;
  onTogglePrivacyMode: () => void;
  maskAccountText: (value?: string | null) => string;
  renderPlanBadge: (driver: T3CodeDriverKind, id?: string, plan?: string, apiService?: boolean) => ReactNode;
  onAdd: (driver: T3CodeDriverKind) => void;
  onSetDefault: (instance: T3CodeProviderInstanceView) => void;
  onSwitch: (instance: T3CodeProviderInstanceView) => void;
  onRemove: (instance: T3CodeProviderInstanceView) => void;
  onConfigureRuntime: (instance: T3CodeProviderInstanceView) => void;
  onRefreshAll: () => void;
  onStartOrRestart: () => void;
}

export function T3codeOverviewSection({
  workspace, instanceOptions, targetId, onSelectTarget,
  providers, t3Running, injectionConnected, loading, busyInstanceId,
  refreshingAll, restarting, gatewayName, privacyModeEnabled, onTogglePrivacyMode,
  maskAccountText, renderPlanBadge, onAdd, onSetDefault, onSwitch, onRemove, onConfigureRuntime, onRefreshAll, onStartOrRestart,
}: T3codeOverviewSectionProps) {
  const { t } = useTranslation();
  const [search, setSearch] = useState('');
  const savedOrder = useDurableUiSetting(T3CODE_PLATFORM_ORDER_STORAGE_KEY);
  const busy = Boolean(busyInstanceId) || restarting;

  const groups = useMemo(() => {
    const query = search.trim().toLowerCase();
    return getOrderedT3codePlatforms(savedOrder).flatMap((spec) => {
      const instances = providers.filter(({ instance }) => instance.driver === spec.driver);
      const accounts = spec.getAccounts(workspace);
      if (!instances.length && !accounts.length) return [];
      const rows = instances.flatMap((row) => {
        const { instance } = row;
        if (!instance.boundAccountId && !instance.boundAccountEmail && instance.isBuiltinDefault) return [];
        const account = accounts.find((item) => item.id === instance.boundAccountId);
        const email = instance.boundAccountEmail?.trim() || account?.email?.trim() || '';
        const displayName = account?.displayName?.trim();
        const primary = instance.boundIsApiService ? gatewayName : maskAccountText(displayName || email || instance.displayName);
        const secondary = !instance.boundIsApiService && displayName && email.toLowerCase() !== displayName.toLowerCase()
          ? maskAccountText(email) : '';
        return [{
          ...row,
          primary,
          secondary,
          email,
          displayName,
          isDefault: instance.instanceId === spec.driver,
        }];
      });
      const visible = rows.filter(({ primary, secondary, email, displayName, instance }) =>
        [spec.label, primary, secondary, email, displayName, instance.boundAccountPlan]
          .some((value) => value?.toLowerCase().includes(query)));
      if (query && !visible.length && !spec.label.toLowerCase().includes(query)) return [];
      visible.sort((a, b) => Number(b.isDefault) - Number(a.isDefault));
      return [{ spec, count: rows.length, rows: visible }];
    });
  }, [gatewayName, maskAccountText, providers, savedOrder, search, workspace]);

  const privacyLabel = t(privacyModeEnabled ? 'privacy.showSensitive' : 'privacy.hideSensitive');
  const refreshLabel = t('common.shared.refreshAll');
  const launchLabel = t(t3Running ? 'instances.actions.restart' : 'instances.actions.start');
  const defaultLabel = t('t3code.providerWorkspace.builtinTag');
  const LaunchIcon = restarting ? RefreshCw : t3Running ? RotateCw : Play;

  return (
    <>
      <div className="toolbar t3code-overview-toolbar">
        <div className="toolbar-left">
          {instanceOptions?.length > 1 && (
            <SingleSelectDropdown
              className="t3code-instance-select"
              value={targetId}
              options={instanceOptions}
              disabled={busy || refreshingAll}
              ariaLabel={t('t3code.providerWorkspace.targetInstance')}
              onChange={onSelectTarget}
            />
          )}
          <div className="search-box">
            <Search size={16} className="search-icon" />
            <input
              type="search"
              aria-label={t('t3code.overview.searchProvidersPlaceholder')}
              placeholder={t('t3code.overview.searchProvidersPlaceholder')}
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
          </div>
          {workspace && (
            <span className={`pill t3code-runtime-pill ${t3Running ? 'pill-success' : 'pill-secondary'}`}>
              <span className="t3code-runtime-dot" aria-hidden="true" />
              {t(t3Running ? 'instances.status.running' : 'instances.status.stopped')}
              {t3Running && injectionConnected && ` · ${t('t3code.providerWorkspace.injectionConnected')}`}
            </span>
          )}
        </div>
        <div className="toolbar-right">
          <button
            type="button"
            className="btn btn-primary btn-sm t3code-overview-launch"
            disabled={busy || !workspace}
            aria-busy={restarting}
            onClick={onStartOrRestart}
          >
            <LaunchIcon size={14} className={restarting ? 'loading-spinner' : undefined} aria-hidden="true" />
            {launchLabel}
          </button>
          <button type="button" className="btn btn-secondary icon-only t3code-toolbar-icon-btn"
            disabled={loading || refreshingAll || busy} title={refreshLabel} aria-label={refreshLabel} onClick={onRefreshAll}>
            <RefreshCw size={14} className={refreshingAll ? 'loading-spinner' : undefined} />
          </button>
          <button type="button" className="btn btn-secondary icon-only t3code-toolbar-icon-btn"
            onClick={onTogglePrivacyMode} title={privacyLabel} aria-label={privacyLabel}>
            {privacyModeEnabled ? <EyeOff size={14} /> : <Eye size={14} />}
          </button>
          <QuickSettingsPopover type="t3code" />
        </div>
      </div>

      <T3codeCommonSettings />

      {loading && !workspace && (
        <div className="loading-container" role="status">
          <RefreshCw size={20} className="loading-spinner" />
          <p>{t('common.loading')}</p>
        </div>
      )}
      {workspace && groups.length === 0 && (
        <div className="empty-state">
          <h3>{t('common.shared.noMatch.title')}</h3>
        </div>
      )}
      {groups.length > 0 && (
        <div className="t3code-overview-list">
          <div className="t3code-overview-columns t3code-overview-aligned" aria-hidden="true">
            <span>{t('t3code.providerWorkspace.platformAccount')}</span>
            <span>{t('t3code.providerWorkspace.quotaLabel')}</span>
            <span>{t('t3code.providerWorkspace.runtimeStatus')}</span>
            <span>{t('t3code.overview.actions')}</span>
          </div>
          {groups.map(({ spec, count, rows }) => (
            <section
              key={spec.driver}
              className={`t3code-overview-group ${rows.length > 0 ? 'has-accounts' : 'is-empty'}`}
              aria-label={spec.label}
            >
              <header className="t3code-overview-group-header">
                <h3 className="t3code-overview-heading">
                  <span className="t3code-group-icon" aria-hidden="true">{renderPlatformIcon(spec.platformId, 16)}</span>
                  <span className="t3code-overview-platform-name" title={spec.label}>{spec.label}</span>
                  <span className={`t3code-overview-count${count > 0 ? ' is-active' : ''}`}>{count}</span>
                </h3>
                <button type="button" className="btn btn-secondary btn-sm t3code-overview-action t3code-overview-add-btn"
                  disabled={busy} onClick={() => onAdd(spec.driver)}>
                  <Plus size={14} />
                  {t('t3code.providerWorkspace.addAccount')}
                </button>
              </header>
              {rows.length === 0 && (
                <p className="t3code-overview-empty">{t('t3code.overview.noLinkedAccounts')}</p>
              )}
              {rows.map(({ instance, status, primary, secondary, isDefault }) => {
                const pending = busyInstanceId === instance.instanceId;
                const setDefaultLabel = t('t3code.overview.setDefaultAccount');
                const removeLabel = t('t3code.overview.removeAccount');
                const switchLabel = t('t3code.providerWorkspace.switchAccount');
                const supportsSwitch = instance.driver === 'qoder' || instance.driver === 'qoder_cn';
                return (
                  <div
                    key={instance.instanceId}
                    className={`t3code-overview-entry t3code-overview-aligned${isDefault ? ' is-default' : ''}`}
                  >
                    <div className="t3code-overview-account">
                      <div className="t3code-overview-account-heading">
                        <span className="t3code-overview-account-name" title={primary}>{primary}</span>
                        <span className="t3code-overview-account-plan">
                          {renderPlanBadge(instance.driver, instance.boundAccountId, instance.boundAccountPlan, instance.boundIsApiService)}
                        </span>
                      </div>
                      {secondary && <span className="t3code-overview-account-email" title={secondary}>{secondary}</span>}
                    </div>
                    <T3codeQuotaSection
                      driver={instance.driver}
                      windows={instance.windows}
                      quotaSummaryGroups={instance.quotaSummaryGroups}
                      providerQuota={instance.providerQuota}
                      allowQuotaOverflow={instance.boundIsApiService}
                      compact
                      emptyLabel={t('t3code.quotaQuery.empty')}
                    />
                    <div className="t3code-overview-runtime">
                      <span className={`t3code-overview-entry-status t3code-status-pill is-${status.tone}`} title={status.title || status.text}>
                        {status.tone === 'ready' && <CheckCircle2 size={13} aria-hidden="true" />}
                        <span>{status.text}</span>
                      </span>
                      {status.pathTarget && (
                        <button type="button" className="btn btn-secondary btn-sm t3code-overview-action"
                          disabled={busy} onClick={() => onConfigureRuntime(instance)}>
                          {t('appPath.missing.configurePath')}
                        </button>
                      )}
                    </div>
                    <div className="t3code-overview-entry-actions">
                      {isDefault ? (
                        <span className="tag-pill t3code-overview-default-tag" title={defaultLabel}>{defaultLabel}</span>
                      ) : instance.boundAccountId && (
                        <button type="button" className="btn btn-secondary btn-sm t3code-overview-action t3code-overview-default-btn"
                          disabled={busy} aria-busy={pending} title={t('t3code.overview.defaultAffectsExisting')}
                          aria-label={`${setDefaultLabel} · ${primary}`} onClick={() => onSetDefault(instance)}>
                          {pending && <RefreshCw size={13} className="loading-spinner" />}
                          {setDefaultLabel}
                        </button>
                      )}
                      {supportsSwitch && (
                        <button type="button" className="btn btn-secondary btn-sm t3code-overview-action t3code-overview-switch"
                          disabled={busy} aria-label={`${switchLabel} · ${primary}`} onClick={() => onSwitch(instance)}>
                          {switchLabel}
                        </button>
                      )}
                      <button type="button" className="btn btn-ghost btn-sm t3code-overview-action t3code-overview-remove"
                        disabled={busy} aria-label={`${removeLabel} · ${primary}`} onClick={() => onRemove(instance)}>
                        {removeLabel}
                      </button>
                    </div>
                  </div>
                );
              })}
            </section>
          ))}
        </div>
      )}
    </>
  );
}
