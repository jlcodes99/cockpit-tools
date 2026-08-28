import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import {
  Activity,
  BarChart3,
  Check,
  CircleAlert,
  Copy,
  DollarSign,
  Eye,
  EyeOff,
  FolderPlus,
  Image as ImageIcon,
  KeyRound,
  Layers,
  Play,
  Plus,
  Power,
  RefreshCw,
  Route,
  ScrollText,
  Send,
  Shield,
  SlidersHorizontal,
  Trash2,
  Users,
  X,
  Wrench,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import * as codebuddyLocalAccessService from "../services/codebuddyLocalAccessService";
import type {
  CodebuddyLocalAccessAccountHealth,
  CodebuddyLocalAccessAccountOption,
  CodebuddyLocalAccessApiKey,
  CodebuddyLocalAccessCollection,
  CodebuddyLocalAccessCustomRoutingRule,
  CodebuddyLocalAccessImageGenerationMode,
  CodebuddyLocalAccessLogPage,
  CodebuddyLocalAccessRoutingStrategy,
  CodebuddyLocalAccessScope,
  CodebuddyLocalAccessState,
  CodebuddyLocalAccessStats,
  CodebuddyLocalAccessUsageStats,
} from "../types/codebuddyLocalAccess";
import { SingleSelectDropdown } from "../components/SingleSelectDropdown";
import { CodexStatsRangePicker } from "../components/CodexStatsRangePicker";
import { ManualHelpIconButton } from "../components/ManualHelpIconButton";
import { PlatformGroupSwitcher } from "../components/platform/PlatformGroupSwitcher";
import {
  buildCodexStatsTimeRange,
  type CodexStatsRangeKey,
  type CodexStatsTimeRange,
} from "../utils/codexStatsRange";
import {
  findGroupByPlatform,
  resolveGroupChildName,
  usePlatformLayoutStore,
} from "../stores/usePlatformLayoutStore";
import { getPlatformLabel } from "../utils/platformMeta";
import "./CodebuddyApiServicePage.css";
import "./CodexApiServicePage.css";

type TabId = "overview" | "keys" | "accounts" | "models" | "logs";

type CopyField =
  | "baseUrl"
  | "apiKey"
  | "compat:chat"
  | "compat:responses"
  | "compat:anthropic"
  | "compat:gemini"
  | "compat:ollama";

const CB_STATS_RANGE_STORAGE_KEY = "agtools.codebuddy.local_access.stats_range";

function normalizeCbStatsRange(
  value: string | null | undefined,
): CodexStatsRangeKey {
  if (value === "weekly" || value === "monthly" || value === "custom") {
    return value;
  }
  return "daily";
}

function readStoredCbStatsRange(): CodexStatsRangeKey {
  try {
    return normalizeCbStatsRange(localStorage.getItem(CB_STATS_RANGE_STORAGE_KEY));
  } catch {
    return "daily";
  }
}

function persistCbStatsRange(value: CodexStatsRangeKey): void {
  try {
    localStorage.setItem(CB_STATS_RANGE_STORAGE_KEY, value);
  } catch {
    // ignore
  }
}

const DEFAULT_COLLECTION: CodebuddyLocalAccessCollection = {
  enabled: false,
  port: 11435,
  bindHost: "127.0.0.1",
  scope: "localhost",
  intlAccountIds: [],
  cnAccountIds: [],
  modelAliases: [],
  excludedModels: [],
  debugLogs: false,
  sessionAffinity: true,
  sessionAffinityTtlMs: 30 * 60 * 1000,
  routingStrategy: "auto",
  customRoutingRules: [],
  maxRetryCredentials: 2,
  maxRetryIntervalMs: 2000,
  disableCooling: false,
  requestTimeoutMs: 120000,
  apiKeys: [],
  imageGenerationMode: "disabled",
  maxConcurrentImageRequests: 1,
  immediateSseResponse: false,
  responsesWebsocketsEnabled: false,
  visionToolEnabled: false,
};

interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

export function CodebuddyApiServicePage() {
  const { t } = useTranslation();
  const [state, setState] = useState<CodebuddyLocalAccessState | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [activating, setActivating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<TabId>("overview");
  const [keyVisible, setKeyVisible] = useState(false);
  const [copiedField, setCopiedField] = useState<CopyField | null>(null);
  const [portInput, setPortInput] = useState<string>("");
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [testOpen, setTestOpen] = useState(false);
  const [portKilling, setPortKilling] = useState(false);
  const [statsRange, setStatsRange] = useState<CodexStatsRangeKey>(() =>
    readStoredCbStatsRange(),
  );
  const [statsTimeRange, setStatsTimeRange] = useState<CodexStatsTimeRange>(() =>
    buildCodexStatsTimeRange(readStoredCbStatsRange()),
  );
  // 全局 stats 状态：供账号池按账号统计使用（按账号缓存命中/credit 等）
  const [stats, setStats] = useState<CodebuddyLocalAccessStats | null>(null);
  const [statsLoading, setStatsLoading] = useState(false);
  const [memberModalOpen, setMemberModalOpen] = useState(false);

  const loadStats = useCallback(async () => {
    setStatsLoading(true);
    try {
      setStats(await codebuddyLocalAccessService.getCodebuddyLocalAccessStats());
    } catch {
      // ignore
    } finally {
      setStatsLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadStats();
  }, [loadStats]);

  // 事件驱动刷新：sidecar 产生新事件时，Rust 侧 emit 事件通知前端刷新统计。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listen("codebuddy-local-access-stats-changed", () => {
      if (!disposed) {
        void loadStats();
      }
    }).then((fn) => {
      if (disposed) {
        fn();
      } else {
        unlisten = fn;
      }
    });

    // 5 分钟轮询兜底：sidecar 未运行或事件丢失时仍能更新统计（避免频繁刷新）。
    const timer = setInterval(() => {
      void loadStats();
    }, 5 * 60 * 1000);

    return () => {
      disposed = true;
      unlisten?.();
      clearInterval(timer);
    };
  }, [loadStats]);

  const handleOpenAddAccount = useCallback(() => {
    // 跳转到 CodeBuddy 中国站账号登录页（用户登录后可在管理成员弹窗中加入池）。
    window.dispatchEvent(
      new CustomEvent("app-request-navigate", { detail: "codebuddy-cn" }),
    );
  }, []);

  useEffect(() => {
    persistCbStatsRange(statsRange);
  }, [statsRange]);

  const collection = state?.collection ?? DEFAULT_COLLECTION;
  const displayBaseUrl = state?.baseUrl ?? "";

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const next = await codebuddyLocalAccessService.getCodebuddyLocalAccessState();
      setState(next);
      setPortInput(String(next?.collection?.port ?? DEFAULT_COLLECTION.port));
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const update = useCallback((patch: Partial<CodebuddyLocalAccessCollection>) => {
    setState((prev) => {
      if (!prev) return prev;
      return { ...prev, collection: { ...prev.collection, ...patch } };
    });
  }, []);

  const save = useCallback(async () => {
    if (!state) return;
    setSaving(true);
    setError(null);
    try {
      const next = await codebuddyLocalAccessService.saveCodebuddyLocalAccessCollection(
        state.collection,
      );
      setState(next);
      setNotice("配置已保存");
      setTimeout(() => setNotice(null), 1800);
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }, [state]);

  // 纯文本模型视觉子代理开关：直接基于当前 collection 持久化（避免 setState
  // 异步导致 save 读到旧值），开启/关闭即时生效。
  const toggleVisionTool = useCallback(
    async (enabled: boolean) => {
      setSaving(true);
      setError(null);
      try {
        const next =
          await codebuddyLocalAccessService.saveCodebuddyLocalAccessCollection({
            ...collection,
            visionToolEnabled: enabled,
          });
        setState(next);
        setNotice(enabled ? "视觉子代理已开启" : "视觉子代理已关闭");
        setTimeout(() => setNotice(null), 1800);
      } catch (err) {
        setError(String(err));
      } finally {
        setSaving(false);
      }
    },
    [collection],
  );

  const toggleEnabled = useCallback(async () => {
    setSaving(true);
    setError(null);
    try {
      const next = await codebuddyLocalAccessService.setCodebuddyLocalAccessEnabled(
        !collection.enabled,
      );
      setState(next);
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }, [collection.enabled]);

  const handleActivateService = useCallback(async () => {
    setActivating(true);
    setError(null);
    try {
      const next = await codebuddyLocalAccessService.setCodebuddyLocalAccessEnabled(true);
      setState(next);
      setNotice("已发出启动指令");
      setTimeout(() => setNotice(null), 1800);
    } catch (err) {
      setError(String(err));
    } finally {
      setActivating(false);
    }
  }, []);

  const handleKillPort = useCallback(async () => {
    const port = state?.actualPort ?? collection.port;
    if (!port) return;
    setPortKilling(true);
    try {
      await codebuddyLocalAccessService.killCodebuddyLocalAccessPort(port);
      await load();
    } catch (err) {
      setError(String(err));
    } finally {
      setPortKilling(false);
    }
  }, [state?.actualPort, collection.port, load]);

  const handleCopy = useCallback(async (field: CopyField, value: string) => {
    if (!value) return;
    try {
      await navigator.clipboard.writeText(value);
      setCopiedField(field);
      setTimeout(() => setCopiedField(null), 1500);
    } catch {
      // ignore
    }
  }, []);

  const handleSavePort = useCallback(async () => {
    const portNum = Number(portInput);
    if (!Number.isFinite(portNum) || portNum < 1024 || portNum > 65535) {
      setError("端口必须是 1024-65535 之间的整数");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const next = await codebuddyLocalAccessService.saveCodebuddyLocalAccessCollection({
        ...collection,
        port: portNum,
      });
      setState(next);
      setNotice("端口已保存");
      setTimeout(() => setNotice(null), 1500);
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }, [collection, portInput]);

  const firstApiKey = collection.apiKeys[0]?.key ?? "";

  // ─── 用量统计公共区（所有 tab 共享，对齐 Codex 布局） ───
  const totals = stats?.totals;
  const successRate =
    totals && totals.requestCount > 0
      ? Math.round((totals.successCount / totals.requestCount) * 1000) / 10
      : 0;
  const avgLatencySec =
    totals && totals.requestCount > 0
      ? totals.totalLatencyMs / totals.requestCount / 1000
      : 0;
  const cacheHitRate =
    totals &&
    totals.promptCacheHitTokens + totals.promptCacheMissTokens > 0
      ? Math.round(
          (totals.promptCacheHitTokens /
            (totals.promptCacheHitTokens + totals.promptCacheMissTokens)) *
            1000,
        ) / 10
      : 0;

  const summaryCards = useMemo(
    () => [
      {
        key: "requests",
        label: "总请求数",
        value: formatCompactNumber(totals?.requestCount ?? 0),
        detail: `成功 ${formatCompactNumber(totals?.successCount ?? 0)} / 失败 ${formatCompactNumber(totals?.failureCount ?? 0)}`,
      },
      {
        key: "tokens",
        label: "总 Token 数",
        value: formatCompactNumber(totals?.totalTokens ?? 0),
        detail: `输入 ${formatCompactNumber(totals?.inputTokens ?? 0)} / 输出 ${formatCompactNumber(totals?.outputTokens ?? 0)}`,
      },
      {
        key: "cache",
        label: "缓存命中",
        value: formatCompactNumber(totals?.promptCacheHitTokens ?? 0),
        detail: `命中率 ${cacheHitRate}% · 未命中 ${formatCompactNumber(totals?.promptCacheMissTokens ?? 0)}`,
      },
      {
        key: "cost",
        label: "Credit 消耗",
        value: formatCredit(totals?.totalCredit ?? 0),
        detail: "按 CodeBuddy Credit 累计",
      },
      {
        key: "latency",
        label: "平均延迟",
        value: avgLatencySec > 0 ? `${avgLatencySec.toFixed(2)}s` : "-",
        detail: `成功率 ${successRate}%`,
      },
    ],
    [totals, cacheHitRate, avgLatencySec, successRate],
  );

  const selectedStatsRangeTitle =
    statsRange === "daily"
      ? "今日"
      : statsRange === "weekly"
        ? "本周"
        : statsRange === "monthly"
          ? "本月"
          : `${statsTimeRange.startInput} - ${statsTimeRange.endInput}`;

  const serviceTabs: Array<{ key: TabId; label: string; icon: ReactNode }> = [
    { key: "overview", label: "服务总览", icon: <Route className="tab-icon" /> },
    { key: "keys", label: "客户端 Key", icon: <KeyRound className="tab-icon" /> },
    { key: "accounts", label: "账号池", icon: <Users className="tab-icon" /> },
    { key: "models", label: "模型与能力", icon: <ImageIcon className="tab-icon" /> },
    { key: "logs", label: "统计与日志", icon: <Activity className="tab-icon" /> },
  ];

  const currentPlatformId = "codebuddy_api_service" as const;
  const { platformGroups } = usePlatformLayoutStore();
  const currentGroup = useMemo(
    () => findGroupByPlatform(platformGroups, currentPlatformId),
    [platformGroups],
  );
  const switchOptions = useMemo(() => {
    const customLabels: Record<string, string> = {
      codebuddy_api_service: "CodeBuddy API 服务",
    };
    return (currentGroup ? currentGroup.platformIds : [currentPlatformId]).map(
      (platformId) => ({
        platformId,
        label: currentGroup
          ? resolveGroupChildName(
              currentGroup,
              platformId,
              customLabels[platformId] ?? getPlatformLabel(platformId, t),
            )
          : customLabels[platformId] ?? getPlatformLabel(platformId, t),
      }),
    );
  }, [currentGroup, t]);

  if (loading) {
    return (
      <div className="codebuddy-api-service-page cb-api-muted">加载中…</div>
    );
  }

  return (
    <div className="codebuddy-api-service-page">
      <div className="page-top-strip">
        <div className="page-top-strip-left">
          <span className="page-top-strip-label">
            {t("settings.general.account", "Accounts")}
          </span>
          <ManualHelpIconButton className="platform-header-help" />
        </div>
        <div className="page-top-strip-right-placeholder" aria-hidden="true" />
      </div>

      <div className="page-tabs-row page-tabs-center page-tabs-row-with-leading">
        <div className="page-tabs-leading">
          <PlatformGroupSwitcher
            currentPlatformId={currentPlatformId}
            currentLabel={
              currentGroup
                ? resolveGroupChildName(
                    currentGroup,
                    currentPlatformId,
                    "CodeBuddy API 服务",
                  )
                : "CodeBuddy API 服务"
            }
            options={switchOptions}
            currentGroupId={currentGroup?.id ?? null}
          />
        </div>
        <div className="page-tabs filter-tabs">
          {serviceTabs.map((tab) => (
            <button
              key={tab.key}
              className={`filter-tab${activeTab === tab.key ? " active" : ""}`}
              onClick={() => setActiveTab(tab.key)}
            >
              {tab.icon}
              <span>{tab.label}</span>
            </button>
          ))}
        </div>
      </div>

      <main className="codex-api-service-content">
        <section className="codex-api-service-hero">
          <div className="codex-api-service-hero-main">
            <div className="codex-api-service-title-row">
              <span className="codex-api-service-title-icon" aria-hidden="true">
                <Route size={24} />
              </span>
              <div className="codex-api-service-title-copy">
                <div className="codex-api-service-title-line">
                  <h1>CodeBuddy API 服务</h1>
                  <span
                    className={`codex-api-service-status ${
                      state?.running
                        ? "running"
                        : collection.enabled
                          ? "stopped"
                          : "disabled"
                    }`}
                  >
                    {collection.enabled
                      ? state?.running
                        ? "运行中"
                        : "未运行"
                      : "已停用"}
                  </span>
                  <span className="codex-api-service-pill mode-sidecar">反代模式</span>
                </div>
              </div>
            </div>
          </div>
          <div className="codex-api-service-hero-actions">
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => void load()}
              disabled={saving || activating}
              title="刷新统计"
            >
              <RefreshCw size={14} className={loading ? "loading-spinner" : ""} />
              刷新统计
            </button>
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => setTestOpen(true)}
              disabled={!collection || saving || activating}
              title="测试"
            >
              <Shield size={14} />
              测试
            </button>
            <button
              type="button"
              className={`btn ${collection.enabled ? "btn-secondary" : "btn-primary"}`}
              onClick={() => void handleActivateService()}
              disabled={!collection || saving || activating}
              title="启动 API 服务"
            >
              {activating ? (
                <RefreshCw size={14} className="loading-spinner" />
              ) : (
                <Play size={14} />
              )}
              启动 API 服务
            </button>
            <button
              type="button"
              className={`btn ${collection.enabled ? "btn-danger" : "btn-secondary"}`}
              onClick={() => void toggleEnabled()}
              disabled={!collection || saving || activating}
            >
              <Power size={14} />
              {collection.enabled ? "停用服务" : "启用服务"}
            </button>
          </div>
        </section>

        {(error || notice || state?.lastError) && (
          <div className="codex-api-service-message-stack">
            {error && (
              <div className="codex-api-service-message error">
                <CircleAlert size={15} />
                <span>{error}</span>
                <button
                  type="button"
                  className="codex-api-service-message-dismiss"
                  onClick={() => setError(null)}
                  aria-label="关闭"
                  title="关闭"
                >
                  <X size={14} />
                </button>
              </div>
            )}
            {state?.lastError && (
              <div className="codex-api-service-message error">
                <CircleAlert size={15} />
                <span>{state.lastError}</span>
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={() => void handleKillPort()}
                  disabled={portKilling || saving || activating}
                >
                  <Wrench size={13} />
                  清理端口
                </button>
              </div>
            )}
            {notice && (
              <div className="codex-api-service-message success">
                <Check size={15} />
                <span>{notice}</span>
                <button
                  type="button"
                  className="codex-api-service-message-dismiss"
                  onClick={() => setNotice(null)}
                  aria-label="关闭"
                  title="关闭"
                >
                  <X size={14} />
                </button>
              </div>
            )}
          </div>
        )}

        {/* 用量统计公共区（所有 tab 共享，对齐 Codex 布局） */}
        <section className="codex-api-service-usage-toolbar">
          <div className="codex-api-service-usage-context">
            <Activity size={16} />
            <div>
              <strong>用量统计</strong>
              <span>
                {selectedStatsRangeTitle}
                {stats?.since
                  ? ` · 最后记录 ${formatDateTime(stats.since)}`
                  : ""}
              </span>
            </div>
          </div>
          <CodexStatsRangePicker
            value={statsRange}
            range={statsTimeRange}
            onPresetChange={(
              key: Exclude<CodexStatsRangeKey, "custom">,
              range: CodexStatsTimeRange,
            ) => {
              setStatsRange(key);
              setStatsTimeRange(range);
            }}
            onCustomApply={(range: CodexStatsTimeRange) => {
              setStatsRange("custom");
              setStatsTimeRange(range);
            }}
            disabled={statsLoading}
            compact
          />
        </section>

        <section className="codex-api-service-summary-grid">
          {summaryCards.map((item) => (
            <div key={item.key} className="codex-api-service-summary-card">
              <span>{item.label}</span>
              <strong>{item.value}</strong>
              <small>{item.detail}</small>
            </div>
          ))}
        </section>

        {activeTab === "overview" && (
          <OverviewTab
            state={state}
            collection={collection}
            displayBaseUrl={displayBaseUrl}
            firstApiKey={firstApiKey}
            keyVisible={keyVisible}
            setKeyVisible={setKeyVisible}
            copiedField={copiedField}
            onCopy={handleCopy}
            portInput={portInput}
            setPortInput={setPortInput}
            onSavePort={handleSavePort}
            onUpdate={update}
            onSave={save}
            saving={saving}
            onOpenAdvanced={() => setAdvancedOpen(true)}
            onOpenTest={() => setTestOpen(true)}
          />
        )}
        {activeTab === "keys" && (
          <KeysTab collection={collection} setState={setState} />
        )}
        {activeTab === "accounts" && (
          <AccountsTab
            state={state}
            collection={collection}
            setState={setState}
            onUpdate={update}
            onSave={save}
            saving={saving}
            stats={stats}
            statsLoading={statsLoading}
            onReloadStats={loadStats}
            onOpenAddAccount={handleOpenAddAccount}
            onOpenMemberModal={() => setMemberModalOpen(true)}
            onJumpToModels={() => setActiveTab("models")}
          />
        )}
        {activeTab === "models" && (
          <ModelsTab
            collection={collection}
            onUpdate={update}
            onRefresh={load}
            baseUrl={displayBaseUrl}
            firstApiKey={firstApiKey}
            onToggleVisionTool={toggleVisionTool}
          />
        )}
        {activeTab === "logs" && <LogsTab />}
      </main>

      {advancedOpen && (
        <AdvancedParamsDialog
          collection={collection}
          onUpdate={update}
          onSave={save}
          saving={saving}
          onClose={() => setAdvancedOpen(false)}
        />
      )}

      {memberModalOpen && (
        <MemberModal
          state={state}
          collection={collection}
          saving={saving}
          onClose={() => setMemberModalOpen(false)}
          onSave={async (nextIntlIds, nextCnIds) => {
            // 直接构造最新 collection 持久化，避免 setState/save 时序问题。
            try {
              const next = await codebuddyLocalAccessService.saveCodebuddyLocalAccessCollection({
                ...collection,
                intlAccountIds: nextIntlIds,
                cnAccountIds: nextCnIds,
              });
              setState(next);
              void loadStats();
            } catch (err) {
              setError(String(err));
            }
            setMemberModalOpen(false);
          }}
        />
      )}

      {testOpen && (
        <ChatTestDialog
          onClose={() => setTestOpen(false)}
          baseUrl={displayBaseUrl}
          firstApiKey={firstApiKey}
        />
      )}
    </div>
  );
}

