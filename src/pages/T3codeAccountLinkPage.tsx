import { useState } from 'react';
import { AlertCircle, ChevronDown, CircleAlert, RefreshCw, RotateCw, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { PlatformOverviewTabsHeader } from '../components/platform/PlatformOverviewTabsHeader';
import { T3codeModals, type T3codeModalState } from '../components/t3code/T3codeModals';
import { T3codeOverviewSection } from '../components/t3code/T3codeOverviewSection';
import { useT3codeWorkspaceController } from '../components/t3code/useT3codeWorkspaceController';
import type { Page } from '../types/navigation';
import './T3codeAccountLinkPage.css';

interface T3codeAccountLinkPageProps {
  onNavigate?: (page: Page) => void;
}

export function T3codeAccountLinkPage({ onNavigate }: T3codeAccountLinkPageProps = {}) {
  const { t } = useTranslation();
  const controller = useT3codeWorkspaceController();
  const [modal, setModal] = useState<T3codeModalState | null>(null);

  return (
    <div className="ghcp-accounts-page t3code-accounts-page">
      <PlatformOverviewTabsHeader platform="t3code" active="overview" tabs={['overview']} />
      <div
        className={`ghcp-flow-notice ${controller.isFlowNoticeCollapsed ? 'collapsed' : ''}`}
        role="note"
        aria-live="polite"
      >
        <button
          type="button"
          className="ghcp-flow-notice-toggle"
          onClick={() => controller.setIsFlowNoticeCollapsed((prev) => !prev)}
          aria-expanded={!controller.isFlowNoticeCollapsed}
        >
          <div className="ghcp-flow-notice-title">
            <CircleAlert size={16} />
            <span>{t('t3code.flowNotice.title')}</span>
          </div>
          <ChevronDown
            size={16}
            className={`ghcp-flow-notice-arrow ${controller.isFlowNoticeCollapsed ? 'collapsed' : ''}`}
          />
        </button>
        {!controller.isFlowNoticeCollapsed && (
          <div className="ghcp-flow-notice-body">
            <div className="ghcp-flow-notice-desc">{t('t3code.flowNotice.desc')}</div>
            <ul className="ghcp-flow-notice-list">
              <li>{t('t3code.flowNotice.permission')}</li>
              <li>{t('t3code.flowNotice.network')}</li>
            </ul>
          </div>
        )}
      </div>

      {controller.showRestartBanner && (
        <div className="t3code-restart-banner" role="status">
          <div className="t3code-restart-banner-text">
            <AlertCircle size={15} />
            <span>{t('t3code.providerWorkspace.needsRestartNotice')}</span>
          </div>
          <button
            className="btn btn-primary btn-sm"
            type="button"
            disabled={
              controller.restarting ||
              Boolean(controller.busyInstanceId) ||
              controller.loading
            }
            onClick={() => void controller.handleStartOrRestart()}
          >
            {controller.restarting ? (
              <RefreshCw size={14} className="loading-spinner" />
            ) : (
              <RotateCw size={14} />
            )}
            {controller.restarting
              ? t('instances.status.restarting')
              : t('t3code.quotaQuery.restartToApply')}
          </button>
        </div>
      )}

      {controller.loadError && (
        <div className="message-bar error" role="alert">
          <span>{controller.loadError}</span>
          <button
            type="button"
            onClick={() => void controller.loadWorkspace()}
            title={t('common.retry')}
          >
            <RefreshCw size={14} />
          </button>
        </div>
      )}

      {controller.message && (
        <div
          className={`message-bar ${controller.message.tone === 'error' ? 'error' : 'success'}`}
          role={controller.message.tone === 'error' ? 'alert' : 'status'}
        >
          <span>{controller.message.text}</span>
          <button
            type="button"
            onClick={() => controller.setMessage(null)}
            aria-label={t('common.close')}
          >
            <X size={14} />
          </button>
        </div>
      )}

      <T3codeOverviewSection
        workspace={controller.activeWorkspace}
        providers={controller.providers}
        t3Running={controller.t3Running}
        injectionConnected={controller.injectionConnected}
        loading={controller.loading}
        busyInstanceId={controller.busyInstanceId}
        refreshingAll={controller.refreshingAll}
        restarting={controller.restarting}
        gatewayName={controller.gatewayName}
        privacyModeEnabled={controller.privacyModeEnabled}
        onTogglePrivacyMode={controller.togglePrivacyMode}
        maskAccountText={controller.maskAccountText}
        renderPlanBadge={controller.renderPlanBadge}
        onAdd={(driver) => setModal({ kind: 'add', driver })}
        onSetDefault={(instance) => void controller.promoteProviderAccount(instance)}
        onRemove={(instance) => setModal({ kind: 'remove', instance })}
        onConfigureRuntime={(instance) => setModal({ kind: 'runtime', instance })}
        onRefreshAll={() => void controller.handleRefreshAll()}
        onStartOrRestart={() => void controller.handleStartOrRestart()}
      />

      {modal && (
        <T3codeModals
          modal={modal}
          workspace={controller.activeWorkspace}
          gatewayName={controller.gatewayName}
          maskAccountText={controller.maskAccountText}
          renderPlanBadge={controller.renderPlanBadge}
          onClose={() => setModal(null)}
          onAdd={controller.addProviderAccount}
          onRemove={controller.removeProviderInstance}
          onNavigate={onNavigate}
        />
      )}
    </div>
  );
}
