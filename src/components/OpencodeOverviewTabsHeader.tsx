import { PlatformOverviewTabsHeader, PlatformOverviewTab } from './platform/PlatformOverviewTabsHeader';

export type OpencodeTab = PlatformOverviewTab;

interface OpencodeOverviewTabsHeaderProps {
  active: OpencodeTab;
  onTabChange?: (tab: OpencodeTab) => void;
}

export function OpencodeOverviewTabsHeader({
  active,
  onTabChange,
}: OpencodeOverviewTabsHeaderProps) {
  return (
    <PlatformOverviewTabsHeader platform="opencode" active={active} onTabChange={onTabChange} />
  );
}