/* ----------------------------- Overview ----------------------------- */

interface OverviewTabProps {
  state: CodebuddyLocalAccessState | null;
  collection: CodebuddyLocalAccessCollection;
  displayBaseUrl: string;
  firstApiKey: string;
  keyVisible: boolean;
  setKeyVisible: (updater: (v: boolean) => boolean) => void;
  copiedField: CopyField | null;
  onCopy: (field: CopyField, value: string) => Promise<void>;
  portInput: string;
  setPortInput: (v: string) => void;
  onSavePort: () => Promise<void>;
  onUpdate: (patch: Partial<CodebuddyLocalAccessCollection>) => void;
  onSave: () => Promise<void>;
  saving: boolean;
  onOpenAdvanced: () => void;
  onOpenTest: () => void;
}

function OverviewTab(props: OverviewTabProps) {
  const {
    state,
    collection,
    displayBaseUrl,
    firstApiKey,
    keyVisible,
    setKeyVisible,
    copiedField,
    onCopy,
    portInput,
    setPortInput,
    onSavePort,
    onOpenAdvanced,
    onOpenTest,
  } = props;

  // 注：国际站（intl）仍在开发中，此处仅统计中国站账号。
  const memberAccounts = useMemo(() => {
    const cn = state?.cnAccounts ?? [];
    return [...cn];
  }, [state?.cnAccounts]);

  const availableAccountCount = state?.cnAccounts?.length ?? 0;

  const imageUnavailableCount =
    state?.accountHealth?.filter(
      (item) => item.imageGenerationStatus === "unavailable",
    ).length ?? 0;

  const compatibilityExamples = useMemo(() => {
    const base = displayBaseUrl;
    return [
      {
        id: "chat" as const,
        title: "OpenAI Chat",
        endpoint: "POST /v1/chat/completions",
        value: `${base}/v1/chat/completions`,
        note: "OpenAI Chat Completions 协议",
      },
      {
        id: "responses" as const,
        title: "OpenAI Responses",
        endpoint: "POST /v1/responses",
        value: `${base}/v1/responses`,
        note: "OpenAI Responses 协议",
      },
      {
        id: "anthropic" as const,
        title: "Anthropic Messages",
        endpoint: "POST /v1/messages",
        value: `${base}/v1/messages`,
        note: "Anthropic Messages 协议",
      },
      {
        id: "gemini" as const,
        title: "Gemini",
        endpoint: "POST /v1beta/models",
        value: `${base}/v1beta`,
        note: "Google Gemini 协议",
      },
      {
        id: "ollama" as const,
        title: "Ollama Bridge",
        endpoint: "POST /api/chat",
        value: `${base}/api`,
        note: "Ollama 兼容协议",
      },
    ];
  }, [displayBaseUrl]);

  const scopeOptions: Array<{ value: string; label: string }> = [
    { value: "localhost", label: "localhost" },
    { value: "lan", label: "局域网(正在完善)" },
  ];

  return (
    <div className="codex-api-service-tab-panel">
      <div className="codex-api-service-grid two">
        <section className="codex-api-service-panel">
          <div className="codex-api-service-panel-head">
            <h2>服务配置</h2>
          </div>
          <div className="codex-api-service-config-list">
            <label>
              <span>Base URL</span>
              <div className="codex-api-service-copy-row">
                <code>{displayBaseUrl || "暂未提供"}</code>
                <button
                  type="button"
                  className="folder-icon-btn"
                  onClick={() => void onCopy("baseUrl", displayBaseUrl)}
                  disabled={!displayBaseUrl}
                  title="复制"
                >
                  {copiedField === "baseUrl" ? (
                    <Check size={14} />
                  ) : (
                    <Copy size={14} />
                  )}
                </button>
              </div>
            </label>

            <label>
              <span>客户端地址</span>
              <div className="codex-api-service-input-row codex-api-service-stacked-control">
                <select
                  value={collection.scope}
                  onChange={(e) =>
                    props.onUpdate({
                      scope: e.target.value as CodebuddyLocalAccessScope,
                    })
                  }
                >
                  {scopeOptions.map((opt) => (
                    <option key={opt.value} value={opt.value}>
                      {opt.label}
                    </option>
                  ))}
                </select>
                <small className="codex-api-service-field-hint">
                  CodeBuddy 反代：客户端地址由绑定地址（{collection.bindHost || "127.0.0.1"}）和访问范围组合决定。
                </small>
              </div>
            </label>

            <label>
              <span>密钥</span>
              <div className="codex-api-service-copy-row">
                <code title={firstApiKey || "-"}>
                  {firstApiKey
                    ? keyVisible
                      ? firstApiKey
                      : `${firstApiKey.slice(0, 6)}••••••${firstApiKey.slice(-4)}`
                    : "暂未提供（请在客户端 Key tab 创建）"}
                </code>
                <button
                  type="button"
                  className="folder-icon-btn"
                  onClick={() => setKeyVisible((v) => !v)}
                  disabled={!firstApiKey}
                  title="显隐"
                >
                  {keyVisible ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
                <button
                  type="button"
                  className="folder-icon-btn"
                  onClick={() => void onCopy("apiKey", firstApiKey)}
                  disabled={!firstApiKey}
                  title="复制"
                >
                  {copiedField === "apiKey" ? (
                    <Check size={14} />
                  ) : (
                    <Copy size={14} />
                  )}
                </button>
              </div>
            </label>

            <label>
              <span>服务端口</span>
              <div className="codex-api-service-input-row">
                <input
                  type="number"
                  min={1024}
                  max={65535}
                  value={portInput}
                  onChange={(e) => setPortInput(e.target.value)}
                />
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={() => void onSavePort()}
                >
                  保存端口
                </button>
              </div>
            </label>

            <label>
              <span>API 代理地址</span>
              <div className="codex-api-service-input-row codex-api-service-proxy-input-row">
                <input
                  type="text"
                  value="暂未提供"
                  disabled
                  placeholder="CodeBuddy 反代暂不支持上游代理配置"
                />
              </div>
            </label>

            <label>
              <span>高级参数</span>
              <div className="codex-api-service-input-row">
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={onOpenAdvanced}
                >
                  <SlidersHorizontal size={14} />
                  超时与重试
                </button>
              </div>
            </label>
          </div>
        </section>

        <section className="codex-api-service-panel">
          <div className="codex-api-service-panel-head">
            <h2>服务健康</h2>
          </div>
          <div className="codex-api-service-health-grid">
            <div>
              <span>可用账号</span>
              <strong>
                {availableAccountCount}/{memberAccounts.length}
              </strong>
            </div>
            <div>
              <span>冷却</span>
              <strong>-</strong>
            </div>
            <div>
              <span>图片不可用</span>
              <strong>{imageUnavailableCount}</strong>
            </div>
            <div>
              <span>客户端 Key</span>
              <strong>{collection.apiKeys.length}</strong>
            </div>
          </div>
          <div className="codex-api-service-quota-strip">
            <span>CodeBuddy 反代暂未上报配额池信息</span>
          </div>
        </section>

        <section className="codex-api-service-panel codex-api-service-compat-panel">
          <div className="codex-api-service-panel-head">
            <div>
              <h2>协议兼容</h2>
              <p className="codex-api-service-panel-desc">
                同一个 API 服务地址支持 OpenAI Chat、Responses、Anthropic Messages、Gemini 和 Ollama 入口。
              </p>
            </div>
            <div className="codex-api-service-head-actions">
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={onOpenTest}
              >
                <Send size={13} /> 多轮测试
              </button>
            </div>
          </div>
          <div className="codex-api-service-compat-grid">
            {compatibilityExamples.map((item) => {
              const copyField: CopyField = `compat:${item.id}`;
              return (
                <div key={item.id} className="codex-api-service-compat-item">
                  <div className="codex-api-service-compat-item-head">
                    <div>
                      <strong>{item.title}</strong>
                      <span>{item.endpoint}</span>
                    </div>
                    <button
                      type="button"
                      className="folder-icon-btn"
                      onClick={() => void onCopy(copyField, item.value)}
                      disabled={!displayBaseUrl}
                      title="复制"
                    >
                      {copiedField === copyField ? (
                        <Check size={14} />
                      ) : (
                        <Copy size={14} />
                      )}
                    </button>
                  </div>
                  <code>{item.value}</code>
                  <small>{item.note}</small>
                </div>
              );
            })}
          </div>
          <div className="codex-api-service-compat-models">
            <span>模型目录</span>
            <code>/v1/models · /v1beta/models · /api/tags</code>
          </div>
        </section>
      </div>
    </div>
  );
}

/* ----------------------------- Advanced params dialog ----------------------------- */

function AdvancedParamsDialog(props: {
  collection: CodebuddyLocalAccessCollection;
  onUpdate: (patch: Partial<CodebuddyLocalAccessCollection>) => void;
  onSave: () => Promise<void>;
  saving: boolean;
  onClose: () => void;
}) {
  const { collection, onUpdate, onSave, saving, onClose } = props;

  return (
    <div className="codex-api-service-modal-backdrop" onClick={onClose}>
      <div
        className="codex-api-service-modal advanced-params-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="codex-api-service-panel-head">
          <SlidersHorizontal />
          <h3>高级参数（超时与重试）</h3>
          <div className="codex-api-service-head-actions">
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={onClose}
            >
              <X size={13} /> 关闭
            </button>
          </div>
        </div>
        <div className="codex-api-service-panel-body">
          <div className="cb-api-field-grid">
            <label className="cb-api-field">
              <span>请求超时（秒）</span>
              <input
                type="number"
                min={10}
                max={600}
                value={Math.round(collection.requestTimeoutMs / 1000)}
                onChange={(e) =>
                  onUpdate({
                    requestTimeoutMs:
                      (Number(e.target.value) || 120) * 1000,
                  })
                }
              />
            </label>
            <label className="cb-api-field">
              <span>最大重试凭据数</span>
              <input
                type="number"
                min={0}
                max={10}
                value={collection.maxRetryCredentials}
                onChange={(e) =>
                  onUpdate({
                    maxRetryCredentials: Number(e.target.value) || 0,
                  })
                }
              />
            </label>
            <label className="cb-api-field">
              <span>重试间隔（毫秒）</span>
              <input
                type="number"
                min={0}
                max={60000}
                value={collection.maxRetryIntervalMs}
                onChange={(e) =>
                  onUpdate({
                    maxRetryIntervalMs: Number(e.target.value) || 0,
                  })
                }
              />
            </label>
            <label className="cb-api-field">
              <span>会话亲和 TTL（秒）</span>
              <input
                type="number"
                min={0}
                value={Math.round(
                  (collection.sessionAffinityTtlMs ?? 0) / 1000,
                )}
                onChange={(e) =>
                  onUpdate({
                    sessionAffinityTtlMs:
                      (Number(e.target.value) || 0) * 1000,
                  })
                }
              />
            </label>
            <label className="cb-api-field">
              <span>图片生成模式</span>
              <select
                value={collection.imageGenerationMode ?? "disabled"}
                onChange={(e) =>
                  onUpdate({
                    imageGenerationMode: e.target
                      .value as CodebuddyLocalAccessImageGenerationMode,
                  })
                }
              >
                <option value="disabled">关闭（Disabled）</option>
                <option value="images_only">仅图片（Images Only）</option>
                <option value="enabled">开启（Enabled）</option>
              </select>
            </label>
            <label className="cb-api-field">
              <span>单账号图片并发</span>
              <input
                type="number"
                min={1}
                max={16}
                value={collection.maxConcurrentImageRequests ?? 1}
                onChange={(e) =>
                  onUpdate({
                    maxConcurrentImageRequests: Number(e.target.value) || 1,
                  })
                }
              />
            </label>
          </div>

          <label className="cb-api-checkbox-row">
            <input
              type="checkbox"
              checked={collection.sessionAffinity}
              onChange={(e) => onUpdate({ sessionAffinity: e.target.checked })}
            />
            <span>会话亲和（同一会话固定账号）</span>
          </label>
          <label className="cb-api-checkbox-row">
            <input
              type="checkbox"
              checked={collection.disableCooling}
              onChange={(e) => onUpdate({ disableCooling: e.target.checked })}
            />
            <span>禁用账号冷却（失败后立即重试其他账号）</span>
          </label>
          <label className="cb-api-checkbox-row">
            <input
              type="checkbox"
              checked={collection.debugLogs}
              onChange={(e) => onUpdate({ debugLogs: e.target.checked })}
            />
            <span>输出调试日志</span>
          </label>

          <div className="cb-api-modal-footer">
            <button
              type="button"
              className="btn btn-primary"
              onClick={() => void onSave()}
              disabled={saving}
            >
              {saving ? (
                <RefreshCw size={14} className="loading-spinner" />
              ) : (
                <Check size={14} />
              )}
              保存并应用
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/* ----------------------------- Keys ----------------------------- */

function KeysTab(props: {
  collection: CodebuddyLocalAccessCollection;
  setState: React.Dispatch<React.SetStateAction<CodebuddyLocalAccessState | null>>;
}) {
  const { collection, setState } = props;
  const [revealed, setRevealed] = useState<Record<string, boolean>>({});
  const [newName, setNewName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

  const apply = useCallback(
    async (action: () => Promise<CodebuddyLocalAccessState>) => {
      setBusy(true);
      setError(null);
      try {
        setState(await action());
      } catch (err) {
        setError(String(err));
      } finally {
        setBusy(false);
      }
    },
    [setState],
  );

  const refresh = useCallback(async () => {
    setRefreshing(true);
    try {
      const next = await codebuddyLocalAccessService.getCodebuddyLocalAccessState();
      setState(next);
    } catch (err) {
      setError(String(err));
    } finally {
      setRefreshing(false);
    }
  }, [setState]);

  const createKey = useCallback(async () => {
    const name = newName.trim();
    if (!name) return;
    await apply(() => codebuddyLocalAccessService.createCodebuddyLocalAccessApiKey(name));
    setNewName("");
  }, [apply, newName]);

  const toggleReveal = useCallback((id: string) => {
    setRevealed((prev) => ({ ...prev, [id]: !prev[id] }));
  }, []);

  const copyKey = useCallback(async (key: string) => {
    try {
      await navigator.clipboard.writeText(key);
    } catch {
      // ignore
    }
  }, []);

  const rotateKey = useCallback(
    (id: string) =>
      apply(() => codebuddyLocalAccessService.rotateCodebuddyLocalAccessApiKey(id)),
    [apply],
  );
  const deleteKey = useCallback(
    (id: string) =>
      apply(() => codebuddyLocalAccessService.deleteCodebuddyLocalAccessApiKey(id)),
    [apply],
  );
  const toggleKey = useCallback(
    (key: CodebuddyLocalAccessApiKey) =>
      apply(() =>
        codebuddyLocalAccessService.updateCodebuddyLocalAccessApiKey(key.id, {
          enabled: !key.enabled,
        }),
      ),
    [apply],
  );

  return (
    <div className="codex-api-service-tab-panel">
      <header className="cb-api-section-header">
        <h3>客户端 Key</h3>
        <button
          type="button"
          className="cb-api-refresh-btn"
          onClick={refresh}
          disabled={refreshing}
          title="刷新 Key 列表"
        >
          <RefreshCw size={14} className={refreshing ? "cb-api-spin" : ""} />
        </button>
      </header>
      {error && (
        <div className="cb-api-banner error">
          <CircleAlert />
          <span>{error}</span>
        </div>
      )}
      <section className="codex-api-service-panel">
        <div className="codex-api-service-panel-head">
          <Plus />
          <h3>新建客户端 Key</h3>
        </div>
        <div className="cb-api-panel-body">
          <div className="cb-api-connect-row">
            <input
              className="cb-api-base-url"
              type="text"
              placeholder="Key 名称（如 codex-cli、claude-code）"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              style={{ flex: 1 }}
            />
            <button
              type="button"
              className="btn btn-primary btn-sm"
              onClick={createKey}
              disabled={busy || !newName.trim()}
            >
              <Plus size={14} /> 创建
            </button>
          </div>
          <p className="cb-api-hint">
            第三方客户端用这里的 Key 作为 <code>Authorization: Bearer &lt;key&gt;</code> 接入本地网关。
          </p>
        </div>
      </section>

      <section className="codex-api-service-panel">
        <div className="codex-api-service-panel-head">
          <KeyRound />
          <h3>已有 Key（{collection.apiKeys.length}）</h3>
        </div>
        <div className="codex-api-service-table">
          {collection.apiKeys.length === 0 ? (
            <div className="cb-api-log-empty">暂无客户端 Key，请先创建。</div>
          ) : (
            <table className="cb-api-key-table">
              <thead>
                <tr>
                  <th>名称</th>
                  <th>Key</th>
                  <th>状态</th>
                  <th>账号绑定</th>
                  <th style={{ textAlign: "right" }}>操作</th>
                </tr>
              </thead>
              <tbody>
                {collection.apiKeys.map((key) => (
                  <tr key={key.id}>
                    <td>{key.name}</td>
                    <td>
                      <span className="cb-api-key-value">
                        {revealed[key.id] ? key.key : maskKey(key.key)}
                        <button
                          type="button"
                          className="cb-api-btn sm"
                          onClick={() => toggleReveal(key.id)}
                          title="显隐"
                        >
                          {revealed[key.id] ? <EyeOff size={13} /> : <Eye size={13} />}
                        </button>
                        <button
                          type="button"
                          className="cb-api-btn sm"
                          onClick={() => copyKey(key.key)}
                          title="复制"
                        >
                          <Copy size={13} />
                        </button>
                      </span>
                    </td>
                    <td>
                      <span className={`cb-api-badge ${key.enabled ? "enabled" : "disabled"}`}>
                        {key.enabled ? "启用" : "停用"}
                      </span>
                    </td>
                    <td>
                      {key.accountIds && key.accountIds.length > 0
                        ? `${key.accountIds.length} 个账号`
                        : "全部账号"}
                    </td>
                    <td>
                      <div className="cb-api-key-actions" style={{ justifyContent: "flex-end" }}>
                        <button
                          type="button"
                          className="cb-api-btn sm"
                          onClick={() => toggleKey(key)}
                        >
                          {key.enabled ? "停用" : "启用"}
                        </button>
                        <button
                          type="button"
                          className="cb-api-btn sm"
                          onClick={() => rotateKey(key.id)}
                        >
                          <RefreshCw size={13} /> 轮换
                        </button>
                        <button
                          type="button"
                          className="cb-api-btn sm danger"
                          onClick={() => deleteKey(key.id)}
                        >
                          <Trash2 size={13} /> 删除
                        </button>
                      </div>
                    </td>
                  </tr>
                  ))}
              </tbody>
            </table>
          )}
        </div>
      </section>
    </div>
  );
}

/* ----------------------------- Accounts ----------------------------- */

const ROUTING_STRATEGY_OPTIONS: Array<{
  value: CodebuddyLocalAccessRoutingStrategy;
  label: string;
}> = [
  { value: "auto", label: "自动（推荐）" },
  { value: "random", label: "随机分散" },
  { value: "single_account", label: "固定首个账号" },
  { value: "quota_high_first", label: "优先高配额" },
  { value: "quota_low_first", label: "优先低配额" },
  { value: "plan_high_first", label: "优先高订阅" },
  { value: "plan_low_first", label: "优先低订阅" },
  { value: "expiry_soon_first", label: "优先近到期" },
  { value: "custom", label: "自定义" },
];

function AccountsTab(props: {
  state: CodebuddyLocalAccessState | null;
  collection: CodebuddyLocalAccessCollection;
  setState: React.Dispatch<React.SetStateAction<CodebuddyLocalAccessState | null>>;
  onUpdate: (patch: Partial<CodebuddyLocalAccessCollection>) => void;
  onSave: () => Promise<void> | void;
  saving: boolean;
  stats: CodebuddyLocalAccessStats | null;
  statsLoading: boolean;
  onReloadStats: () => Promise<void> | void;
  onOpenAddAccount: () => void;
  onOpenMemberModal: () => void;
  onJumpToModels: () => void;
}) {
  const {
    state,
    collection,
    setState,
    onUpdate,
    onSave,
    saving,
    stats,
    statsLoading,
    onReloadStats,
    onOpenAddAccount,
    onOpenMemberModal,
    onJumpToModels,
  } = props;
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setRefreshing(true);
    setError(null);
    try {
      const next = await codebuddyLocalAccessService.getCodebuddyLocalAccessState();
      setState(next);
      void onReloadStats();
    } catch (err) {
      setError(String(err));
    } finally {
      setRefreshing(false);
    }
  }, [setState, onReloadStats]);

  // 仅渲染池内成员（对齐 Codex 卡片网格：左栏只列已加入账号池的账号）。
  // 注：国际站（intl）仍在开发中，此处仅展示中国站账号。
  const memberAccounts = useMemo(() => {
    const cn = state?.cnAccounts ?? [];
    const selected = new Set(collection.cnAccountIds ?? []);
    return cn.filter((a) => selected.has(a.id));
  }, [state?.cnAccounts, collection.cnAccountIds]);

  // 自定义路由所需：池内成员的完整账号对象。
  const selectedAccountOptions = memberAccounts;

  const usageByAccount = useMemo(() => {
    const map = new Map<string, CodebuddyLocalAccessUsageStats>();
    if (stats?.byAccount) {
      for (const entry of stats.byAccount) {
        map.set(entry.accountId, entry.usage);
      }
    }
    return map;
  }, [stats?.byAccount]);

  const healthByAccount = useMemo(() => {
    const map = new Map<string, CodebuddyLocalAccessAccountHealth>();
    if (state?.accountHealth) {
      for (const item of state.accountHealth) {
        map.set(item.accountId, item);
      }
    }
    return map;
  }, [state?.accountHealth]);

  // 移出账号池（对齐 Codex 删除按钮：从 intlAccountIds / cnAccountIds 过滤后保存）。
  const handleRemoveMember = useCallback(
    (accountId: string) => {
      setState((prev) => {
        if (!prev) return prev;
        const nextIntl = (prev.collection.intlAccountIds ?? []).filter(
          (id) => id !== accountId,
        );
        const nextCn = (prev.collection.cnAccountIds ?? []).filter(
          (id) => id !== accountId,
        );
        return {
          ...prev,
          collection: {
            ...prev.collection,
            intlAccountIds: nextIntl,
            cnAccountIds: nextCn,
          },
        };
      });
      void onSave();
    },
    [setState, onSave],
  );

  const handleSave = useCallback(() => {
    void onSave();
  }, [onSave]);

  return (
    <div className="codex-api-service-tab-panel">
      <header className="cb-api-section-header">
        <h3>账号池</h3>
        <button
          type="button"
          className="cb-api-refresh-btn"
          onClick={refresh}
          disabled={refreshing || statsLoading}
          title="刷新账号列表与统计"
        >
          <RefreshCw
            size={14}
            className={refreshing || statsLoading ? "cb-api-spin" : ""}
          />
        </button>
      </header>
      {error && (
        <div className="cb-api-banner error">
          <CircleAlert />
          <span>{error}</span>
        </div>
      )}
      <div className="codex-api-service-grid accounts">
        {/* 左栏：按账号统计（Codex 风格账号卡片网格 + 工具栏） */}
        <section className="codex-api-service-panel">
          <div className="codex-api-service-panel-head">
            <h3>按账号统计</h3>
            <div className="codex-api-service-head-actions">
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={onOpenAddAccount}
                disabled={saving || refreshing}
                title="添加账号"
              >
                <Plus size={14} />
                添加账号
              </button>
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={onJumpToModels}
                disabled={saving || refreshing}
                title="模型映射"
              >
                <Route size={14} />
                模型映射
              </button>
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={onJumpToModels}
                disabled={saving || refreshing}
                title="禁用模型"
              >
                <Wrench size={14} />
                禁用模型
              </button>
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={onOpenMemberModal}
                disabled={saving || refreshing}
                title="管理成员"
              >
                <FolderPlus size={14} />
                管理成员
              </button>
            </div>
          </div>
          <div className="codex-api-service-account-grid">
            {memberAccounts.length === 0 ? (
              <div className="codex-api-service-empty">
                <p>暂无池内成员，请添加账号或管理成员。</p>
                <div className="codex-api-service-empty-actions">
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={onOpenAddAccount}
                    disabled={saving || refreshing}
                  >
                    <Plus size={14} />
                    添加账号
                  </button>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={onOpenMemberModal}
                    disabled={saving || refreshing}
                  >
                    <FolderPlus size={14} />
                    管理成员
                  </button>
                </div>
              </div>
            ) : (
              memberAccounts.map((account) => {
                const usage = usageByAccount.get(account.id);
                const health = healthByAccount.get(account.id);
                return (
                  <AccountStatsCard
                    key={account.id}
                    account={account}
                    usage={usage}
                    health={health}
                    onRemove={() => handleRemoveMember(account.id)}
                  />
                );
              })
            )}
          </div>
        </section>

        {/* 右栏：调度选项（保存选项按钮在右上角，对齐 Codex） */}
        <section className="codex-api-service-panel">
          <div className="codex-api-service-panel-head">
            <Route />
            <h3>调度选项</h3>
            <div className="codex-api-service-head-actions">
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                onClick={handleSave}
                disabled={saving}
              >
                {saving ? (
                  <RefreshCw size={14} className="loading-spinner" />
                ) : (
                  <Check size={14} />
                )}
                保存选项
              </button>
            </div>
          </div>
          <div className="cb-api-panel-body">
            <RoutingOptionsEditor
              collection={collection}
              selectedAccountOptions={selectedAccountOptions}
              onUpdate={onUpdate}
              saving={saving}
            />
          </div>
        </section>
      </div>
    </div>
  );
}

function AccountStatsCard(props: {
  account: CodebuddyLocalAccessAccountOption;
  usage: CodebuddyLocalAccessUsageStats | undefined;
  health: CodebuddyLocalAccessAccountHealth | undefined;
  onRemove: () => void;
}) {
  const { account, usage, health, onRemove } = props;
  const requestCount = usage?.requestCount ?? 0;
  const totalTokens = usage?.totalTokens ?? 0;
  const successCount = usage?.successCount ?? 0;
  const failureCount = usage?.failureCount ?? 0;
  const canceledCount = usage?.clientCanceledCount ?? 0;
  const upstreamFailedCount = usage?.upstreamResponseFailedCount ?? 0;
  const streamIncompleteCount = usage?.streamIncompleteCount ?? 0;
  // 失败数 = 总失败 - 取消 - 上游失败 - 流未完成（对齐 Codex formatRequestResultDetail）。
  const plainFailureCount = Math.max(
    0,
    failureCount - canceledCount - upstreamFailedCount - streamIncompleteCount,
  );
  const consecutiveFailures = health?.consecutiveFailures ?? 0;
  const cooldowns = health?.cooldowns ?? [];
  const available = health?.available ?? true;
  const lastFailureCategory = health?.lastFailureCategory ?? null;
  const imageStatus = health?.imageGenerationStatus ?? "unknown";

  const planClass = account.planClass ?? "free";
  const planLabel = (account.planType ?? "free").toUpperCase();

  const healthLabel = formatHealthLabel(
    available,
    cooldowns.length,
    lastFailureCategory,
  );
  const imageLabel = formatImageStatusLabel(imageStatus);

  return (
    <div className="codex-api-service-account-card cb-account-card">
      <div>
        <strong title={account.email || account.id}>
          {account.email || account.id}
        </strong>
        <span className={`tier-badge ${planClass}`}>{planLabel}</span>
      </div>
      <div className="codex-api-service-account-meta">
        <span>{formatCompactNumber(requestCount)} 次</span>
        <span className="codex-api-service-account-meta-token">
          {formatCompactNumber(totalTokens)} Tokens
        </span>
        <span>
          成功 {formatCompactNumber(successCount)} / 失败{" "}
          {formatCompactNumber(plainFailureCount)} / 取消{" "}
          {formatCompactNumber(canceledCount)} / 上游失败{" "}
          {formatCompactNumber(upstreamFailedCount)} / 流未完成{" "}
          {formatCompactNumber(streamIncompleteCount)}
        </span>
        <span>连续失败 {formatCompactNumber(consecutiveFailures)}</span>
        <span className={healthLabel === "可用" ? "codex-api-service-account-meta-token" : ""}>
          {healthLabel}
        </span>
        <span>图片 {imageLabel}</span>
      </div>
      <button
        type="button"
        className="folder-icon-btn cb-account-remove-btn"
        onClick={onRemove}
        title="移出账号池"
        aria-label="移出账号池"
      >
        <Trash2 size={14} />
      </button>
    </div>
  );
}

function RoutingOptionsEditor(props: {
  collection: CodebuddyLocalAccessCollection;
  selectedAccountOptions: CodebuddyLocalAccessAccountOption[];
  onUpdate: (patch: Partial<CodebuddyLocalAccessCollection>) => void;
  saving: boolean;
}) {
  const { collection, selectedAccountOptions, onUpdate, saving } = props;
  return (
    <div className="codex-api-service-config-list codex-api-service-routing-form">
      <label>
        <span>调度策略</span>
        <SingleSelectDropdown
          value={collection.routingStrategy}
          options={ROUTING_STRATEGY_OPTIONS}
          onChange={(value) =>
            onUpdate({
              routingStrategy: value as CodebuddyLocalAccessRoutingStrategy,
            })
          }
          disabled={saving}
          ariaLabel="账号池调度策略"
        />
      </label>
      <label className="codex-api-service-checkbox-row">
        <input
          type="checkbox"
          checked={collection.sessionAffinity}
          onChange={(e) => onUpdate({ sessionAffinity: e.target.checked })}
          disabled={saving}
        />
        <span>会话亲和（同一会话稳定路由到同一账号，最大化命中缓存）</span>
      </label>
      <label>
        <span>过期时间（秒）</span>
        <input
          type="number"
          min={60}
          max={86400}
          value={Math.round((collection.sessionAffinityTtlMs ?? 1_800_000) / 1000)}
          onChange={(e) =>
            onUpdate({
              sessionAffinityTtlMs:
                Math.max(60, Math.min(86400, Number(e.target.value) || 1800)) * 1000,
            })
          }
          disabled={saving}
        />
      </label>
      <label className="codex-api-service-checkbox-row">
        <input
          type="checkbox"
          checked={collection.responsesWebsocketsEnabled ?? false}
          onChange={(e) =>
            onUpdate({ responsesWebsocketsEnabled: e.target.checked })
          }
          disabled={saving}
        />
        <span>WebSocket 响应传输（客户端 Key 启用 Responses WS 端点）</span>
      </label>
      <label>
        <span>重试账号数</span>
        <input
          type="number"
          min={0}
          max={8}
          value={collection.maxRetryCredentials ?? 2}
          onChange={(e) =>
            onUpdate({
              maxRetryCredentials: Math.max(0, Math.min(8, Number(e.target.value) || 0)),
            })
          }
          disabled={saving}
        />
      </label>
      <label>
        <span>重试等待（秒）</span>
        <input
          type="number"
          min={0}
          max={30}
          value={Math.round((collection.maxRetryIntervalMs ?? 2000) / 1000)}
          onChange={(e) =>
            onUpdate({
              maxRetryIntervalMs:
                Math.max(0, Math.min(30, Number(e.target.value) || 0)) * 1000,
            })
          }
          disabled={saving}
        />
      </label>
      <label className="codex-api-service-checkbox-row">
        <input
          type="checkbox"
          checked={collection.disableCooling ?? false}
          onChange={(e) => onUpdate({ disableCooling: e.target.checked })}
          disabled={saving}
        />
        <span>禁用冷却（失败后立即重试其他账号）</span>
      </label>
      <label className="codex-api-service-checkbox-row">
        <input
          type="checkbox"
          checked={collection.immediateSseResponse ?? false}
          onChange={(e) => onUpdate({ immediateSseResponse: e.target.checked })}
          disabled={saving}
        />
        <span>SSE 立即返回 200（先写 SSE 头再转发上游，降低客户端感知延迟）</span>
      </label>
      <label>
        <span>每账户图片请求数</span>
        <input
          type="number"
          min={1}
          max={16}
          value={collection.maxConcurrentImageRequests ?? 1}
          onChange={(e) =>
            onUpdate({
              maxConcurrentImageRequests: Math.max(1, Math.min(16, Number(e.target.value) || 1)),
            })
          }
          disabled={saving}
        />
      </label>
      {collection.routingStrategy === "custom" && (
        <CustomRoutingRulesEditor
          selectedAccounts={selectedAccountOptions}
          rules={collection.customRoutingRules ?? []}
          onChange={(rules) => onUpdate({ customRoutingRules: rules })}
          saving={saving}
        />
      )}
    </div>
  );
}

function MemberModal(props: {
  state: CodebuddyLocalAccessState | null;
  collection: CodebuddyLocalAccessCollection;
  saving: boolean;
  onClose: () => void;
  onSave: (nextIntlIds: string[], nextCnIds: string[]) => Promise<void> | void;
}) {
  const { state, collection, saving, onClose, onSave } = props;
  // 国际站（intl）仍在开发中，UI 已隐藏，但保留其历史选择值以便保存时原样回传（可逆）。
  const intlDraft = new Set(collection.intlAccountIds ?? []);
  const [cnDraft, setCnDraft] = useState<Set<string>>(
    () => new Set(collection.cnAccountIds ?? []),
  );

  const toggleCn = useCallback((accountId: string) => {
    setCnDraft((prev) => {
      const next = new Set(prev);
      if (next.has(accountId)) {
        next.delete(accountId);
      } else {
        next.add(accountId);
      }
      return next;
    });
  }, []);

  const handleConfirm = useCallback(() => {
    void onSave(Array.from(intlDraft), Array.from(cnDraft));
  }, [onSave, intlDraft, cnDraft]);

  const cnAccounts = state?.cnAccounts ?? [];

  return (
    <div className="codex-api-service-modal-overlay" onClick={onClose}>
      <div
        className="codex-api-service-modal cb-member-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="codex-api-service-modal-head">
          <h2>管理成员</h2>
          <button
            type="button"
            className="folder-icon-btn"
            onClick={onClose}
            aria-label="关闭"
            title="关闭"
          >
            <X size={14} />
          </button>
        </div>
        <div className="codex-api-service-modal-body">
          {cnAccounts.length === 0 ? (
            <p className="cb-api-hint">
              暂无已登录账号，请先在 CodeBuddy 账号页面登录。
            </p>
          ) : (
            <>
              {cnAccounts.length > 0 && (
                <div className="cb-member-group">
                  <h3>中国站（cn）</h3>
                  {cnAccounts.map((account) => (
                    <label
                      key={account.id}
                      className="cb-member-row"
                    >
                      <input
                        type="checkbox"
                        checked={cnDraft.has(account.id)}
                        onChange={() => toggleCn(account.id)}
                        disabled={saving}
                      />
                      <span title={account.email || account.id}>
                        {account.email || account.id}
                      </span>
                      <span className={`tier-badge ${account.planClass ?? "free"}`}>
                        {(account.planType ?? "free").toUpperCase()}
                      </span>
                    </label>
                  ))}
                </div>
              )}
            </>
          )}
        </div>
        <div className="codex-api-service-modal-foot">
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={onClose}
            disabled={saving}
          >
            取消
          </button>
          <button
            type="button"
            className="btn btn-primary btn-sm"
            onClick={handleConfirm}
            disabled={saving}
          >
            {saving ? (
              <RefreshCw size={14} className="loading-spinner" />
            ) : (
              <Check size={14} />
            )}
            确认
          </button>
        </div>
      </div>
    </div>
  );
}

function CustomRoutingRulesEditor(props: {
  selectedAccounts: CodebuddyLocalAccessAccountOption[];
  rules: CodebuddyLocalAccessCustomRoutingRule[];
  onChange: (rules: CodebuddyLocalAccessCustomRoutingRule[]) => void;
  saving: boolean;
}) {
  const { selectedAccounts, rules, onChange, saving } = props;

  const ruleByAccount = useMemo(() => {
    const map = new Map<string, CodebuddyLocalAccessCustomRoutingRule>();
    for (const rule of rules) {
      map.set(rule.accountId, rule);
    }
    return map;
  }, [rules]);

  const updateRule = (
    accountId: string,
    patch: Partial<Omit<CodebuddyLocalAccessCustomRoutingRule, "accountId">>,
  ) => {
    const existing = ruleByAccount.get(accountId) ?? {
      accountId,
      priority: 0,
      weight: 1,
      isBackup: false,
      isPreferred: false,
    };
    const updated = { ...existing, ...patch };
    const next = new Map(ruleByAccount);
    next.set(accountId, updated);
    onChange(Array.from(next.values()));
  };

  if (selectedAccounts.length === 0) {
    return (
      <p className="cb-api-hint">选择账号后可配置自定义路由优先级、权重与备份偏好。</p>
    );
  }

  return (
    <div className="cb-custom-rules">
      <div className="cb-custom-rules-head">
        <span>自定义路由规则（priority 越大越优先，同组按 weight 加权轮询）</span>
      </div>
      {selectedAccounts.map((account) => {
        const rule = ruleByAccount.get(account.id) ?? {
          accountId: account.id,
          priority: 0,
          weight: 1,
          isBackup: false,
          isPreferred: false,
        };
        return (
          <div key={account.id} className="cb-custom-rule-row">
            <span className="cb-custom-rule-account" title={account.id}>
              {account.email || account.id}
            </span>
            <label className="cb-custom-rule-field">
              <span>优先级</span>
              <input
                type="number"
                value={rule.priority}
                min={-999}
                max={999}
                onChange={(e) =>
                  updateRule(account.id, { priority: Number(e.target.value) || 0 })
                }
                disabled={saving}
              />
            </label>
            <label className="cb-custom-rule-field">
              <span>权重</span>
              <input
                type="number"
                value={rule.weight}
                min={1}
                max={100}
                onChange={(e) =>
                  updateRule(account.id, {
                    weight: Math.max(1, Number(e.target.value) || 1),
                  })
                }
                disabled={saving}
              />
            </label>
            <label className="cb-custom-rule-check">
              <input
                type="checkbox"
                checked={rule.isPreferred}
                onChange={(e) =>
                  updateRule(account.id, { isPreferred: e.target.checked })
                }
                disabled={saving}
              />
              <span>偏好</span>
            </label>
            <label className="cb-custom-rule-check">
              <input
                type="checkbox"
                checked={rule.isBackup}
                onChange={(e) =>
                  updateRule(account.id, { isBackup: e.target.checked })
                }
                disabled={saving}
              />
              <span>备份</span>
            </label>
          </div>
        );
      })}
    </div>
  );
}

/* ----------------------------- Models ----------------------------- */

function ModelsTab(props: {
  collection: CodebuddyLocalAccessCollection;
  onUpdate: (patch: Partial<CodebuddyLocalAccessCollection>) => void;
  onRefresh?: () => Promise<void> | void;
  baseUrl?: string;
  firstApiKey?: string;
  onToggleVisionTool?: (enabled: boolean) => Promise<void> | void;
}) {
  const {
    collection,
    onUpdate,
    onRefresh,
    baseUrl = "",
    firstApiKey = "",
    onToggleVisionTool,
  } = props;
  const [aliasText, setAliasText] = useState(
    collection.modelAliases.map((a) => `${a.sourceModel}->${a.alias}`).join("\n"),
  );
  const [excludedText, setExcludedText] = useState(collection.excludedModels.join("\n"));
  const [refreshing, setRefreshing] = useState(false);

  // ─── 可用模型卡片 state ───
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [selectedModel, setSelectedModel] = useState<string | null>(null);
  const [syncing, setSyncing] = useState(false);
  const [copiedModel, setCopiedModel] = useState(false);
  const [priceEditing, setPriceEditing] = useState(false);

  const loadAvailableModels = useCallback(async () => {
    if (!baseUrl) {
      setAvailableModels([]);
      return;
    }
    setModelsLoading(true);
    setModelsError(null);
    try {
      const headers: Record<string, string> = firstApiKey
        ? { Authorization: `Bearer ${firstApiKey}` }
        : {};
      const resp = await fetch(`${baseUrl.replace(/\/$/, "")}/v1/models`, { headers });
      if (!resp.ok) throw new Error(`/v1/models 返回 ${resp.status}`);
      const data = await resp.json();
      const ids: string[] = Array.isArray(data?.data)
        ? data.data
            .map((m: any) => (typeof m?.id === "string" ? m.id : ""))
            .filter((s: string) => s.length > 0)
        : [];
      setAvailableModels(ids);
    } catch (err) {
      console.warn("拉取可用模型失败:", err);
      setModelsError(String(err));
      setAvailableModels([]);
    } finally {
      setModelsLoading(false);
    }
  }, [baseUrl, firstApiKey]);

  useEffect(() => {
    void loadAvailableModels();
  }, [loadAvailableModels]);

  const syncModels = useCallback(async () => {
    if (!baseUrl || !firstApiKey) return;
    setSyncing(true);
    setModelsError(null);
    try {
      const resp = await fetch(
        `${baseUrl.replace(/\/$/, "")}/v1/cockpit/codebuddy/sync`,
        {
          method: "POST",
          headers: { Authorization: `Bearer ${firstApiKey}` },
        },
      );
      if (!resp.ok) throw new Error(`同步请求返回 ${resp.status}`);
      await loadAvailableModels();
      if (onRefresh) await onRefresh();
    } catch (err) {
      console.error("手动同步模型失败:", err);
      setModelsError(String(err));
    } finally {
      setSyncing(false);
    }
  }, [baseUrl, firstApiKey, loadAvailableModels, onRefresh]);

  const copySelectedModel = useCallback(async () => {
    if (!selectedModel) return;
    try {
      await navigator.clipboard.writeText(selectedModel);
      setCopiedModel(true);
      setTimeout(() => setCopiedModel(false), 1500);
    } catch {
      // ignore
    }
  }, [selectedModel]);

  const refresh = useCallback(async () => {
    if (!onRefresh) return;
    setRefreshing(true);
    try {
      await onRefresh();
    } finally {
      setRefreshing(false);
    }
  }, [onRefresh]);

  const applyAliases = useCallback(() => {
    const aliases = aliasText
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean)
      .map((line) => {
        const [sourceModel, alias] = line.split("->").map((s) => s.trim());
        return { sourceModel: sourceModel ?? "", alias: alias ?? "", fork: false };
      })
      .filter((a) => a.sourceModel && a.alias);
    onUpdate({ modelAliases: aliases });
  }, [aliasText, onUpdate]);

  const applyExcluded = useCallback(() => {
    onUpdate({
      excludedModels: excludedText
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean),
    });
  }, [excludedText, onUpdate]);

  return (
    <div className="codex-api-service-tab-panel">
      <header className="cb-api-section-header">
        <h3>模型与能力</h3>
        <span className="cb-api-section-hint">
          别名 {collection.modelAliases.length} 个 / 排除 {collection.excludedModels.length} 个
        </span>
        {onRefresh && (
          <button
            type="button"
            className="cb-api-refresh-btn"
            onClick={refresh}
            disabled={refreshing}
            title="刷新模型列表"
          >
            <RefreshCw size={14} className={refreshing ? "cb-api-spin" : ""} />
          </button>
        )}
      </header>

      {/* ─────────── 纯文本模型视觉子代理开关 ─────────── */}
      <section className="codex-api-service-panel cb-vision-agentic-card">
        <div className="codex-api-service-panel-head">
          <Eye />
          <h3>纯文本模型视觉子代理</h3>
          <div className="codex-api-service-head-actions">
            <button
              type="button"
              role="switch"
              aria-checked={collection.visionToolEnabled ?? false}
              className={`cb-vision-toggle ${collection.visionToolEnabled ? "on" : ""}`}
              onClick={() => void onToggleVisionTool?.(!(collection.visionToolEnabled ?? false))}
              title={
                collection.visionToolEnabled
                  ? "点击关闭视觉子代理"
                  : "点击开启视觉子代理"
              }
            >
              <span className="cb-vision-toggle-knob" />
            </button>
          </div>
        </div>
        <div className="codex-api-service-panel-body">
          <p className="cb-vision-agentic-desc">
            {collection.visionToolEnabled
              ? "已开启：纯文本模型（如 deepseek）可接收图片，并在推理中自主调用混元视觉模型（hy3-preview）充当「眼睛」，反复查看图片细节。"
              : "已关闭：纯文本模型无法接收图片（客户端会过滤图片输入）。开启后，deepseek 等纯文本模型能通过内部子代理调用混元免费视觉模型看图。"}
          </p>
        </div>
      </section>

      {/* ─────────── 可用模型卡片 ─────────── */}
      <section className="codex-api-service-panel">
        <div className="codex-api-service-panel-head">
          <Layers />
          <h3>可用模型</h3>
          <div className="codex-api-service-head-actions">
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={() => void syncModels()}
              disabled={syncing || !baseUrl || !firstApiKey}
              title="从本机官方客户端 app.asar 重新提取模型清单"
            >
              <RefreshCw size={13} className={syncing ? "cb-api-spin" : ""} />
              <span style={{ marginLeft: 4 }}>同步模型</span>
            </button>
            <button
              type="button"
              className={`btn btn-secondary btn-sm${priceEditing ? " active" : ""}`}
              onClick={() => setPriceEditing((v) => !v)}
              title="设置模型计费价格"
            >
              <DollarSign size={13} />
              <span style={{ marginLeft: 4 }}>价格设置</span>
            </button>
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={() => void copySelectedModel()}
              disabled={!selectedModel}
              title="复制选中模型名称"
            >
              {copiedModel ? <Check size={13} /> : <Copy size={13} />}
            </button>
          </div>
        </div>
        <div className="cb-api-panel-body">
          {modelsLoading ? (
            <p className="cb-api-hint">
              <RefreshCw size={12} className="cb-api-spin" style={{ verticalAlign: -2 }} />{" "}
              正在拉取模型清单…
            </p>
          ) : modelsError ? (
            <p className="cb-api-hint" style={{ color: "var(--danger)" }}>
              拉取失败：{modelsError}
            </p>
          ) : availableModels.length === 0 ? (
            <p className="cb-api-hint">暂无可用模型（请先启动服务并点击「同步模型」）</p>
          ) : (
            <div className="cb-api-model-pill-list">
              {availableModels.map((m) => (
                <button
                  key={m}
                  type="button"
                  className={`cb-api-model-pill${selectedModel === m ? " is-selected" : ""}`}
                  onClick={() => setSelectedModel(m)}
                  title={m}
                >
                  {m}
                </button>
              ))}
            </div>
          )}
          {priceEditing && (
            <p className="cb-api-hint" style={{ marginTop: 10 }}>
              价格设置占位（待接入计费模块）。当前选中：
              <code style={{ marginLeft: 4 }}>{selectedModel ?? "（未选择）"}</code>
            </p>
          )}
        </div>
      </section>

      <section className="codex-api-service-panel">
        <div className="codex-api-service-panel-head">
          <Layers />
          <h3>模型别名</h3>
          <div className="codex-api-service-head-actions">
            <button type="button" className="btn btn-secondary btn-sm" onClick={applyAliases}>
              应用别名
            </button>
          </div>
        </div>
        <div className="cb-api-panel-body">
          <label className="cb-api-field">
            <span>每行一个：源模型-&gt;别名</span>
            <textarea
              value={aliasText}
              onChange={(e) => setAliasText(e.target.value)}
              placeholder={"deepseek-v4-flash->ds-v4\nglm-5.2->glm"}
            />
          </label>
          <p className="cb-api-hint">
            别名用于第三方客户端用自定义模型名调用，网关会自动映射到真实 CodeBuddy 模型。
          </p>
        </div>
      </section>

      <section className="codex-api-service-panel">
        <div className="codex-api-service-panel-head">
          <X />
          <h3>排除模型</h3>
          <div className="codex-api-service-head-actions">
            <button type="button" className="btn btn-secondary btn-sm" onClick={applyExcluded}>
              应用排除
            </button>
          </div>
        </div>
        <div className="cb-api-panel-body">
          <label className="cb-api-field">
            <span>每行一个模型 ID</span>
            <textarea
              value={excludedText}
              onChange={(e) => setExcludedText(e.target.value)}
              placeholder={"kimi-k2.5\nminimax-m3-pay"}
            />
          </label>
          <p className="cb-api-hint">被排除的模型不会暴露给第三方客户端。</p>
        </div>
      </section>
    </div>
  );
}

