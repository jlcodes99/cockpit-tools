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
  ShieldAlert,
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
        title: t('manual.quickStart.title', "Quick Start (5 minutes)"),
        summary: t(
          'manual.quickStart.summary',
          "Follow the first-run sequence: \"Check status -> Add account -> Switch account -> Create multi-instance\" to avoid getting lost.",
        ),
        outcomes: [
          t('manual.quickStart.outcomes.0', "First check global status on Dashboard, then operate on specific platform pages."),
          t('manual.quickStart.outcomes.1', "Complete one full closed loop on one platform first, then scale to others."),
          t('manual.quickStart.outcomes.2', "Understand that \"Account\" and \"multi-instance\" are two different workflows."),
        ],
        steps: [
          t('manual.quickStart.steps.0', "Open \"Dashboard\" and confirm the platforms you need are visible."),
          t('manual.quickStart.steps.1', "Go to target platform page (for example Codex / GitHub Copilot), then add 1 account first."),
          t('manual.quickStart.steps.2', "Use \"Switch/Inject\" button to verify account switching takes effect."),
          t('manual.quickStart.steps.3', "Then go to \"Multi-Instance\" and create 1 instance to verify isolation and parallel running."),
        ],
        cautions: [
          t('manual.quickStart.cautions.0', "For first-time usage, operate one platform only. Finish one full flow before bulk import."),
          t('manual.quickStart.cautions.1', "If a platform startup path is missing, fill path in settings before continuing."),
        ],
        keywords: [
          t('manual.quickStart.keywords.0', "quick start"),
          t('manual.quickStart.keywords.1', "beginner"),
          t('manual.quickStart.keywords.2', 'first run'),
          t('manual.quickStart.keywords.3', 'onboarding'),
        ],
        actions: [
          { id: 'go-dashboard', kind: 'navigate', page: 'dashboard', label: t('manual.actions.goDashboard', "Go to Dashboard"), primary: true },
          { id: 'go-overview', kind: 'navigate', page: 'overview', label: t('manual.actions.goAntigravity', "Go to Antigravity") },
          { id: 'go-settings', kind: 'navigate', page: 'settings', label: t('manual.actions.goSettings', "Go to Settings") },
        ],
      },
      {
        id: 'dashboard',
        icon: <Compass size={18} />,
        title: t('manual.dashboard.title', "Dashboard"),
        summary: t(
          'manual.dashboard.summary',
          "For global overview: quickly check account status on each platform, use recommended switching, and jump to feature pages.",
        ),
        outcomes: [
          t('manual.dashboard.outcomes.0', "See multi-platform \"current account / recommended account / quota overview\" on one page."),
          t('manual.dashboard.outcomes.1', "Use quick switch buttons to reduce page-hopping cost."),
        ],
        steps: [
          t('manual.dashboard.steps.0', "First check whether each platform card has account and quota data."),
          t('manual.dashboard.steps.1', "If status is abnormal, click refresh or enter the corresponding platform page to handle it."),
          t('manual.dashboard.steps.2', "If you need to change platform display order, open \"Platform Layout\" management."),
        ],
        cautions: [
          t('manual.dashboard.cautions.0', "Dashboard is for the big picture. For fine-grained operations, go to the specific platform page."),
        ],
        keywords: [
          t('manual.dashboard.keywords.0', "dashboard"),
          t('manual.dashboard.keywords.1', "overview"),
          t('manual.dashboard.keywords.2', "recommended switch"),
          t('manual.dashboard.keywords.3', 'dashboard'),
        ],
        actions: [
          { id: 'go-dashboard', kind: 'navigate', page: 'dashboard', label: t('manual.actions.goDashboard', "Go to Dashboard"), primary: true },
          { id: 'open-layout', kind: 'layout', label: t('manual.actions.openLayout', "Open Platform Layout") },
        ],
      },
      {
        id: 'antigravity',
        icon: <Sparkles size={18} />,
        title: t('manual.antigravity.title', "Antigravity Account"),
        summary: t(
          'manual.antigravity.summary',
          "Manage the Antigravity account lifecycle: add, refresh quota, switch, group, and tag.",
        ),
        outcomes: [
          t('manual.antigravity.outcomes.0', "Supports OAuth, import, export, and bulk operations."),
          t('manual.antigravity.outcomes.1', "Supports tag filtering, sorting, and grouped display."),
        ],
        steps: [
          t('manual.antigravity.steps.0', "Click \"Add Account\" to complete authorization or import."),
          t('manual.antigravity.steps.1', "Use \"Refresh All\" to sync one full round of quota status first."),
          t('manual.antigravity.steps.2', "Sort by quota/reset time and choose the account you want to use now."),
        ],
        cautions: [
          t('manual.antigravity.cautions.0', "If you see abnormal statuses (for example 403/refresh failed), refresh first, then decide whether to remove and re-login."),
        ],
        keywords: [
          t('manual.antigravity.keywords.0', 'antigravity'),
          t('manual.antigravity.keywords.1', "Account"),
          t('manual.antigravity.keywords.2', "quota"),
          t('manual.antigravity.keywords.3', "tags"),
        ],
        actions: [
          { id: 'go-overview', kind: 'navigate', page: 'overview', label: t('manual.actions.goAntigravity', "Go to Antigravity"), primary: true },
        ],
      },
      {
        id: 'providers',
        icon: <BookOpen size={18} />,
        title: t('manual.providers.title', 'Codex / GitHub Copilot / Windsurf / Kiro'),
        summary: t(
          'manual.providers.summary',
          "The four platform pages share the same structure: account overview + multi-instance, with OAuth, Token/JSON import and switching.",
        ),
        outcomes: [
          t('manual.providers.outcomes.0', "Codex: account switching + quota refresh."),
          t('manual.providers.outcomes.1', "GitHub Copilot/Windsurf/Kiro: support injection into VS Code chain."),
          t('manual.providers.outcomes.2', "Each platform supports independent filtering, grouping, import, and export."),
        ],
        steps: [
          t('manual.providers.steps.0', "Complete OAuth or Token import first."),
          t('manual.providers.steps.1', "Confirm plan/quota and reset time are visible in account list."),
          t('manual.providers.steps.2', "Execute \"Switch/Inject\" and verify in client that it takes effect."),
        ],
        cautions: [
          t('manual.providers.cautions.0', "Account notes describe local read/write and permission scope. Read first before operating."),
          t('manual.providers.cautions.1', "Some platforms or capabilities are system-limited. Follow on-page prompts."),
        ],
        keywords: [
          t('manual.providers.keywords.0', 'codex'),
          t('manual.providers.keywords.1', 'github copilot'),
          t('manual.providers.keywords.2', 'windsurf'),
          t('manual.providers.keywords.3', 'kiro'),
          t('manual.providers.keywords.4', "inject"),
          t('manual.providers.keywords.5', "account switch"),
        ],
        actions: [
          { id: 'go-codex', kind: 'navigate', page: 'codex', label: t('manual.actions.goCodex', "Go to Codex"), primary: true },
          { id: 'go-ghcp', kind: 'navigate', page: 'github-copilot', label: t('manual.actions.goGitHubCopilot', "Go to GitHub Copilot") },
          { id: 'go-windsurf', kind: 'navigate', page: 'windsurf', label: t('manual.actions.goWindsurf', "Go to Windsurf") },
          { id: 'go-kiro', kind: 'navigate', page: 'kiro', label: t('manual.actions.goKiro', "Go to Kiro") },
        ],
      },
      {
        id: 'instances',
        icon: <LayoutGrid size={18} />,
        title: t('manual.instances.title', "Multi-Instance (Key)"),
        summary: t(
          'manual.instances.summary',
          "Used for account isolation and parallel running. Each instance has independent directory and state to avoid cross-contamination.",
        ),
        outcomes: [
          t('manual.instances.outcomes.0', "Isolate work/personal account environments."),
          t('manual.instances.outcomes.1', "Run multiple accounts in parallel to reduce repeated login switching."),
          t('manual.instances.outcomes.2', "Validate new config in a test instance first, then return to main instance."),
        ],
        steps: [
          t('manual.instances.steps.0', "When creating a new instance, prefer \"Copy Source Instance\" for faster usability."),
          t('manual.instances.steps.1', "If you choose \"Blank Instance\", start once for initialization, then bind account."),
          t('manual.instances.steps.2', "After binding account, manage lifecycle via \"Start / Locate Window / Stop\"."),
        ],
        cautions: [
          t('manual.instances.cautions.0', "Before copying from source instance, close the source first to avoid data inconsistency."),
          t('manual.instances.cautions.1', "You cannot bind an account before a blank instance is initialized. This is expected behavior."),
        ],
        keywords: [
          t('manual.instances.keywords.0', "multi-instance"),
          t('manual.instances.keywords.1', "instance"),
          t('manual.instances.keywords.2', "isolation"),
          t('manual.instances.keywords.3', "parallel"),
          t('manual.instances.keywords.4', "initialization"),
        ],
        actions: [
          { id: 'go-instances', kind: 'navigate', page: 'instances', label: t('manual.actions.goInstances', "Go to Multi-Instance"), primary: true },
        ],
      },
      {
        id: 'fingerprints',
        icon: <ShieldAlert size={18} />,
        title: t('manual.fingerprints.title', "Device Fingerprints"),
        summary: t(
          'manual.fingerprints.summary',
          "Manage fingerprint templates and account bindings. Supports generation, capture, import, and binding maintenance.",
        ),
        outcomes: [
          t('manual.fingerprints.outcomes.0', "Generate/capture new fingerprints and maintain metadata."),
          t('manual.fingerprints.outcomes.1', "View accounts bound to a fingerprint and add/remove bindings."),
        ],
        steps: [
          t('manual.fingerprints.steps.0', "Create or import fingerprints first."),
          t('manual.fingerprints.steps.1', "Open details and confirm fingerprint info is correct."),
          t('manual.fingerprints.steps.2', "In binding management, bind accounts to target fingerprints."),
        ],
        cautions: [
          t('manual.fingerprints.cautions.0', "Before deleting a fingerprint, check bound account count first to avoid affecting production accounts by mistake."),
        ],
        keywords: [
          t('manual.fingerprints.keywords.0', "fingerprint"),
          t('manual.fingerprints.keywords.1', "binding"),
          t('manual.fingerprints.keywords.2', "import"),
          t('manual.fingerprints.keywords.3', 'capture'),
        ],
        actions: [
          { id: 'go-fingerprints', kind: 'navigate', page: 'fingerprints', label: t('manual.actions.goFingerprints', "Go to Device Fingerprints"), primary: true },
        ],
      },
      {
        id: 'wakeup',
        icon: <Rocket size={18} />,
        title: t('manual.wakeup.title', "Wakeup Tasks & Verification"),
        summary: t(
          'manual.wakeup.summary',
          "Used for scheduled wakeup tasks and batch verification, helping continuous tracking of account availability and status.",
        ),
        outcomes: [
          t('manual.wakeup.outcomes.0', "Create recurring tasks and record historical execution results."),
          t('manual.wakeup.outcomes.1', "Use verification page for batch checks and failure analysis."),
        ],
        steps: [
          t('manual.wakeup.steps.0', "Create a new task in \"Wakeup Tasks\" and set model and trigger mode."),
          t('manual.wakeup.steps.1', "Run manually once first to ensure task parameters are valid."),
          t('manual.wakeup.steps.2', "On \"Verification\" page, run batch inspection and historical review."),
        ],
        cautions: [
          t('manual.wakeup.cautions.0', "For first-time setup, select only a small number of accounts. Expand after stability is confirmed."),
          t('manual.wakeup.cautions.1', "If verification result is abnormal, check errors and verification links in details first."),
        ],
        keywords: [
          t('manual.wakeup.keywords.0', "wakeup"),
          t('manual.wakeup.keywords.1', "task"),
          t('manual.wakeup.keywords.2', "verification"),
          t('manual.wakeup.keywords.3', "schedule"),
          t('manual.wakeup.keywords.4', 'history'),
        ],
        actions: [
          { id: 'go-wakeup', kind: 'navigate', page: 'wakeup', label: t('manual.actions.goWakeup', "Go to Wakeup Tasks"), primary: true },
          { id: 'go-verification', kind: 'navigate', page: 'verification', label: t('manual.actions.goVerification', "Go to Wakeup Verification") },
        ],
      },
      {
        id: 'settings',
        icon: <Settings size={18} />,
        title: t('manual.settings.title', "Settings & System Capabilities"),
        summary: t(
          'manual.settings.summary',
          "Centralized management for language, theme, auto-refresh, alert threshold, app paths, and network service settings.",
        ),
        outcomes: [
          t('manual.settings.outcomes.0', "Configure auto-refresh and quota alerts across platforms in one place."),
          t('manual.settings.outcomes.1', "Set startup paths for each client and fix \"missing path\" issues."),
          t('manual.settings.outcomes.2', "Adjust window behavior, language, and theme to fit your habits."),
        ],
        steps: [
          t('manual.settings.steps.0', "In \"General\" tab, finish baseline language/theme/path setup first."),
          t('manual.settings.steps.1', "Then adjust auto-refresh and alert threshold per platform."),
          t('manual.settings.steps.2', "After network service port changes, restart as prompted to apply."),
        ],
        cautions: [
          t('manual.settings.cautions.0', "When path detection fails, manually selecting executable path is the safest option."),
          t('manual.settings.cautions.1', "Too-low thresholds trigger frequent alerts. Start around 20%."),
        ],
        keywords: [
          t('manual.settings.keywords.0', "settings"),
          t('manual.settings.keywords.1', "path"),
          t('manual.settings.keywords.2', "refresh"),
          t('manual.settings.keywords.3', "threshold"),
          t('manual.settings.keywords.4', "language"),
          t('manual.settings.keywords.5', "theme"),
        ],
        actions: [
          { id: 'go-settings', kind: 'navigate', page: 'settings', label: t('manual.actions.goSettings', "Go to Settings"), primary: true },
          { id: 'open-layout', kind: 'layout', label: t('manual.actions.openLayout', "Open Platform Layout") },
        ],
      },
      {
        id: 'data-and-privacy',
        icon: <Lightbulb size={18} />,
        title: t('manual.dataPrivacy.title', "Import/Export, Privacy & Troubleshooting"),
        summary: t(
          'manual.dataPrivacy.summary',
          "Covers daily maintenance: JSON import/export, masked email display, exception handling and recovery flow.",
        ),
        outcomes: [
          t('manual.dataPrivacy.outcomes.0', "Import accounts in bulk for fast environment migration."),
          t('manual.dataPrivacy.outcomes.1', "Export JSON for backup or cross-device migration."),
          t('manual.dataPrivacy.outcomes.2', "Locate issues via error hints and file repair guidance."),
          t('manual.dataPrivacy.outcomes.3', "Quickly locate runtime issues through app.log* in Data Directory/logs."),
        ],
        steps: [
          t('manual.dataPrivacy.steps.0', "Before bulk operations, export current data once as a snapshot."),
          t('manual.dataPrivacy.steps.1', "On list pages, switch privacy view via \"Show/Hide Email\"."),
          t('manual.dataPrivacy.steps.2', "When file corruption prompts appear, follow the dialog and open the directory to repair."),
          t('manual.dataPrivacy.steps.3', "For troubleshooting, go to \"Settings -> Data Directory -> Open\", then enter logs folder."),
          t('manual.dataPrivacy.steps.4', "Check the latest app.log or app.log.* first (date-rotated log files)."),
          t('manual.dataPrivacy.steps.5', "When submitting feedback, include: occurrence time, platform, reproduction steps, key error logs (20 lines before/after)."),
        ],
        cautions: [
          t('manual.dataPrivacy.cautions.0', "Before importing JSON from untrusted sources, validate it in a test environment first."),
          t('manual.dataPrivacy.cautions.1', "Before deletion and bulk operations, confirm filters first to avoid accidental deletion."),
          t('manual.dataPrivacy.cautions.2', "Before sharing logs, redact sensitive data. Do not paste full token, cookie, email, etc."),
        ],
        keywords: [
          t('manual.dataPrivacy.keywords.0', "import"),
          t('manual.dataPrivacy.keywords.1', "export"),
          t('manual.dataPrivacy.keywords.2', "privacy"),
          t('manual.dataPrivacy.keywords.3', "email masking"),
          t('manual.dataPrivacy.keywords.4', "troubleshooting"),
          t('manual.dataPrivacy.keywords.5', "logs"),
          t('manual.dataPrivacy.keywords.6', 'logs'),
          t('manual.dataPrivacy.keywords.7', 'app.log'),
        ],
        actions: [
          { id: 'go-overview', kind: 'navigate', page: 'overview', label: t('manual.actions.goAntigravity', "Go to Antigravity"), primary: true },
          { id: 'go-settings', kind: 'navigate', page: 'settings', label: t('manual.actions.goSettings', "Go to Settings") },
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
        <div className="page-title">{t('manual.title', "User Manual")}</div>
        <div className="page-subtitle">
          {t(
            'manual.subtitle',
            "Built-in instructions organized by task scenarios: know \"why to use\" first, then follow steps to \"use immediately\".",
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
            placeholder={t('manual.searchPlaceholder', "Search feature, scenario, or keyword (for example: multi-instance, inject, wakeup, fingerprint)")}
            aria-label={t('manual.searchAria', "Search manual")}
          />
        </div>
        <div className="manual-toolbar-actions">
          <span className="manual-count">
            {t('manual.resultCount', "{{count}} sections total", { count: filteredSections.length })}
          </span>
          <button className="btn btn-secondary" type="button" onClick={handleToggleAll}>
            {allExpanded
              ? t('manual.actions.collapseAll', "Collapse all")
              : t('manual.actions.expandAll', "Expand all")}
          </button>
        </div>
      </section>

      {filteredSections.length === 0 ? (
        <div className="empty-state manual-empty-state">
          <h3>{t('manual.empty.title', "No matching content")}</h3>
          <p>{t('manual.empty.desc', "Try different keywords, for example \"instance\", \"switch\", \"settings\", \"export\".")}</p>
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
                        <h4>💡 {t('manual.blocks.outcomes', "What this feature helps with")}</h4>
                        <ul>
                          {section.outcomes.map((item, idx) => (
                            <li key={`${section.id}-outcome-${idx}`}>{item}</li>
                          ))}
                        </ul>
                      </section>
                      <section className="manual-info-block">
                        <h4>🎯 {t('manual.blocks.steps', "Recommended steps")}</h4>
                        <ol>
                          {section.steps.map((item, idx) => (
                            <li key={`${section.id}-step-${idx}`}>{item}</li>
                          ))}
                        </ol>
                      </section>
                      <section className="manual-info-block caution">
                        <h4>⚠️ {t('manual.blocks.cautions', "Common pitfalls / notes")}</h4>
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
