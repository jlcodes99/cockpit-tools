import type { ComponentType } from 'react';
import { Boxes, FlaskConical, Network, ScrollText, Settings, UserRound, type LucideIcon } from 'lucide-react';
import type { CodexProxySectionId } from './CodexProxyWorkspaceContext';
import { CodexProxyAccountsPage } from './CodexProxyAccountsPage';
import { CodexProxyConnectionsSection } from './CodexProxyConnectionsSection';
import { CodexProxyResourcesSection } from './CodexProxyResourcesSection';
import { CodexProxyLogsSection } from './CodexProxyLogsSection';
import { CodexProxyTestsSection } from './CodexProxyTestsSection';
import { CodexProxySettingsSection } from './CodexProxySettingsSection';

/** Only the two everyday tasks are primary navigation. Diagnostics are opened on demand. */
export const PROXY_SECTION_IDS = ['resources', 'accounts'] as const;
export const PROXY_DIAGNOSTIC_IDS = ['connections', 'logs', 'tests', 'settings'] as const;
export const PROXY_SECTION_LABEL_KEYS: Record<CodexProxySectionId, string> = {
  overview: 'codex.proxy.manager.proxies',
  resources: 'codex.proxy.manager.proxies',
  accounts: 'codex.proxy.manager.accounts',
  rules: 'codex.proxy.manager.accounts',
  connections: 'codex.proxy.workspace.connections',
  logs: 'codex.proxy.workspace.logs',
  tests: 'codex.proxy.workspace.tests',
  settings: 'codex.proxy.workspace.settings',
};
export const PROXY_SECTION_ICONS: Record<CodexProxySectionId, LucideIcon> = {
  overview: Boxes, resources: Boxes, accounts: UserRound, rules: UserRound,
  connections: Network, logs: ScrollText, tests: FlaskConical, settings: Settings,
};
export const PROXY_SECTION_COMPONENTS: Record<CodexProxySectionId, ComponentType> = {
  overview: CodexProxyResourcesSection, resources: CodexProxyResourcesSection,
  accounts: CodexProxyAccountsPage, rules: CodexProxyAccountsPage,
  connections: CodexProxyConnectionsSection, logs: CodexProxyLogsSection,
  tests: CodexProxyTestsSection, settings: CodexProxySettingsSection,
};