/* ----------------------------- Logs ----------------------------- */

// renderModelLabel renders the request-log model cell. When the request was
// handled by the pure-text vision sub-agent (visionSubagent flag), a blue
// "视" pill is appended to the plain model label.
function renderModelLabel(model: string | null | undefined, visionSubagent?: boolean) {
  if (!model) return "-";
  if (!visionSubagent) return model;
  return (
    <>
      {model}
      <span className="cb-api-vision-badge" title="由「纯文本视觉子代理」处理（主模型 + 视觉模型协作）">视</span>
    </>
  );
}

function LogsTab() {
  const { t } = useTranslation();
  const [stats, setStats] = useState<CodebuddyLocalAccessStats | null>(null);
  const [logs, setLogs] = useState<CodebuddyLocalAccessLogPage | null>(null);
  const [page, setPage] = useState(1);
  const [modelFilter, setModelFilter] = useState("");
  const [successFilter, setSuccessFilter] = useState<string>("");
  const [statsLoading, setStatsLoading] = useState(false);
  const [logsLoading, setLogsLoading] = useState(false);
  const [clearStatsConfirmOpen, setClearStatsConfirmOpen] = useState(false);
  const [clearingStats, setClearingStats] = useState(false);

  const loadLogs = useCallback(async () => {
    setLogsLoading(true);
    try {
      setLogs(
        await codebuddyLocalAccessService.getCodebuddyLocalAccessLogs(
          page,
          20,
          modelFilter || undefined,
          undefined,
          successFilter === "" ? undefined : successFilter === "ok",
        ),
      );
    } catch {
      // ignore
    } finally {
      setLogsLoading(false);
    }
  }, [page, modelFilter, successFilter]);

  const loadStats = useCallback(async () => {
    setStatsLoading(true);
    try {
      setStats(await codebuddyLocalAccessService.getCodebuddyLocalAccessStats());
    } catch {
      // ignore
    } finally {
      setStatsLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadLogs();
  }, [loadLogs]);

  useEffect(() => {
    void loadStats();
  }, [loadStats]);

  const refreshAll = useCallback(async () => {
    await Promise.all([loadStats(), loadLogs()]);
  }, [loadStats, loadLogs]);

  const clearStats = useCallback(async () => {
    if (clearingStats) return;
    setClearingStats(true);
    try {
      setStats(await codebuddyLocalAccessService.clearCodebuddyLocalAccessStats());
      setModelFilter("");
      setPage(1);
      void loadLogs();
    } catch {
      // ignore
    } finally {
      setClearingStats(false);
      setClearStatsConfirmOpen(false);
    }
  }, [clearingStats, loadLogs]);

  const totals = stats?.totals;

  // 「按模型筛选」下拉框数据源：复用统计接口 byModel（后端已按请求量降序、去重统计），
  //选项与日志中出现过的模型保持一致。
  const modelOptions = (stats?.byModel ?? [])
  .map((m) => m.modelId)
  .filter((id) => id.length >0);

  const summaryCards = [
    { key: "requests", label: "总请求", value: String(totals?.requestCount ?? 0) },
    { key: "success", label: "成功", value: String(totals?.successCount ?? 0), tone: "ok" as const },
    { key: "failure", label: "失败", value: String(totals?.failureCount ?? 0), tone: "err" as const },
    { key: "input", label: "输入 Token", value: formatNumber(totals?.inputTokens ?? 0) },
    { key: "output", label: "输出 Token", value: formatNumber(totals?.outputTokens ?? 0) },
    { key: "credit", label: "Credit", value: formatCredit(totals?.totalCredit ?? 0), tone: "credit" as const },
  ];

  return (
    <div className="codex-api-service-tab-panel">
      <header className="cb-api-section-header">
        <h3>统计与日志</h3>
        <button
          type="button"
          className="cb-api-refresh-btn"
          onClick={refreshAll}
          disabled={statsLoading || logsLoading}
          title="刷新统计与日志"
        >
          <RefreshCw size={14} className={statsLoading || logsLoading ? "cb-api-spin" : ""} />
        </button>
      </header>

      <section className="codex-api-service-panel">
        <div className="codex-api-service-panel-head">
          <BarChart3 />
          <h3>用量统计</h3>
          <div className="codex-api-service-head-actions">
            <button
              type="button"
              className="cb-api-btn danger"
              onClick={() => setClearStatsConfirmOpen(true)}
              disabled={clearingStats}
            >
              <Trash2 size={14} /> 清空统计
            </button>
          </div>
        </div>
        <div className="cb-api-panel-body">
          <div className="cb-api-summary-grid">
            {summaryCards.map((c) => (
              <SummaryCard
                key={c.key}
                icon={<BarChart3 />}
                label={c.label}
                value={c.value}
                tone={c.tone}
              />
            ))}
          </div>
        </div>
      </section>

      <section className="codex-api-service-panel">
        <div className="codex-api-service-panel-head">
          <ScrollText />
          <h3>请求日志</h3>
          <div className="codex-api-service-head-actions">
            <select
              className="cb-api-field"
              value={modelFilter}
              onChange={(e) => {
                setModelFilter(e.target.value);
                setPage(1);
              }}
              style={{ padding: "6px 8px", minWidth:150, maxWidth:240 }}
              >
                <option value="">全部模型</option>
                {modelOptions.map((id) => (
                <option key={id} value={id}>
                  {id}
                  </option>
                  ))}
              </select>
            <select
              className="cb-api-field"
              value={successFilter}
              onChange={(e) => {
                setSuccessFilter(e.target.value);
                setPage(1);
              }}
              style={{ padding: "6px 8px" }}
            >
              <option value="">全部</option>
              <option value="ok">成功</option>
              <option value="err">失败</option>
            </select>
            <button type="button" className="cb-api-btn sm" onClick={loadLogs}>
              <RefreshCw size={13} /> 刷新
            </button>
          </div>
        </div>
        {!logs || logs.logs.length === 0 ? (
          <div className="cb-api-log-empty">暂无请求日志。</div>
        ) : (
          <>
            <table className="cb-api-log-table">
              <thead>
                <tr>
                  <th>时间</th>
                  <th>模型</th>
                  <th>Key</th>
                  <th>状态</th>
                  <th>耗时</th>
                  <th>输入</th>
                  <th>输出</th>
                  <th>Credit</th>
                </tr>
              </thead>
              <tbody>
                {logs.logs.map((log) => (
                  <tr key={log.requestId}>
                    <td>{new Date(log.timestamp).toLocaleTimeString()}</td>
                    <td>{renderModelLabel(log.model, log.visionSubagent)}</td>
                    <td>{log.apiKeyId || "-"}</td>
                    <td>
                      <span className={`cb-api-log-status ${log.success ? "ok" : "err"}`}>
                        {log.status}
                      </span>
                    </td>
                    <td>{log.latencyMs}ms</td>
                    <td>{formatNumber(log.inputTokens)}</td>
                    <td>{formatNumber(log.outputTokens)}</td>
                    <td>{formatCredit(log.credit)}</td>
                  </tr>
                  ))}
              </tbody>
            </table>
            <div className="cb-api-pagination" style={{ padding: "10px 14px" }}>
              <span>
                第 {logs.page} / {logs.totalPages} 页（共 {logs.total} 条）
              </span>
              <button
                type="button"
                className="cb-api-btn sm"
                disabled={logs.page <= 1}
                onClick={() => setPage((p) => p - 1)}
              >
                上一页
              </button>
              <button
                type="button"
                className="cb-api-btn sm"
                disabled={logs.page >= logs.totalPages}
                onClick={() => setPage((p) => p + 1)}
              >
                下一页
              </button>
            </div>
          </>
        )}
      </section>

      {clearStatsConfirmOpen && (
        <div
          className="confirm-overlay"
          onClick={() => {
            if (!clearingStats) setClearStatsConfirmOpen(false);
          }}
        >
          <div
            className="confirm-dialog"
            onClick={(e) => e.stopPropagation()}
            role="alertdialog"
            aria-modal="true"
            aria-labelledby="cb-api-clear-stats-title"
            aria-describedby="cb-api-clear-stats-desc"
          >
            <h3 id="cb-api-clear-stats-title">
              {t("codebuddyLocalAccess.clearStatsTitle", "清空统计")}
            </h3>
            <p id="cb-api-clear-stats-desc">
              {t(
                "codebuddyLocalAccess.clearStatsConfirm",
                "确定要清空 API 服务统计吗？此操作不可撤销。",
              )}
            </p>
            <div className="confirm-actions">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => setClearStatsConfirmOpen(false)}
                disabled={clearingStats}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                type="button"
                className="btn btn-danger"
                onClick={() => void clearStats()}
                disabled={clearingStats}
              >
                {clearingStats ? (
                  <RefreshCw size={14} className="loading-spinner" />
                ) : (
                  <Trash2 size={14} />
                )}
                {t("codebuddyLocalAccess.clearStats", "清空统计")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/* ----------------------------- Chat test dialog ----------------------------- */

function ChatTestDialog(props: {
  onClose: () => void;
  baseUrl?: string;
  firstApiKey?: string;
}) {
  const { onClose, baseUrl = "", firstApiKey = "" } = props;
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [model, setModel] = useState("auto");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [modelOptions, setModelOptions] = useState<string[]>(["auto"]);

  // 测试模型下拉复用「同步模型」的数据源：从 /v1/models 拉取 sidecar 实时
  // 暴露的真实可用模型（经后端同步 / app.asar 提取 + excludedModels 过滤），
  // 而非 Rust 硬编码模型清单。这样后端不支持（如 kimi-k3）的模型不会出现。
  useEffect(() => {
    if (!baseUrl) {
      setModelOptions(["auto"]);
      return;
    }
    let disposed = false;
    const headers: Record<string, string> = firstApiKey
      ? { Authorization: `Bearer ${firstApiKey}` }
      : {};
    fetch(`${baseUrl.replace(/\/$/, "")}/v1/models`, { headers })
      .then((resp) => {
        if (!resp.ok) throw new Error(`/v1/models 返回 ${resp.status}`);
        return resp.json();
      })
      .then((data) => {
        if (disposed) return;
        const ids: string[] = Array.isArray(data?.data)
          ? data.data
              .map((m: any) => (typeof m?.id === "string" ? m.id : ""))
              .filter((s: string) => s.length > 0)
          : [];
        // auto 始终在最前（自动路由），其余按后端返回顺序。
        setModelOptions(ids.length > 0 ? ["auto", ...ids] : ["auto"]);
      })
      .catch((err) => {
        if (!disposed) {
          console.warn("拉取测试模型失败:", err);
          setModelOptions(["auto"]);
        }
      });
    return () => {
      disposed = true;
    };
  }, [baseUrl, firstApiKey]);

  const send = useCallback(async () => {
    const content = input.trim();
    if (!content || sending) return;
    const nextMessages: ChatMessage[] = [...messages, { role: "user", content }];
    setMessages(nextMessages);
    setInput("");
    setSending(true);
    setError(null);
    try {
      const result = await codebuddyLocalAccessService.chatTestCodebuddyLocalAccess(
        model,
        nextMessages.map((m) => ({ role: m.role, content: m.content })),
      );
      const choice = (result.choices as Array<{ message?: { content?: string } }>)?.[0];
      const reply = choice?.message?.content ?? "(无回复)";
      setMessages((prev) => [...prev, { role: "assistant", content: reply }]);
    } catch (err) {
      setError(String(err));
    } finally {
      setSending(false);
    }
  }, [input, messages, model, sending]);

  return (
    <div className="codex-api-service-modal-backdrop" onClick={onClose}>
      <div
        className="codex-api-service-modal chat-test-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="codex-api-service-panel-head">
          <Send />
          <h3>多轮对话测试</h3>
          <div className="codex-api-service-head-actions">
            <select
              value={model}
              onChange={(e) => setModel(e.target.value)}
              style={{ padding: "6px 8px", minWidth: 180 }}
            >
              {modelOptions.map((id) => (
                <option key={id} value={id}>
                  {id}
                  </option>
              ))}
            </select>
            <button type="button" className="btn btn-secondary btn-sm" onClick={onClose}>
              <X size={13} /> 关闭
            </button>
          </div>
        </div>
        <div className="cb-api-panel-body">
          <div className="cb-api-chat-box">
            <div className="cb-api-chat-messages">
              {messages.length === 0 && (
                <p className="cb-api-hint">发送一条消息，验证本地网关端到端联通。</p>
              )}
              {messages.map((msg, idx) => (
                <div key={idx} className={`cb-api-chat-msg ${msg.role}`}>
                  {msg.content}
                </div>
              ))}
            </div>
            {error && (
              <div className="cb-api-banner error">
                <CircleAlert />
                <span>{error}</span>
              </div>
            )}
            <div className="cb-api-chat-input-row">
              <input
                type="text"
                placeholder="输入消息…"
                value={input}
                onChange={(e) => setInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.shiftKey) {
                    e.preventDefault();
                    void send();
                  }
                }}
              />
              <button
                type="button"
                className="btn btn-primary btn-sm"
                onClick={send}
                disabled={sending || !input.trim()}
              >
                {sending ? (
                  <RefreshCw size={14} className="cb-api-spin" />
                ) : (
                  <Send size={14} />
                )}
                发送
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

/* ----------------------------- Summary card ----------------------------- */

function SummaryCard(props: {
  icon: ReactNode;
  label: string;
  value: string;
  tone?: "ok" | "err" | "credit";
}) {
  return (
    <div className={`cb-api-summary-card ${props.tone ? `tone-${props.tone}` : ""}`}>
      <span className="cb-api-summary-label">
        {props.icon}
        {props.label}
      </span>
      <span className="cb-api-summary-value">{props.value}</span>
    </div>
  );
}

/* ----------------------------- helpers ----------------------------- */

function maskKey(key: string): string {
  if (key.length <= 12) return "••••••••••••";
  return `${key.slice(0, 6)}••••••${key.slice(-4)}`;
}

function formatNumber(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

function formatCompactNumber(n: number): string {
  return formatNumber(n);
}

function formatCredit(c: number): string {
  if (c === 0) return "0";
  if (c < 0.01) return "<0.01";
  return c.toFixed(2);
}

function formatDateTime(ts: string | number): string {
  try {
    return new Date(ts).toLocaleString();
  } catch {
    return String(ts);
  }
}

/**
 * 健康状态标签（对齐 Codex 卡片渲染语义）。
 * - lastFailureCategory 含 auth/unauthorized/401/403 → "鉴权异常"
 * - cooldowns 非空 → "冷却 N"
 * - !available → "暂不可用"
 * - 否则 → "可用"
 */
function formatHealthLabel(
  available: boolean,
  cooldownCount: number,
  lastFailureCategory: string | null,
): string {
  const category = (lastFailureCategory ?? "").toLowerCase();
  if (
    category.includes("auth") ||
    category.includes("unauthorized") ||
    category.includes("401") ||
    category.includes("403")
  ) {
    return "鉴权异常";
  }
  if (cooldownCount > 0) {
    return `冷却 ${cooldownCount}`;
  }
  if (!available) {
    return "暂不可用";
  }
  return "可用";
}

function formatImageStatusLabel(status: string): string {
  switch (status) {
    case "disabled":
      return "禁用";
    case "available":
      return "可用";
    case "unavailable":
      return "不可用";
    default:
      return "unknown";
  }
}
