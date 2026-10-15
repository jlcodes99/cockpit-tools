import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CodexProxyResources } from './CodexProxyResources';
import { CodexProxyAssignDialog, type ProxyAssignment } from './CodexProxyAssignDialog';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';
import { refreshCodexProxyAccounts } from '../../utils/codexProxyRemoval';
import '../../styles/pages/codex-proxy-resources-page.css';

/** Adding, selecting and assigning stay in the same resource list. */
export function CodexProxyResourcesSection() {
  const { t } = useTranslation();
  const { reloadCatalog, reloadUnified } = useCodexProxyWorkspace();
  const [assignment, setAssignment] = useState<ProxyAssignment | null>(null);
  return <section className="codex-proxy-resources-page" aria-label={t('codex.proxy.manager.proxies')}>
    <CodexProxyResources
      onAssign={(sourceId, itemId, selections, groupId, catalog) => setAssignment({ sourceId, itemId, groupId, selections, catalog })}
      onBindingsChanged={async () => {
        try { await refreshCodexProxyAccounts(); }
        finally { reloadCatalog(); reloadUnified(); }
      }}
    />
    {assignment && <CodexProxyAssignDialog choice={assignment} onClose={() => setAssignment(null)} />}
  </section>;
}
