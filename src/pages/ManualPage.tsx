import { useMemo, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import {
  BookOpen,
  ChevronDown,
  Compass,
  LayoutGrid,
  Lightbulb,
  Rocket,
  Search,
  Settings,
  Sparkles,
} from 'lucide-react';
import type { Page } from '../types/navigation';

type ManualAction =
  | { id: string; kind: 'navigate'; page: Page; label: string; primary?: boolean }
  | { id: string; kind: 'layout'; label: string; primary?: boolean };

interface ManualSection {
  id: string;
  icon: ReactNode;
  title: string;
  summary: string;
  outcomes: string[];
  steps: string[];
  cautions: string[];
  keywords: string[];
  actions: ManualAction[];
}

interface ManualPageProps {
  onNavigate: (page: Page) => void;
  onOpenPlatformLayout: () => void;
}

function normalizeSearchText(text: string): string {
  return text.trim().toLowerCase();
}

export function ManualPage({ onNavigate, onOpenPlatformLayout }: ManualPageProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState('');
  const [expandedIds, setExpandedIds] = useState<Set<string>>(
    () => new Set(['quick-start', 'instances', 'settings']),
  );

  const sections = useMemo<ManualSection[]>(
    () => [
      {
        id: 'quick-start',
        icon: <Rocket size={18} />,
        title: t('manual.quickStart.title', '快速开始（5 分钟）'),
        summary: t(
          'manual.quickStart.summary',
          '按“看状态 -> 加账号 -> 切账号 -> 开多实例”的顺序完成首轮上手，避免迷路。',
        ),
        outcomes: [
          t('manual.quickStart.outcomes.0', '先在仪表盘看全局状态，再进入具体平台页操作。'),
          t('manual.quickStart.outcomes.1', '先完成一个平台的完整闭环，再扩展到其他平台。'),
          t('manual.quickStart.outcomes.2', '理解“账号管理”和“多开实例”是两条不同工作流。'),
        ],
        steps: [
          t('manual.quickStart.steps.0', '打开“仪表盘”，确认你需要管理的平台已经可见。'),
          t('manual.quickStart.steps.1', '进入目标平台页（如 Codex / GitHub Copilot），先添加 1 个账号。'),
          t('manual.quickStart.steps.2', '使用“切换/注入”按钮验证切号生效。'),
          t('manual.quickStart.steps.3', '再进入“多开实例”创建 1 个实例，验证隔离和并行运行。'),
        ],
        cautions: [
          t('manual.quickStart.cautions.0', '第一次上手建议只操作一个平台，先跑通流程再批量导入。'),
          t('manual.quickStart.cautions.1', '如果某平台启动路径缺失，先在设置里补路径再继续。'),
        ],
        keywords: [
          t('manual.quickStart.keywords.0', '快速开始'),
          t('manual.quickStart.keywords.1', '新手'),
          t('manual.quickStart.keywords.2', 'first run'),
          t('manual.quickStart.keywords.3', 'onboarding'),
        ],
        actions: [
          { id: 'go-dashboard', kind: 'navigate', page: 'dashboard', label: t('manual.actions.goDashboard', '前往仪表盘'), primary: true },
          { id: 'go-overview', kind: 'navigate', page: 'overview', label: t('manual.actions.goAntigravity', '前往 Antigravity IDE') },
          { id: 'go-settings', kind: 'navigate', page: 'settings', label: t('manual.actions.goSettings', '前往设置') },
        ],
      },
      {
        id: 'dashboard',
        icon: <Compass size={18} />,
        title: t('manual.dashboard.title', '仪表盘'),
        summary: t(
          'manual.dashboard.summary',
          '用于全局看板：快速查看各平台账号状态、推荐切换账号、进入具体功能页。',
        ),
        outcomes: [
          t('manual.dashboard.outcomes.0', '在一个页面看到多平台“当前账号/推荐账号/配额概览”。'),
          t('manual.dashboard.outcomes.1', '通过快捷切换按钮，减少来回切页成本。'),
        ],
        steps: [
          t('manual.dashboard.steps.0', '先看每个平台卡片是否有账号和配额数据。'),
          t('manual.dashboard.steps.1', '若状态异常，点击刷新或进入对应平台页处理。'),
          t('manual.dashboard.steps.2', '需要改平台显示顺序时，打开“平台布局”管理。'),
        ],
        cautions: [
          t('manual.dashboard.cautions.0', '仪表盘适合“看全局”，细粒度操作请进入具体平台页。'),
        ],
        keywords: [
          t('manual.dashboard.keywords.0', '仪表盘'),
          t('manual.dashboard.keywords.1', '概览'),
          t('manual.dashboard.keywords.2', '推荐切换'),
          t('manual.dashboard.keywords.3', 'dashboard'),
        ],
        actions: [
          { id: 'go-dashboard', kind: 'navigate', page: 'dashboard', label: t('manual.actions.goDashboard', '前往仪表盘'), primary: true },
          { id: 'open-layout', kind: 'layout', label: t('manual.actions.openLayout', '打开平台布局') },
        ],
      },
      {
        id: 'antigravity',
        icon: <Sparkles size={18} />,
        title: t('manual.antigravity.title', 'Antigravity IDE 账号管理'),
        summary: t(
          'manual.antigravity.summary',
          '管理 Antigravity IDE 账号生命周期：添加、刷新配额、切换、分组与标签。',
        ),
        outcomes: [
          t('manual.antigravity.outcomes.0', '支持 OAuth、导入、导出和批量操作。'),
          t('manual.antigravity.outcomes.1', '支持标签筛选、排序、分组展示。'),
        ],
        steps: [
          t('manual.antigravity.steps.0', '点击“添加账号”完成授权或导入。'),
          t('manual.antigravity.steps.1', '使用“刷新全部”先同步一轮配额状态。'),
          t('manual.antigravity.steps.2', '按配额/重置时间排序，选择当前要使用的账号。'),
        ],
        cautions: [
          t('manual.antigravity.cautions.0', '若看到异常状态（如 403/刷新失败），先刷新再判断是否删除重登。'),
        ],
        keywords: [
          t('manual.antigravity.keywords.0', 'antigravity'),
          t('manual.antigravity.keywords.1', '账号管理'),
          t('manual.antigravity.keywords.2', '配额'),
          t('manual.antigravity.keywords.3', '标签'),
        ],
        actions: [
          { id: 'go-overview', kind: 'navigate', page: 'overview', label: t('manual.actions.goAntigravity', '前往 Antigravity IDE'), primary: true },
        ],
      },
      {
        id: 'providers',
        icon: <BookOpen size={18} />,
        title: t('manual.providers.title', 'Codex / GitHub Copilot / Windsurf / Kiro'),
        summary: t(
          'manual.providers.summary',
          '四个平台页结构一致：账号总览 + 多开实例，支持 OAuth、Token/JSON 导入与切换。',
        ),
        outcomes: [
          t('manual.providers.outcomes.0', 'Codex：账号切换 + 配额刷新。'),
          t('manual.providers.outcomes.1', 'GitHub Copilot/Windsurf/Kiro：支持注入到 VS Code 链路。'),
          t('manual.providers.outcomes.2', '每个平台都可独立做筛选、分组、导入导出。'),
        ],
        steps: [
          t('manual.providers.steps.0', '先完成 OAuth 或 Token 导入。'),
          t('manual.providers.steps.1', '确认账号列表中能看到计划/配额和重置时间。'),
          t('manual.providers.steps.2', '执行“切换/注入”并在客户端验证生效。'),
        ],
        cautions: [
          t('manual.providers.cautions.0', '账号管理说明条里写了本地读写和权限范围，建议先读再操作。'),
          t('manual.providers.cautions.1', '部分平台或能力受系统限制，请按页面提示处理。'),
        ],
        keywords: [
          t('manual.providers.keywords.0', 'codex'),
          t('manual.providers.keywords.1', 'github copilot'),
          t('manual.providers.keywords.2', 'windsurf'),
          t('manual.providers.keywords.3', 'kiro'),
          t('manual.providers.keywords.4', '注入'),
          t('manual.providers.keywords.5', '切号'),
        ],
        actions: [
          { id: 'go-codex', kind: 'navigate', page: 'codex', label: t('manual.actions.goCodex', '前往 Codex'), primary: true },
          { id: 'go-ghcp', kind: 'navigate', page: 'github-copilot', label: t('manual.actions.goGitHubCopilot', '前往 GitHub Copilot') },
          { id: 'go-windsurf', kind: 'navigate', page: 'windsurf', label: t('manual.actions.goWindsurf', '前往 Windsurf') },
          { id: 'go-kiro', kind: 'navigate', page: 'kiro', label: t('manual.actions.goKiro', '前往 Kiro') },
        ],
      },
      {
        id: 'instances',
        icon: <LayoutGrid size={18} />,
        title: t('manual.instances.title', '多开实例（重点）'),
        summary: t(
          'manual.instances.summary',
          '用于账号隔离和并行运行。每个实例有独立目录、独立状态，可避免互相污染。',
        ),
        outcomes: [
          t('manual.instances.outcomes.0', '工作号/个人号环境隔离。'),
          t('manual.instances.outcomes.1', '多账号并行运行，减少反复登录切换。'),
          t('manual.instances.outcomes.2', '新配置先在测试实例验证，再回主实例。'),
        ],
        steps: [
          t('manual.instances.steps.0', '新建实例时优先选“复制来源实例”，可更快进入可用状态。'),
          t('manual.instances.steps.1', '若选“空白实例”，先启动一次初始化，再绑定账号。'),
          t('manual.instances.steps.2', '绑定账号后，通过“启动/定位窗口/停止”管理生命周期。'),
        ],
        cautions: [
          t('manual.instances.cautions.0', '复制来源实例前，建议先关闭来源实例，避免数据不一致。'),
          t('manual.instances.cautions.1', '空白实例未初始化前不能绑定账号，这是正常行为。'),
        ],
        keywords: [
          t('manual.instances.keywords.0', '多开'),
          t('manual.instances.keywords.1', '实例'),
          t('manual.instances.keywords.2', '隔离'),
          t('manual.instances.keywords.3', '并行'),
          t('manual.instances.keywords.4', '初始化'),
        ],
        actions: [
          { id: 'go-instances', kind: 'navigate', page: 'instances', label: t('manual.actions.goInstances', '前往多开实例'), primary: true },
        ],
      },
      {
        id: 'wakeup',
        icon: <Rocket size={18} />,
        title: t('manual.wakeup.title', '唤醒任务与验证'),
        summary: t(
          'manual.wakeup.summary',
          '用于定时运行唤醒任务和批量验证结果，帮助你持续跟踪账号可用性与状态。',
        ),
        outcomes: [
          t('manual.wakeup.outcomes.0', '创建周期任务并记录历史执行结果。'),
          t('manual.wakeup.outcomes.1', '用验证页批量跑检查并查看失败原因。'),
        ],
        steps: [
          t('manual.wakeup.steps.0', '在“唤醒任务”新建任务，设置模型和触发方式。'),
          t('manual.wakeup.steps.1', '先手动运行一次，确认任务参数可用。'),
          t('manual.wakeup.steps.2', '在“验证”页面做批量巡检和历史复盘。'),
        ],
        cautions: [
          t('manual.wakeup.cautions.0', '首次配置建议只选少量账号，确认稳定后再扩大范围。'),
          t('manual.wakeup.cautions.1', '验证结果异常时优先看详情里的错误与验证链接。'),
        ],
        keywords: [
          t('manual.wakeup.keywords.0', '唤醒'),
          t('manual.wakeup.keywords.1', '任务'),
          t('manual.wakeup.keywords.2', '验证'),
          t('manual.wakeup.keywords.3', '定时'),
          t('manual.wakeup.keywords.4', 'history'),
        ],
        actions: [
          { id: 'go-wakeup', kind: 'navigate', page: 'wakeup', label: t('manual.actions.goWakeup', '前往唤醒任务'), primary: true },
          { id: 'go-verification', kind: 'navigate', page: 'verification', label: t('manual.actions.goVerification', '前往唤醒验证') },
        ],
      },
      {
        id: 'settings',
        icon: <Settings size={18} />,
        title: t('manual.settings.title', '设置与系统能力'),
        summary: t(
          'manual.settings.summary',
          '集中管理语言、主题、自动刷新、告警阈值、应用路径与网络服务配置。',
        ),
        outcomes: [
          t('manual.settings.outcomes.0', '统一配置各平台自动刷新和配额告警。'),
          t('manual.settings.outcomes.1', '设置各客户端启动路径，修复“路径缺失”问题。'),
          t('manual.settings.outcomes.2', '调整窗口行为、语言和主题，适配你的使用习惯。'),
        ],
        steps: [
          t('manual.settings.steps.0', '先在“通用”页完成语言/主题/路径的基础设置。'),
          t('manual.settings.steps.1', '再按平台调自动刷新与告警阈值。'),
          t('manual.settings.steps.2', '网络服务改端口后按提示重启生效。'),
        ],
        cautions: [
          t('manual.settings.cautions.0', '路径探测失败时，手动选择可执行文件路径最稳妥。'),
          t('manual.settings.cautions.1', '阈值过低会频繁提醒，建议先从 20% 左右开始。'),
        ],
        keywords: [
          t('manual.settings.keywords.0', '设置'),
          t('manual.settings.keywords.1', '路径'),
          t('manual.settings.keywords.2', '刷新'),
          t('manual.settings.keywords.3', '阈值'),
          t('manual.settings.keywords.4', '语言'),
          t('manual.settings.keywords.5', '主题'),
        ],
        actions: [
          { id: 'go-settings', kind: 'navigate', page: 'settings', label: t('manual.actions.goSettings', '前往设置'), primary: true },
          { id: 'open-layout', kind: 'layout', label: t('manual.actions.openLayout', '打开平台布局') },
        ],
      },
      {
        id: 'data-and-privacy',
        icon: <Lightbulb size={18} />,
        title: t('manual.dataPrivacy.title', '导入导出、隐私与排障'),
        summary: t(
          'manual.dataPrivacy.summary',
          '覆盖日常维护能力：JSON 导入导出、邮箱脱敏显示、异常处理与恢复流程。',
        ),
        outcomes: [
          t('manual.dataPrivacy.outcomes.0', '批量导入账号，快速迁移环境。'),
          t('manual.dataPrivacy.outcomes.1', '导出 JSON 做备份或跨设备迁移。'),
          t('manual.dataPrivacy.outcomes.2', '通过错误提示和文件修复指引定位问题。'),
          t('manual.dataPrivacy.outcomes.3', '可通过数据目录下 logs 的 app.log* 快速定位运行异常。'),
        ],
        steps: [
          t('manual.dataPrivacy.steps.0', '批量操作前先导出一次当前数据作为快照。'),
          t('manual.dataPrivacy.steps.1', '在列表页通过“显示/隐藏邮箱”切换隐私视图。'),
          t('manual.dataPrivacy.steps.2', '遇到文件损坏提示时，按弹窗指引打开目录修复。'),
          t('manual.dataPrivacy.steps.3', '排障时先进入“设置 -> 数据目录 -> 打开”，进入 logs 文件夹。'),
          t('manual.dataPrivacy.steps.4', '优先查看最新的 app.log 或 app.log.*（按日期滚动的日志文件）。'),
          t('manual.dataPrivacy.steps.5', '提交反馈时建议附上：发生时间、平台、复现步骤、关键报错日志（前后 20 行）。'),
        ],
        cautions: [
          t('manual.dataPrivacy.cautions.0', '导入来源不可信的 JSON 前先在测试环境验证。'),
          t('manual.dataPrivacy.cautions.1', '删除与批量操作前先确认筛选条件，避免误删。'),
          t('manual.dataPrivacy.cautions.2', '反馈日志前请先脱敏，避免粘贴完整 token、cookie、邮箱等敏感信息。'),
        ],
        keywords: [
          t('manual.dataPrivacy.keywords.0', '导入'),
          t('manual.dataPrivacy.keywords.1', '导出'),
          t('manual.dataPrivacy.keywords.2', '隐私'),
          t('manual.dataPrivacy.keywords.3', '邮箱脱敏'),
          t('manual.dataPrivacy.keywords.4', '故障'),
          t('manual.dataPrivacy.keywords.5', '日志'),
          t('manual.dataPrivacy.keywords.6', 'logs'),
          t('manual.dataPrivacy.keywords.7', 'app.log'),
        ],
        actions: [
          { id: 'go-overview', kind: 'navigate', page: 'overview', label: t('manual.actions.goAntigravity', '前往 Antigravity IDE'), primary: true },
          { id: 'go-settings', kind: 'navigate', page: 'settings', label: t('manual.actions.goSettings', '前往设置') },
        ],
      },
    ],
    [t],
  );

  const normalizedQuery = normalizeSearchText(query);

  const filteredSections = useMemo(() => {
    if (!normalizedQuery) return sections;
    return sections.filter((section) => {
      const payload = [
        section.title,
        section.summary,
        ...section.outcomes,
        ...section.steps,
        ...section.cautions,
        ...section.keywords,
      ]
        .join(' ')
        .toLowerCase();
      return payload.includes(normalizedQuery);
    });
  }, [normalizedQuery, sections]);

  const filteredIds = useMemo(() => filteredSections.map((section) => section.id), [filteredSections]);
  const allExpanded = filteredIds.length > 0 && filteredIds.every((id) => expandedIds.has(id));

  const toggleSection = (id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  };

  const handleToggleAll = () => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (allExpanded) {
        filteredIds.forEach((id) => next.delete(id));
      } else {
        filteredIds.forEach((id) => next.add(id));
      }
      return next;
    });
  };

  const handleAction = (action: ManualAction) => {
    if (action.kind === 'layout') {
      onOpenPlatformLayout();
      return;
    }
    onNavigate(action.page);
  };

  return (
    <main className="main-content manual-page">
      <div className="page-header">
        <div className="page-title">{t('manual.title', '功能使用手册')}</div>
        <div className="page-subtitle">
          {t(
            'manual.subtitle',
            '按任务场景组织的内置说明：先知道“为什么用”，再按步骤“马上可用”。',
          )}
        </div>
      </div>

      <section className="manual-toolbar">
        <div className="manual-search">
          <Search size={16} className="manual-search-icon" />
          <input
            type="text"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t('manual.searchPlaceholder', '搜索功能、场景或关键词（如：多开、注入、唤醒）')}
            aria-label={t('manual.searchAria', '搜索手册')}
          />
        </div>
        <div className="manual-toolbar-actions">
          <span className="manual-count">
            {t('manual.resultCount', '共 {{count}} 个章节', { count: filteredSections.length })}
          </span>
          <button className="btn btn-secondary" type="button" onClick={handleToggleAll}>
            {allExpanded
              ? t('manual.actions.collapseAll', '全部收起')
              : t('manual.actions.expandAll', '全部展开')}
          </button>
        </div>
      </section>

      {filteredSections.length === 0 ? (
        <div className="empty-state manual-empty-state">
          <h3>{t('manual.empty.title', '没有匹配内容')}</h3>
          <p>{t('manual.empty.desc', '尝试更换关键词，例如“实例”“切换”“设置”“导出”。')}</p>
        </div>
      ) : (
        <div className="manual-sections">
          {filteredSections.map((section) => {
            const expanded = expandedIds.has(section.id);
            return (
              <article
                key={section.id}
                className={`manual-card ${expanded ? 'expanded' : ''}`}
              >
                <button
                  type="button"
                  className="manual-card-header"
                  onClick={() => toggleSection(section.id)}
                  aria-expanded={expanded}
                >
                  <div className="manual-card-title-wrap">
                    <span className="manual-card-icon">{section.icon}</span>
                    <div className="manual-card-title-block">
                      <h3>{section.title}</h3>
                      <p>{section.summary}</p>
                    </div>
                  </div>
                  <ChevronDown size={18} className={`manual-card-arrow ${expanded ? 'expanded' : ''}`} />
                </button>

                {expanded && (
                  <div className="manual-card-body">
                    <div className="manual-info-grid">
                      <section className="manual-info-block">
                        <h4>💡 {t('manual.blocks.outcomes', '这个功能能帮你什么')}</h4>
                        <ul>
                          {section.outcomes.map((item, idx) => (
                            <li key={`${section.id}-outcome-${idx}`}>{item}</li>
                          ))}
                        </ul>
                      </section>
                      <section className="manual-info-block">
                        <h4>🎯 {t('manual.blocks.steps', '推荐操作步骤')}</h4>
                        <ol>
                          {section.steps.map((item, idx) => (
                            <li key={`${section.id}-step-${idx}`}>{item}</li>
                          ))}
                        </ol>
                      </section>
                      <section className="manual-info-block caution">
                        <h4>⚠️ {t('manual.blocks.cautions', '常见坑位 / 注意事项')}</h4>
                        <ul>
                          {section.cautions.map((item, idx) => (
                            <li key={`${section.id}-caution-${idx}`}>{item}</li>
                          ))}
                        </ul>
                      </section>
                    </div>

                    <div className="manual-keywords">
                      {section.keywords.map((keyword) => (
                        <span key={`${section.id}-${keyword}`} className="manual-keyword-chip">
                          {keyword}
                        </span>
                      ))}
                    </div>

                    <div className="manual-card-actions">
                      {section.actions.map((action) => (
                        <button
                          key={action.id}
                          type="button"
                          className={action.primary ? "btn btn-primary" : "btn btn-secondary"}
                          onClick={() => handleAction(action)}
                        >
                          {action.label}
                        </button>
                      ))}
                    </div>
                  </div>
                )}
              </article>
            );
          })}
        </div>
      )}
    </main>
  );
}
