import { ReactNode, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { AlarmClock, Fingerprint, Layers, ShieldCheck } from 'lucide-react';
import { Page } from '../types/navigation';
import { RobotIcon } from './icons/RobotIcon';
import { ManualHelpIconButton } from './ManualHelpIconButton';
import { TopCenterPromoBanner } from './TopCenterPromoBanner';
import { AntigravityInstalledVersionBadge } from './AntigravityInstalledVersionBadge';
import { PlatformId } from '../types/platform';
import {
  findGroupByPlatform,
  resolveGroupChildName,
  usePlatformLayoutStore,
} from '../stores/usePlatformLayoutStore';
import { getPlatformLabel } from '../utils/platformMeta';
import { PlatformGroupSwitcher } from './platform/PlatformGroupSwitcher';
import { useAntigravityRuntimeTarget } from '../hooks/useAntigravityRuntimeTarget';

interface OverviewTabsHeaderProps {
  active: Page;
  onNavigate?: (page: Page) => void;
  subtitle: string;
  title?: string;
  onOpenManual?: () => void;
}

interface TabSpec {
  key: Page;
  label: string;
  icon: ReactNode;
}

export function OverviewTabsHeader({
  active,
  onNavigate,
  subtitle,
  title,
  onOpenManual,
}: OverviewTabsHeaderProps) {
  void subtitle;
  const { t } = useTranslation();
  const { platformGroups } = usePlatformLayoutStore();
  const currentPlatformId: PlatformId = useAntigravityRuntimeTarget();
  const currentGroup = useMemo(
    () => findGroupByPlatform(platformGroups, currentPlatformId),
    [platformGroups, currentPlatformId],
  );
  const switchablePlatforms = currentGroup ? currentGroup.platformIds : [currentPlatformId];
  const currentPlatformLabel = getPlatformLabel(currentPlatformId, t);
  const currentDisplayName = useMemo(
    () =>
      title
        ? title
        : currentGroup
          ? resolveGroupChildName(currentGroup, currentPlatformId, currentPlatformLabel)
          : currentPlatformLabel,
    [title, currentGroup, currentPlatformId, currentPlatformLabel],
  );
  const switchOptions = useMemo(
    () =>
      switchablePlatforms.map((platformId) => ({
        platformId,
        label: currentGroup
          ? resolveGroupChildName(currentGroup, platformId, getPlatformLabel(platformId, t))
          : getPlatformLabel(platformId, t),
      })),
    [switchablePlatforms, currentGroup, t],
  );
  const tabs: TabSpec[] = [
    {
      key: 'overview',
      label: t('overview.title'),
      icon: <RobotIcon className="tab-icon" />,
    },
    {
      key: 'instances',
      label: t('instances.title', '多开实例'),
      icon: <Layers className="tab-icon" />,
    },
    {
      key: 'fingerprints',
      label: t('fingerprints.title'),
      icon: <Fingerprint className="tab-icon" />,
    },
    {
      key: 'wakeup',
      label: t('wakeup.title'),
      icon: <AlarmClock className="tab-icon" />,
    },
    {
      key: 'verification',
      label: t('wakeup.verification.title'),
      icon: <ShieldCheck className="tab-icon" />,
    },
  ];

  return (
    <>
      <div className="page-top-strip">
        <div className="page-top-strip-left">
          <span className="page-top-strip-label">
            {t('settings.general.account', '账号')}
          </span>
          <ManualHelpIconButton className="platform-header-help" onClick={onOpenManual} />
        </div>
        <TopCenterPromoBanner />
        <div className="page-top-strip-right">
          <AntigravityInstalledVersionBadge />
        </div>
      </div>
      <div className="page-tabs-row page-tabs-center page-tabs-row-with-leading">
        <div className="page-tabs-leading">
          <PlatformGroupSwitcher
            currentPlatformId={currentPlatformId}
            currentLabel={currentDisplayName}
            options={switchOptions}
            currentGroupId={currentGroup?.id ?? null}
          />
        </div>
        <div className="page-tabs filter-tabs">
          {tabs.map((tab) => (
            <button
              key={tab.key}
              className={`filter-tab${active === tab.key ? ' active' : ''}`}
              onClick={() => onNavigate?.(tab.key)}
            >
              {tab.icon}
              <span>{tab.label}</span>
            </button>
          ))}
        </div>
      </div>
    </>
  );
}
