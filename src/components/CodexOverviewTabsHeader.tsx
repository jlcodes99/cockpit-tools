import { useEffect } from 'react';
import { PlatformOverviewTabsHeader, PlatformOverviewTab } from './platform/PlatformOverviewTabsHeader';
import { CODEX_SUITE_ENSURE_MOUNTED_EVENT } from '../utils/codexAddAccountRequest';

export type CodexTab = PlatformOverviewTab;

interface CodexOverviewTabsHeaderProps {
  active: CodexTab;
  onTabChange?: (tab: CodexTab) => void;
  tabs?: CodexTab[];
}

export function CodexOverviewTabsHeader({
  active,
  onTabChange,
  tabs,
}: CodexOverviewTabsHeaderProps) {
  useEffect(() => {
    if (!onTabChange) return;
    const showSharedAccountModal = () => onTabChange('overview');
    window.addEventListener(CODEX_SUITE_ENSURE_MOUNTED_EVENT, showSharedAccountModal);
    return () => {
      window.removeEventListener(CODEX_SUITE_ENSURE_MOUNTED_EVENT, showSharedAccountModal);
    };
  }, [onTabChange]);

  return (
    <PlatformOverviewTabsHeader
      platform="codex"
      active={active}
      onTabChange={onTabChange}
      tabs={tabs}
    />
  );
}
