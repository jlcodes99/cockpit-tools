import { ReactNode, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Bot, Clock3, FolderOpen, Github, Layers, Server } from 'lucide-react';
import { CodexIcon } from '../icons/CodexIcon';
import { WindsurfIcon } from '../icons/WindsurfIcon';
import { KiroIcon } from '../icons/KiroIcon';
import { CursorIcon } from '../icons/CursorIcon';
import { GeminiIcon } from '../icons/GeminiIcon';
import { CodebuddyIcon } from '../icons/CodebuddyIcon';
import { QoderIcon } from '../icons/QoderIcon';
import { WorkbuddyIcon } from '../icons/WorkbuddyIcon';
import { ZedIcon } from '../icons/ZedIcon';
import { ManualHelpIconButton } from '../ManualHelpIconButton';
import { TopCenterPromoBanner } from '../TopCenterPromoBanner';
import { PlatformId } from '../../types/platform';
import {
  findGroupByPlatform,
  resolveGroupChildName,
  usePlatformLayoutStore,
} from '../../stores/usePlatformLayoutStore';
import { getPlatformLabel } from '../../utils/platformMeta';
import { PlatformGroupSwitcher } from './PlatformGroupSwitcher';

export type PlatformOverviewTab = 'overview' | 'wakeup' | 'instances' | 'sessions' | 'providers';
export type PlatformOverviewHeaderId =
  | 'codex'
  | 'zed'
  | 'github-copilot'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'gemini'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'trae'
  | 'workbuddy';

interface PlatformOverviewTabsHeaderProps {
  platform: PlatformOverviewHeaderId;
  active: PlatformOverviewTab;
  onTabChange?: (tab: PlatformOverviewTab) => void;
  tabs?: PlatformOverviewTab[];
}

interface PlatformOverviewConfig {
  platformLabel: string;
  overviewIcon: ReactNode;
}

interface TabSpec {
  key: PlatformOverviewTab;
  label: string;
  icon: ReactNode;
}

const CONFIGS: Record<PlatformOverviewHeaderId, PlatformOverviewConfig> = {
  codex: {
    platformLabel: 'Codex',
    overviewIcon: <CodexIcon className="tab-icon" />,
  },
  zed: {
    platformLabel: 'Zed',
    overviewIcon: <ZedIcon className="tab-icon" />,
  },
  'github-copilot': {
    platformLabel: 'GitHub Copilot',
    overviewIcon: <Github className="tab-icon" />,
  },
  windsurf: {
    platformLabel: 'Windsurf',
    overviewIcon: <WindsurfIcon className="tab-icon" />,
  },
  kiro: {
    platformLabel: 'Kiro',
    overviewIcon: <KiroIcon className="tab-icon" />,
  },
  cursor: {
    platformLabel: 'Cursor',
    overviewIcon: <CursorIcon className="tab-icon" />,
  },
  gemini: {
    platformLabel: 'Gemini Cli',
    overviewIcon: <GeminiIcon className="tab-icon" />,
  },
  codebuddy: {
    platformLabel: 'CodeBuddy',
    overviewIcon: <CodebuddyIcon className="tab-icon" />,
  },
  codebuddy_cn: {
    platformLabel: 'CodeBuddy CN',
    overviewIcon: <CodebuddyIcon className="tab-icon" />,
  },
  qoder: {
    platformLabel: 'Qoder',
    overviewIcon: <QoderIcon className="tab-icon" />,
  },
  trae: {
    platformLabel: 'Trae',
    overviewIcon: <Bot className="tab-icon" />,
  },
  workbuddy: {
    platformLabel: 'WorkBuddy',
    overviewIcon: <WorkbuddyIcon className="tab-icon" />,
  },
};

export function PlatformOverviewTabsHeader({
  platform,
  active,
  onTabChange,
  tabs,
}: PlatformOverviewTabsHeaderProps) {
  const { t } = useTranslation();
  const { platformGroups } = usePlatformLayoutStore();
  const config = CONFIGS[platform];
  const currentPlatformId = platform as PlatformId;
  const currentGroup = useMemo(
    () => findGroupByPlatform(platformGroups, currentPlatformId),
    [platformGroups, currentPlatformId],
  );
  const switchablePlatforms = currentGroup ? currentGroup.platformIds : [currentPlatformId];
  const currentPlatformLabel = getPlatformLabel(currentPlatformId, t);
  const currentDisplayName = useMemo(
    () =>
      currentGroup
        ? resolveGroupChildName(currentGroup, currentPlatformId, currentPlatformLabel || config.platformLabel)
        : currentPlatformLabel || config.platformLabel,
    [currentGroup, currentPlatformId, currentPlatformLabel, config.platformLabel],
  );
  const switchOptions = useMemo(
    () =>
      switchablePlatforms.map((platformId) => {
        const platformName = currentGroup
          ? resolveGroupChildName(currentGroup, platformId, getPlatformLabel(platformId, t))
          : getPlatformLabel(platformId, t);
        return {
          platformId,
          label: platformName,
        };
      }),
    [switchablePlatforms, currentGroup, t],
  );
  const tabOrder: PlatformOverviewTab[] =
    tabs && tabs.length > 0 ? tabs : ['overview', 'instances'];
  const tabLabels: Record<PlatformOverviewTab, TabSpec> = {
    overview: {
      key: 'overview',
      label: t('overview.title', '账号总览'),
      icon: config.overviewIcon,
    },
    wakeup: {
      key: 'wakeup',
      label:
        platform === 'codex'
          ? t('codex.wakeup.tab', '唤醒任务')
          : t('wakeup.title', '唤醒任务'),
      icon: <Clock3 className="tab-icon" />,
    },
    instances: {
      key: 'instances',
      label: t('instances.title', '多开实例'),
      icon: <Layers className="tab-icon" />,
    },
    sessions: {
      key: 'sessions',
      label: t('codex.sessionManager.title', '会话管理'),
      icon: <FolderOpen className="tab-icon" />,
    },
    providers: {
      key: 'providers',
      label: t('codex.modelProviders.tab', '模型供应商'),
      icon: <Server className="tab-icon" />,
    },
  };
  const tabSpecs: TabSpec[] = tabOrder.map((tab) => tabLabels[tab]);

  return (
    <>
      <div className="page-top-strip">
        <div className="page-top-strip-left">
          <span className="page-top-strip-label">
            {t('settings.general.account', '账号')}
          </span>
          <ManualHelpIconButton className="platform-header-help" />
        </div>
        <TopCenterPromoBanner />
        <div className="page-top-strip-right-placeholder" aria-hidden="true" />
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
          {tabSpecs.map((tab) => (
            <button
              key={tab.key}
              className={`filter-tab${active === tab.key ? ' active' : ''}`}
              onClick={() => onTabChange?.(tab.key)}
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
