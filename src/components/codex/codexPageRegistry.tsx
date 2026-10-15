import type { ReactNode } from 'react';
import type { TFunction } from 'i18next';
import { Clock3, FolderOpen, Globe2, Layers, PanelTop, Server } from 'lucide-react';
import type { CodexTab } from '../CodexOverviewTabsHeader';
import { CodexIcon } from '../icons/CodexIcon';

export interface CodexRegisteredPage { id: CodexTab; label: string; icon: ReactNode }
/** Header, More, layout and render guard consume this one live page registry. */
export function codexPageRegistry(t: TFunction): CodexRegisteredPage[] {
  return [
    { id: 'overview', label: t('overview.title'), icon: <CodexIcon className="tab-icon" /> },
    { id: 'providers', label: t('codex.modelProviders.tab'), icon: <Server className="tab-icon" /> },
    { id: 'wakeup', label: t('codex.wakeup.tab'), icon: <Clock3 className="tab-icon" /> },
    { id: 'instances', label: t('instances.title'), icon: <Layers className="tab-icon" /> },
    { id: 'sessions', label: t('codex.sessionManager.title'), icon: <FolderOpen className="tab-icon" /> },
    { id: 'proxy', label: t('codex.proxy.management'), icon: <Globe2 className="tab-icon" /> },
    { id: 'top-layout', label: t('codex.more.topLayoutTitle'), icon: <PanelTop className="tab-icon" /> },
  ];
}
