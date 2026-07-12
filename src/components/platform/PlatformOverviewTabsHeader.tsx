import { ReactNode, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Clock3, FolderOpen, Github, Layers, Server } from 'lucide-react';
import { CodexIcon } from '../icons/CodexIcon';
import { ClaudeIcon } from '../icons/ClaudeIcon';
import { WindsurfIcon } from '../icons/WindsurfIcon';
import { KiroIcon } from '../icons/KiroIcon';
import { CursorIcon } from '../icons/CursorIcon';
import { GeminiIcon } from '../icons/GeminiIcon';
import { GrokIcon } from '../icons/GrokIcon';
import { CodebuddyIcon } from '../icons/CodebuddyIcon';
import { QoderIcon } from '../icons/QoderIcon';
import { TraeCnIcon, TraeIcon, TraeSoloCnIcon, TraeSoloIcon } from '../icons/TraeIcon';
import { WorkbuddyIcon } from '../icons/WorkbuddyIcon';
import { ZedIcon } from '../icons/ZedIcon';
import { ZcodeIcon } from '../icons/ZcodeIcon';
import { ManualHelpIconButton } from '../ManualHelpIconButton';
import { PlatformId } from '../../types/platform';
import {
  findGroupByPlatform,
  resolveGroupChildName,
  usePlatformLayoutStore,
} from '../../stores/usePlatformLayoutStore';
import { getPlatformLabel } from '../../utils/platformMeta';
import { PlatformGroupSwitcher } from './PlatformGroupSwitcher';
import { useRemoteConfigStore } from '../../stores/useRemoteConfigStore';

export type PlatformOverviewTab = 'overview' | 'wakeup' | 'instances' | 'sessions' | 'providers';
export type PlatformOverviewHeaderId =
  | 'codex'
  | 'claude'
  | 'zed'
  | 'github-copilot'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'gemini'
  | 'grok'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'zcode'
  | 'trae'
  | 'trae_solo'
  | 'trae_cn'
  | 'trae_solo_cn'
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
  claude: {
    platformLabel: 'Claude',
    overviewIcon: <ClaudeIcon className="tab-icon" />,
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
  grok: {
    platformLabel: 'Grok CLI',
    overviewIcon: <GrokIcon className="tab-icon" />,
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
  zcode: {
    platformLabel: 'ZCode',
    overviewIcon: <ZcodeIcon className="tab-icon" />,
  },
  trae: {
    platformLabel: 'Trae',
    overviewIcon: <TraeIcon className="tab-icon" />,
  },
  trae_solo: {
    platformLabel: 'TRAE SOLO',
    overviewIcon: <TraeSoloIcon className="tab-icon" />,
  },
  trae_cn: {
    platformLabel: 'Trae CN',
    overviewIcon: <TraeCnIcon className="tab-icon" />,
  },
  trae_solo_cn: {
    platformLabel: 'TRAE SOLO CN',
    overviewIcon: <TraeSoloCnIcon className="tab-icon" />,
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
  const remoteHiddenPlatformIds = useRemoteConfigStore((state) => state.hiddenPlatformIds);
  const config = CONFIGS[platform];
  const currentPlatformId = platform as PlatformId;
  const remoteHiddenPlatformSet = useMemo(
    () => new Set(remoteHiddenPlatformIds),
    [remoteHiddenPlatformIds],
  );
  const currentGroup = useMemo(
    () => findGroupByPlatform(platformGroups, currentPlatformId),
    [platformGroups, currentPlatformId],
  );
  const switchablePlatforms = useMemo(
    () => {
      const source = currentGroup ? currentGroup.platformIds : [currentPlatformId];
      const visible = source.filter((platformId) =>
        platformId === currentPlatformId || !remoteHiddenPlatformSet.has(platformId),
      );
      return visible.length > 0 ? visible : [currentPlatformId];
    },
    [currentGroup, currentPlatformId, remoteHiddenPlatformSet],
  );
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
  const extraSwitchOptions = useMemo(
    () =>
      platform === 'codex'
        ? [
            {
              id: 'codex-api-service',
              label: t('codex.apiService.navTitle', 'Codex API 服务'),
              page: 'codex-api-service' as const,
              icon: <CodexIcon size={18} />,
            },
          ]
        : [],
    [platform, t],
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
      label: t('instances.title', '应用多开'),
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
        <div className="page-top-strip-right-placeholder" aria-hidden="true" />
      </div>
      <div className="page-tabs-row page-tabs-center page-tabs-row-with-leading">
        <div className="page-tabs-leading">
          <PlatformGroupSwitcher
            currentPlatformId={currentPlatformId}
            currentLabel={currentDisplayName}
            options={switchOptions}
            currentGroupId={currentGroup?.id ?? null}
            extraOptions={extraSwitchOptions}
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
