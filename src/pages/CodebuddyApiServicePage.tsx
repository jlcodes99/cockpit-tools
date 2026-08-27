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
import * as codebuddyLocalAccessService from "../services/codebuddyLocalAccessService";
import type {
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
            statsRange={statsRange}
            statsTimeRange={statsTimeRange}
            onStatsPresetChange={(
              key: Exclude<CodexStatsRangeKey, "custom">,
              range: CodexStatsTimeRange,
            ) => {
              setStatsRange(key);
              setStatsTimeRange(range);
            }}
            onStatsCustomApply={(range: CodexStatsTimeRange) => {
              setStatsRange("custom");
              setStatsTimeRange(range);
            }}
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
            saving={saving}
          />
        )}
        {activeTab === "models" && (
          <ModelsTab
            collection={collection}
            onUpdate={update}
            onRefresh={load}
            baseUrl={displayBaseUrl}
            firstApiKey={firstApiKey}
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

      {testOpen && <ChatTestDialog onClose={() => setTestOpen(false)} />}

      <footer className="cb-api-footer">
        <button
          type="button"
          className="btn btn-primary"
          onClick={save}
          disabled={saving}
        >
          {saving ? (
            <RefreshCw size={14} className="loading-spinner" />
          ) : (
            <Check size={14} />
          )}
          保存并应用
        </button>
      </footer>
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
  statsRange: CodexStatsRangeKey;
  statsTimeRange: CodexStatsTimeRange;
  onStatsPresetChange: (
    key: Exclude<CodexStatsRangeKey, "custom">,
    range: CodexStatsTimeRange,
  ) => void;
  onStatsCustomApply: (range: CodexStatsTimeRange) => void;
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
    statsRange,
    statsTimeRange,
    onStatsPresetChange,
    onStatsCustomApply,
  } = props;

  const [stats, setStats] = useState<CodebuddyLocalAccessStats | null>(null);
  const [statsLoading, setStatsLoading] = useState(false);

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

  const totals = stats?.totals;
  const successRate =
    totals && totals.requestCount > 0
      ? Math.round((totals.successCount / totals.requestCount) * 1000) / 10
      : 0;
  const avgLatency =
    totals && totals.requestCount > 0
      ? Math.round(totals.totalLatencyMs / totals.requestCount)
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

  const summaryCards = [
    {
      key: "requests",
      label: "总请求数",
      value: formatCompactNumber(totals?.requestCount ?? 0),
      detail: `成功 ${formatCompactNumber(totals?.successCount ?? 0)} / 失败 ${formatCompactNumber(totals?.failureCount ?? 0)}`,
    },
    {
      key: "images",
      label: "图片请求",
      value: formatCompactNumber(totals?.imageRequestCount ?? 0),
      detail: `生成 ${formatCompactNumber(totals?.imageGenerationRequestCount ?? 0)} / 编辑 ${formatCompactNumber(totals?.imageEditRequestCount ?? 0)} / 权限 ${formatCompactNumber(totals?.imageGenerationCapabilityFailureCount ?? 0)}`,
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
      value: avgLatency > 0 ? `${avgLatency}ms` : "-",
      detail: `成功率 ${successRate}%`,
    },
  ];

  const memberAccounts = useMemo(() => {
    const intl = state?.intlAccounts ?? [];
    const cn = state?.cnAccounts ?? [];
    return [...intl, ...cn];
  }, [state?.intlAccounts, state?.cnAccounts]);

  const accountEmailById = useMemo(() => {
    const map = new Map<string, string>();
    for (const account of memberAccounts) {
      map.set(account.id, account.email || account.id);
    }
    return map;
  }, [memberAccounts]);

  const availableAccountCount =
    (state?.intlAccounts?.length ?? 0) + (state?.cnAccounts?.length ?? 0);

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
    { value: "lan", label: "lan" },
  ];

  const selectedStatsRangeTitle =
    statsRange === "daily"
      ? "今日"
      : statsRange === "weekly"
        ? "本周"
        : statsRange === "monthly"
          ? "本月"
          : `${statsTimeRange.startInput} - ${statsTimeRange.endInput}`;

  return (
    <div className="codex-api-service-tab-panel">
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
          onPresetChange={onStatsPresetChange}
          onCustomApply={onStatsCustomApply}
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

      {stats && stats.byAccount.length > 0 && (
        <section className="codex-api-service-panel cb-account-stats-panel">
          <div className="codex-api-service-panel-head">
            <h2>按账号统计</h2>
            <span className="cb-account-stats-hint">
              会话亲和可让同一会话稳定命中缓存，降低 credit 消耗
            </span>
          </div>
          <div className="cb-account-stats-table">
            <div className="cb-account-stats-row cb-account-stats-head-row">
              <span>账号</span>
              <span>请求</span>
              <span>缓存命中率</span>
              <span>命中 Token</span>
              <span>Credit</span>
            </div>
            {stats.byAccount.map((account) => {
              const usage = account.usage;
              const cacheTotal =
                usage.promptCacheHitTokens + usage.promptCacheMissTokens;
              const rate =
                cacheTotal > 0
                  ? Math.round((usage.promptCacheHitTokens / cacheTotal) * 1000) /
                    10
                  : 0;
              const displayName =
                accountEmailById.get(account.accountId) ?? account.accountId;
              return (
                <div key={account.accountId} className="cb-account-stats-row">
                  <span className="cb-account-stats-id" title={account.accountId}>
                    {displayName}
                  </span>
                  <span>{formatCompactNumber(usage.requestCount)}</span>
                  <span className={rate > 0 ? "cb-account-cache-hit" : ""}>
                    {rate}%
                  </span>
                  <span>{formatCompactNumber(usage.promptCacheHitTokens)}</span>
                  <span>{formatCredit(usage.totalCredit)}</span>
                </div>
              );
            })}
          </div>
        </section>
      )}

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
  saving: boolean;
}) {
  const { state, collection, setState, onUpdate, saving } = props;
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const toggleAccount = useCallback(
    (region: "intl" | "cn", accountId: string) => {
      setState((prev) => {
        if (!prev) return prev;
        const key = region === "intl" ? "intlAccountIds" : "cnAccountIds";
        const current = prev.collection[key] ?? [];
        const next = current.includes(accountId)
          ? current.filter((id) => id !== accountId)
          : [...current, accountId];
        return { ...prev, collection: { ...prev.collection, [key]: next } };
      });
    },
    [setState],
  );

  const refresh = useCallback(async () => {
    setRefreshing(true);
    setError(null);
    try {
      const next = await codebuddyLocalAccessService.getCodebuddyLocalAccessState();
      setState(next);
    } catch (err) {
      setError(String(err));
    } finally {
      setRefreshing(false);
    }
  }, [setState]);

  const selectedCount =
    (collection.intlAccountIds?.length ?? 0) + (collection.cnAccountIds?.length ?? 0);

  const selectedAccountOptions = useMemo(() => {
    const intl = state?.intlAccounts ?? [];
    const cn = state?.cnAccounts ?? [];
    const all = [...intl, ...cn];
    const selected = new Set([
      ...(collection.intlAccountIds ?? []),
      ...(collection.cnAccountIds ?? []),
    ]);
    return all.filter((a) => selected.has(a.id));
  }, [state?.intlAccounts, state?.cnAccounts, collection.intlAccountIds, collection.cnAccountIds]);

  return (
    <div className="codex-api-service-tab-panel">
      <header className="cb-api-section-header">
        <h3>账号池</h3>
        <button
          type="button"
          className="cb-api-refresh-btn"
          onClick={refresh}
          disabled={refreshing}
          title="刷新账号列表"
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
          <Route />
          <h3>调度策略</h3>
        </div>
        <div className="cb-api-panel-body cb-routing-panel">
          <div className="cb-routing-row">
            <span className="cb-routing-label">路由策略</span>
            <SingleSelectDropdown
              value={collection.routingStrategy}
              options={ROUTING_STRATEGY_OPTIONS}
              onChange={(value) =>
                onUpdate({
                  routingStrategy: value as CodebuddyLocalAccessRoutingStrategy,
                })
              }
              disabled={saving}
              className="cb-routing-select"
              ariaLabel="账号池调度策略"
            />
          </div>
          <label className="cb-routing-checkbox">
            <input
              type="checkbox"
              checked={collection.sessionAffinity}
              onChange={(e) =>
                onUpdate({ sessionAffinity: e.target.checked })
              }
              disabled={saving}
            />
            <span>会话亲和（同一会话稳定路由到同一账号，最大化命中缓存）</span>
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
      </section>
      <section className="codex-api-service-panel">
        <div className="codex-api-service-panel-head">
          <Users />
          <h3>账号选择（已选 {selectedCount} 个）</h3>
        </div>
        <div className="cb-api-panel-body">
          <div className="codex-api-service-account-columns">
            <AccountColumn
              title="国际站（codebuddy.ai）"
              accounts={state?.intlAccounts ?? []}
              selectedIds={collection.intlAccountIds ?? []}
              onToggle={(id) => toggleAccount("intl", id)}
            />
            <AccountColumn
              title="中国站（codebuddy.cn / workbuddy.cn）"
              accounts={state?.cnAccounts ?? []}
              selectedIds={collection.cnAccountIds ?? []}
              onToggle={(id) => toggleAccount("cn", id)}
            />
          </div>
          {selectedCount === 0 && (
            <p className="cb-api-hint">请至少选择一个已登录的 CodeBuddy 账号作为凭据来源。</p>
          )}
        </div>
      </section>
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

function AccountColumn(props: {
  title: string;
  accounts: CodebuddyLocalAccessAccountOption[];
  selectedIds: string[];
  onToggle: (id: string) => void;
}) {
  const { title, accounts, selectedIds, onToggle } = props;
  return (
    <div className="codex-api-service-account-column">
      <h4>{title}</h4>
      {accounts.length === 0 ? (
        <p className="cb-api-hint">暂无账号，请先在账号页面登录。</p>
      ) : (
        <ul className="cb-api-account-list">
          {accounts.map((account) => {
            const checked = selectedIds.includes(account.id);
            return (
              <li key={account.id}>
                <label className="cb-api-account-item">
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={() => onToggle(account.id)}
                  />
                  <span className="cb-api-account-email">{account.email}</span>
                  {account.planType && (
                    <span className="cb-api-account-plan">{account.planType}</span>
                  )}
                </label>
              </li>
            );
          })}
        </ul>
      )}
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
}) {
  const { collection, onUpdate, onRefresh, baseUrl = "", firstApiKey = "" } = props;
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

function LogsTab() {
  const [stats, setStats] = useState<CodebuddyLocalAccessStats | null>(null);
  const [logs, setLogs] = useState<CodebuddyLocalAccessLogPage | null>(null);
  const [page, setPage] = useState(1);
  const [modelFilter, setModelFilter] = useState("");
  const [successFilter, setSuccessFilter] = useState<string>("");
  const [statsLoading, setStatsLoading] = useState(false);
  const [logsLoading, setLogsLoading] = useState(false);

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
    try {
      setStats(await codebuddyLocalAccessService.clearCodebuddyLocalAccessStats());
      void loadLogs();
    } catch {
      // ignore
    }
  }, [loadLogs]);

  const totals = stats?.totals;

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
            <button type="button" className="cb-api-btn danger" onClick={clearStats}>
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
            <input
              className="cb-api-base-url"
              type="text"
              placeholder="按模型筛选"
              value={modelFilter}
              onChange={(e) => {
                setModelFilter(e.target.value);
                setPage(1);
              }}
              style={{ width: 140 }}
            />
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
                    <td>{log.model || "-"}</td>
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
    </div>
  );
}

/* ----------------------------- Chat test dialog ----------------------------- */

function ChatTestDialog(props: { onClose: () => void }) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [model, setModel] = useState("auto");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
    <div className="codex-api-service-modal-backdrop" onClick={props.onClose}>
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
              style={{ padding: "6px 8px" }}
            >
              <option value="auto">auto（自动）</option>
              <option value="deepseek-v4-flash">deepseek-v4-flash</option>
              <option value="deepseek-v4-pro">deepseek-v4-pro</option>
              <option value="glm-5.2">glm-5.2</option>
              <option value="kimi-k2.7">kimi-k2.7</option>
            </select>
            <button type="button" className="btn btn-secondary btn-sm" onClick={props.onClose}>
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
