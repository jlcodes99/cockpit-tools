import { useTranslation } from 'react-i18next';
import { CodexProxyAccountsSection } from './CodexProxyAccountsSection';
import { CodexUnifiedProxyPanel } from './CodexUnifiedProxyPanel';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';

/** One default for all eligible accounts, followed by explicit account overrides. */
export function CodexProxyAccountsPage() {
  const { t } = useTranslation();
  const { accounts, catalog, acceptCatalog, unified, unifiedErrorKey, reloadUnified, acceptUnified, goSection } = useCodexProxyWorkspace();
  return <div className="codex-proxy-manager-accounts">
    {unifiedErrorKey ? <div className="codex-proxy-page-error" role="alert">
      {t(unifiedErrorKey)}
      <button type="button" className="btn btn-secondary compact" onClick={reloadUnified}>{t('common.retry')}</button>
    </div> : <CodexUnifiedProxyPanel totalAccounts={accounts.length} catalog={catalog} view={unified} onCatalogChange={acceptCatalog}
      onViewChange={acceptUnified} onGoResources={() => goSection('resources')} />}
    <CodexProxyAccountsSection />
  </div>;
}
