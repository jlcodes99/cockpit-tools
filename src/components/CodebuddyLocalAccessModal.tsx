import { useCallback, useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  Check,
  CircleAlert,
  Copy,
  Eye,
  EyeOff,
  KeyRound,
  Power,
  RefreshCw,
  Route,
  Search,
  Shield,
  Trash2,
  Users,
  X,
  Zap,
} from "lucide-react";
import { SingleSelectDropdown } from "./SingleSelectDropdown";
import * as codebuddyLocalAccessService from "../services/codebuddyLocalAccessService";
import type {
  CodebuddyLocalAccessAccountOption,
  CodebuddyLocalAccessRoutingStrategy,
  CodebuddyLocalAccessState,
} from "../types/codebuddyLocalAccess";
import { useEscClose } from "../hooks/useEscClose";
import "./CodebuddyLocalAccessModal.css";

export type CodebuddyLocalAccessModalMode = "panel" | "members" | "remove";

export interface CodebuddyLocalAccessModalProps {
  /** 弹窗是否打开 */
  isOpen: boolean;
  /** 弹窗模式：panel=服务面板；members=成员选择；remove=移除 API 服务 */
  mode: CodebuddyLocalAccessModalMode;
  /** 当前反代服务状态；由父组件通过 getCodebuddyLocalAccessState() 拉取 */
  state: CodebuddyLocalAccessState | null;
  /** 关闭弹窗 */
  onClose: () => void;
  /** 跳转完整 API 服务页（可选） */
  onOpenFullPage?: () => void;
  /** 启用/停用 API 服务（在 panel 模式中切换开关） */
  onToggleEnabled?: () => Promise<void> | void;
  /** 保存账号选择（members 模式） */
  onSaveAccounts?: (intlAccountIds: string[], cnAccountIds: string[]) => Promise<void> | void;
  /** 移除 API 服务（remove 模式） */
  onRemoveApiService?: () => Promise<void> | void;
  /** 杀掉端口占用（panel 模式可选） */
  onKillPort?: () => Promise<void> | void;
  /** 轮换客户端 Key（panel 模式可选） */
  onRotateApiKey?: () => Promise<void> | void;
  /** 刷新统计（panel 模式可选） */
  onRefreshStats?: () => Promise<void> | void;
  /** 更新调度策略（panel 模式可选） */
  onUpdateRoutingStrategy?: (
    strategy: CodebuddyLocalAccessRoutingStrategy,
  ) => Promise<void> | void;
  /** 更新会话亲和开关（panel 模式可选） */
  onUpdateSessionAffinity?: (enabled: boolean) => Promise<void> | void;
  /** 是否在异步操作中（按钮禁用态） */
  saving?: boolean;
  /** 风险提示已确认（首次启用时） */
  riskNoticeAccepted?: boolean;
  /** 风险提示确认回调 */
  onAcceptRiskNotice?: () => void;
}

interface DraftSelection {
  intlAccountIds: string[];
  cnAccountIds: string[];
}

const EMPTY_DRAFT: DraftSelection = { intlAccountIds: [], cnAccountIds: [] };

function maskKey(key: string): string {
  if (!key) return "";
  if (key.length <= 12) return "••••••••••••";
  return `${key.slice(0, 6)}••••••${key.slice(-4)}`;
}

function formatNumber(n: number): string {
  if (!Number.isFinite(n)) return "0";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

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

export function CodebuddyLocalAccessModal(props: CodebuddyLocalAccessModalProps) {
  const {
    isOpen,
    mode,
    state,
    onClose,
    onOpenFullPage,
    onToggleEnabled,
    onSaveAccounts,
    onRemoveApiService,
    onKillPort,
    onRotateApiKey,
    onRefreshStats,
    onUpdateRoutingStrategy,
    onUpdateSessionAffinity,
    saving = false,
    riskNoticeAccepted = false,
    onAcceptRiskNotice,
  } = props;

  const [draft, setDraft] = useState<DraftSelection>(EMPTY_DRAFT);
  const [search, setSearch] = useState("");
  const [revealedKey, setRevealedKey] = useState(false);
  const [copied, setCopied] = useState(false);
  const [stats, setStats] = useState<{
    totalRequests: number;
    totalTokens: number;
    totalCredit: number;
    promptCacheHitTokens: number;
    promptCacheWriteTokens: number;
  } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);

  useEscClose(isOpen, onClose);

  // 当 state 变化时同步草稿
  useEffect(() => {
    if (!state) return;
    setDraft({
      intlAccountIds: [...(state.collection.intlAccountIds ?? [])],
      cnAccountIds: [...(state.collection.cnAccountIds ?? [])],
    });
  }, [state]);

  // 拉取统计
  useEffect(() => {
    if (!isOpen || mode !== "panel") return;
    void (async () => {
      try {
        const s = await codebuddyLocalAccessService.getCodebuddyLocalAccessStats();
        setStats({
          totalRequests: s.totals?.requestCount ?? 0,
          totalTokens: s.totals?.totalTokens ?? 0,
          totalCredit: s.totals?.totalCredit ?? 0,
          promptCacheHitTokens: s.totals?.promptCacheHitTokens ?? 0,
          promptCacheWriteTokens: s.totals?.promptCacheWriteTokens ?? 0,
        });
      } catch {
        // ignore
      }
    })();
  }, [isOpen, mode]);

  const refreshStats = useCallback(async () => {
    if (!onRefreshStats) {
      try {
        const s = await codebuddyLocalAccessService.getCodebuddyLocalAccessStats();
        setStats({
          totalRequests: s.totals?.requestCount ?? 0,
          totalTokens: s.totals?.totalTokens ?? 0,
          totalCredit: s.totals?.totalCredit ?? 0,
          promptCacheHitTokens: s.totals?.promptCacheHitTokens ?? 0,
          promptCacheWriteTokens: s.totals?.promptCacheWriteTokens ?? 0,
        });
      } catch {
        // ignore
      }
      return;
    }
    await onRefreshStats();
  }, [onRefreshStats]);

  const toggleAccount = useCallback(
    (region: "intl" | "cn", id: string) => {
      setDraft((prev) => {
        const key = region === "intl" ? "intlAccountIds" : "cnAccountIds";
        const list = prev[key];
        const next = list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
        return { ...prev, [key]: next };
      });
    },
    [],
  );

  const saveAccounts = useCallback(async () => {
    if (!onSaveAccounts) return;
    setBusy(true);
    setError(null);
    try {
      await onSaveAccounts(draft.intlAccountIds, draft.cnAccountIds);
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }, [draft, onSaveAccounts, onClose]);

  const removeApiService = useCallback(async () => {
    if (!onRemoveApiService) return;
    setBusy(true);
    setError(null);
    try {
      await onRemoveApiService();
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }, [onRemoveApiService, onClose]);

  const copyBaseUrl = useCallback(async () => {
    if (!state?.baseUrl) return;
    try {
      await navigator.clipboard.writeText(state.baseUrl);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // ignore
    }
  }, [state?.baseUrl]);

  const filteredIntl = useMemo(() => {
    if (!search) return state?.intlAccounts ?? [];
    const q = search.toLowerCase();
    return (state?.intlAccounts ?? []).filter((a) => a.email.toLowerCase().includes(q));
  }, [state?.intlAccounts, search]);

  const filteredCn = useMemo(() => {
    if (!search) return state?.cnAccounts ?? [];
    const q = search.toLowerCase();
    return (state?.cnAccounts ?? []).filter((a) => a.email.toLowerCase().includes(q));
  }, [state?.cnAccounts, search]);

  const totalSelected = draft.intlAccountIds.length + draft.cnAccountIds.length;

  if (!isOpen) return null;

  const title =
    mode === "panel"
      ? "CodeBuddy API 服务面板"
      : mode === "members"
        ? "选择 API 凭据账号"
        : "移除 CodeBuddy API 服务";

  return (
    <div
      className="cb-api-modal-backdrop"
      role="dialog"
      aria-modal="true"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="cb-api-modal-dialog">
        <header className="cb-api-modal-head">
          <div className="cb-api-modal-title">
            <Route />
            <h3>{title}</h3>
          </div>
          <button type="button" className="cb-api-modal-close" onClick={onClose} title="关闭">
            <X size={16} />
          </button>
        </header>

        <div className="cb-api-modal-body">
          {error && (
            <div className="cb-api-banner error">
              <CircleAlert />
              <span>{error}</span>
            </div>
          )}

          {mode === "panel" && (
            <PanelMode
              state={state}
              stats={stats}
              copied={copied}
              revealedKey={revealedKey}
              saving={saving}
              busy={busy}
              riskNoticeAccepted={riskNoticeAccepted}
              onCopyBaseUrl={copyBaseUrl}
              onToggleRevealKey={() => setRevealedKey((v) => !v)}
              onToggleEnabled={onToggleEnabled}
              onOpenFullPage={onOpenFullPage}
              onKillPort={onKillPort}
              onRotateApiKey={onRotateApiKey}
              onRefreshStats={refreshStats}
              onAcceptRiskNotice={onAcceptRiskNotice}
              onUpdateRoutingStrategy={onUpdateRoutingStrategy}
              onUpdateSessionAffinity={onUpdateSessionAffinity}
            />
          )}

          {mode === "members" && (
            <MembersMode
              state={state}
              draft={draft}
              search={search}
              filteredIntl={filteredIntl}
              filteredCn={filteredCn}
              totalSelected={totalSelected}
              busy={busy}
              onSearch={setSearch}
              onToggleAccount={toggleAccount}
              onSave={saveAccounts}
            />
          )}

          {mode === "remove" && (
            <RemoveMode
              state={state}
              confirmRemove={confirmRemove}
              busy={busy}
              onToggleConfirm={() => setConfirmRemove((v) => !v)}
              onRemove={removeApiService}
            />
          )}
        </div>
      </div>
    </div>
  );
}

/* ----------------------------- Panel mode ----------------------------- */

function PanelMode(props: {
  state: CodebuddyLocalAccessState | null;
  stats: {
    totalRequests: number;
    totalTokens: number;
    totalCredit: number;
    promptCacheHitTokens: number;
    promptCacheWriteTokens: number;
  } | null;
  copied: boolean;
  revealedKey: boolean;
  saving: boolean;
  busy: boolean;
  riskNoticeAccepted: boolean;
  onCopyBaseUrl: () => void;
  onToggleRevealKey: () => void;
  onToggleEnabled?: () => Promise<void> | void;
  onOpenFullPage?: () => void;
  onKillPort?: () => Promise<void> | void;
  onRotateApiKey?: () => Promise<void> | void;
  onRefreshStats: () => Promise<void>;
  onAcceptRiskNotice?: () => void;
  onUpdateRoutingStrategy?: (
    strategy: CodebuddyLocalAccessRoutingStrategy,
  ) => Promise<void> | void;
  onUpdateSessionAffinity?: (enabled: boolean) => Promise<void> | void;
}) {
  const {
    state,
    stats,
    copied,
    revealedKey,
    saving,
    busy,
    riskNoticeAccepted,
    onCopyBaseUrl,
    onToggleRevealKey,
    onToggleEnabled,
    onOpenFullPage,
    onKillPort,
    onRotateApiKey,
    onRefreshStats,
    onAcceptRiskNotice,
    onUpdateRoutingStrategy,
    onUpdateSessionAffinity,
  } = props;

  const firstApiKey = state?.collection.apiKeys?.[0];
  const running = state?.running ?? false;

  const routingStrategy = state?.collection.routingStrategy ?? "auto";
  const sessionAffinity = state?.collection.sessionAffinity ?? true;

  const handleChangeRoutingStrategy = async (nextStrategy: string) => {
    if (!onUpdateRoutingStrategy) return;
    await onUpdateRoutingStrategy(nextStrategy as CodebuddyLocalAccessRoutingStrategy);
  };

  const handleToggleSessionAffinity = async () => {
    if (!onUpdateSessionAffinity) return;
    await onUpdateSessionAffinity(!sessionAffinity);
  };

  return (
    <div className="cb-api-modal-panel">
      {!riskNoticeAccepted && state?.collection.enabled && (
        <div className="cb-api-risk-notice">
          <Shield />
          <div>
            <p className="cb-api-risk-title">启用 API 反代服务须知</p>
            <p className="cb-api-risk-text">
              启用后，已登录的 CodeBuddy 账号将通过本地网关以 OpenAI 兼容协议对外提供推理服务。请确保仅在可信环境使用。
            </p>
            {onAcceptRiskNotice && (
              <button
                type="button"
                className="cb-api-btn primary sm"
                onClick={onAcceptRiskNotice}
              >
                <Check size={13} /> 我已了解
              </button>
            )}
          </div>
        </div>
      )}

      <section className="cb-api-modal-section">
        <div className="cb-api-modal-section-head">
          <Zap size={14} />
          <h4>服务状态</h4>
          <span className={`cb-api-status-pill ${running ? "running" : "stopped"}`}>
            {running ? "运行中" : "已停用"}
          </span>
        </div>
        <div className="cb-api-modal-row">
          <code className="cb-api-modal-url">{state?.baseUrl ?? ""}/v1</code>
          <button type="button" className="cb-api-btn sm" onClick={onCopyBaseUrl}>
            {copied ? <Check size={13} /> : <Copy size={13} />}
            {copied ? "已复制" : "复制"}
          </button>
          <button type="button" className="cb-api-btn sm" onClick={onRefreshStats} disabled={busy}>
            <RefreshCw size={13} className={busy ? "cb-api-spin" : ""} /> 刷新
          </button>
        </div>
        {state?.collection.scope === "lan" && state?.lanBaseUrl && (
          <div className="cb-api-modal-row cb-api-modal-lan-row">
            <span className="cb-api-lan-label">局域网 URL：</span>
            <code className="cb-api-modal-url">{state.lanBaseUrl}/v1</code>
          </div>
        )}
      </section>

      <section className="cb-api-modal-section">
        <div className="cb-api-modal-section-head">
          <KeyRound size={14} />
          <h4>客户端 Key</h4>
          {onRotateApiKey && (
            <button type="button" className="cb-api-btn sm" onClick={onRotateApiKey} disabled={busy}>
              <RefreshCw size={13} /> 轮换
            </button>
          )}
        </div>
        {firstApiKey ? (
          <div className="cb-api-modal-key-row">
            <span className="cb-api-modal-key-name">{firstApiKey.name}</span>
            <code className="cb-api-modal-key-value">
              {revealedKey ? firstApiKey.key : maskKey(firstApiKey.key)}
            </code>
            <button type="button" className="cb-api-btn sm" onClick={onToggleRevealKey} title="显隐">
              {revealedKey ? <EyeOff size={13} /> : <Eye size={13} />}
            </button>
          </div>
        ) : (
          <p className="cb-api-hint">暂无客户端 Key，请在 API 服务页中创建。</p>
        )}
      </section>

      <section className="cb-api-modal-section">
        <div className="cb-api-modal-section-head">
          <Users size={14} />
          <h4>账号池</h4>
        </div>
        <div className="cb-api-modal-stat-grid">
          <div className="cb-api-modal-stat">
            <span>国际站</span>
            <strong>{state?.intlAccounts?.length ?? 0}</strong>
          </div>
          <div className="cb-api-modal-stat">
            <span>中国站</span>
            <strong>{state?.cnAccounts?.length ?? 0}</strong>
          </div>
          <div className="cb-api-modal-stat">
            <span>已选凭据</span>
            <strong>
              {(state?.collection.intlAccountIds?.length ?? 0) +
                (state?.collection.cnAccountIds?.length ?? 0)}
            </strong>
          </div>
        </div>
      </section>

      <section className="cb-api-modal-section">
        <div className="cb-api-modal-section-head">
          <Route size={14} />
          <h4>调度策略</h4>
        </div>
        <div className="cb-api-modal-row">
          <span className="cb-api-modal-key-name">路由策略</span>
          <SingleSelectDropdown
            value={routingStrategy}
            options={ROUTING_STRATEGY_OPTIONS}
            onChange={(value) => void handleChangeRoutingStrategy(value)}
            disabled={busy || !onUpdateRoutingStrategy}
            className="cb-api-routing-select"
            ariaLabel="调度策略"
          />
        </div>
        <label className="cb-api-checkbox-row">
          <input
            type="checkbox"
            checked={sessionAffinity}
            onChange={() => void handleToggleSessionAffinity()}
            disabled={busy || !onUpdateSessionAffinity}
          />
          <span>会话亲和（同一会话稳定路由到同一账号，最大化命中缓存）</span>
        </label>
      </section>

      {stats && (
        <section className="cb-api-modal-section">
          <div className="cb-api-modal-section-head">
            <RefreshCw size={14} />
            <h4>统计摘要</h4>
          </div>
          <div className="cb-api-modal-stat-grid">
            <div className="cb-api-modal-stat">
              <span>总请求</span>
              <strong>{formatNumber(stats.totalRequests)}</strong>
            </div>
            <div className="cb-api-modal-stat">
              <span>总 Token</span>
              <strong>{formatNumber(stats.totalTokens)}</strong>
            </div>
            <div className="cb-api-modal-stat tone-credit">
              <span>Credit</span>
              <strong>{stats.totalCredit.toFixed(2)}</strong>
            </div>
            <div className="cb-api-modal-stat tone-cache">
              <span>缓存命中</span>
              <strong>{formatNumber(stats.promptCacheHitTokens)}</strong>
            </div>
            <div className="cb-api-modal-stat">
              <span>缓存写入</span>
              <strong>{formatNumber(stats.promptCacheWriteTokens)}</strong>
            </div>
          </div>
        </section>
      )}

      <footer className="cb-api-modal-footer">
        {onOpenFullPage && (
          <button type="button" className="cb-api-btn" onClick={onOpenFullPage}>
            <Route size={13} /> 打开完整页
          </button>
        )}
        {onKillPort && (
          <button type="button" className="cb-api-btn" onClick={onKillPort} disabled={busy}>
            <AlertTriangle size={13} /> 杀掉端口
          </button>
        )}
        {onToggleEnabled && (
          <button
            type="button"
            className={`cb-api-btn ${running ? "danger" : "primary"}`}
            onClick={onToggleEnabled}
            disabled={saving}
          >
            {running ? <Power size={13} /> : <Zap size={13} />}
            {running ? "停止服务" : "启动服务"}
          </button>
        )}
      </footer>
    </div>
  );
}

/* ----------------------------- Members mode ----------------------------- */

function MembersMode(props: {
  state: CodebuddyLocalAccessState | null;
  draft: DraftSelection;
  search: string;
  filteredIntl: CodebuddyLocalAccessAccountOption[];
  filteredCn: CodebuddyLocalAccessAccountOption[];
  totalSelected: number;
  busy: boolean;
  onSearch: (value: string) => void;
  onToggleAccount: (region: "intl" | "cn", id: string) => void;
  onSave: () => Promise<void>;
}) {
  const {
    state,
    draft,
    search,
    filteredIntl,
    filteredCn,
    totalSelected,
    busy,
    onSearch,
    onToggleAccount,
    onSave,
  } = props;

  return (
    <div className="cb-api-modal-panel">
      <section className="cb-api-modal-section">
        <div className="cb-api-modal-section-head">
          <Search size={14} />
          <h4>选择作为 API 凭据的账号</h4>
          <span className="cb-api-section-hint">已选 {totalSelected} 个</span>
        </div>
        <div className="cb-api-modal-search-row">
          <Search size={13} />
          <input
            type="text"
            placeholder="搜索邮箱…"
            value={search}
            onChange={(e) => onSearch(e.target.value)}
          />
        </div>
      </section>

      <div className="cb-api-modal-account-columns">
        <AccountColumn
          title="国际站（codebuddy.ai）"
          accounts={filteredIntl}
          selectedIds={draft.intlAccountIds}
          onToggle={(id) => onToggleAccount("intl", id)}
        />
        <AccountColumn
          title="中国站（codebuddy.cn / workbuddy.cn）"
          accounts={filteredCn}
          selectedIds={draft.cnAccountIds}
          onToggle={(id) => onToggleAccount("cn", id)}
        />
      </div>

      {!state ||
      (state.intlAccounts.length === 0 && state.cnAccounts.length === 0) ? (
        <p className="cb-api-hint">暂无可用账号，请先在账号页面登录。</p>
      ) : null}

      <footer className="cb-api-modal-footer">
        <button type="button" className="cb-api-btn primary" onClick={onSave} disabled={busy}>
          <Check size={13} /> 保存选择
        </button>
      </footer>
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
    <div className="cb-api-modal-account-column">
      <h5>{title}</h5>
      {accounts.length === 0 ? (
        <p className="cb-api-hint">暂无账号</p>
      ) : (
        <ul className="cb-api-modal-account-list">
          {accounts.map((account) => {
            const checked = selectedIds.includes(account.id);
            return (
              <li key={account.id}>
                <label className={`cb-api-modal-account-item ${checked ? "checked" : ""}`}>
                  <input
                    type="checkbox"
                    checked={checked}
                    onChange={() => onToggle(account.id)}
                  />
                  <span className="cb-api-modal-account-email">{account.email}</span>
                  {account.planType && (
                    <span className="cb-api-modal-account-plan">{account.planType}</span>
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

/* ----------------------------- Remove mode ----------------------------- */

function RemoveMode(props: {
  state: CodebuddyLocalAccessState | null;
  confirmRemove: boolean;
  busy: boolean;
  onToggleConfirm: () => void;
  onRemove: () => Promise<void>;
}) {
  const { state, confirmRemove, busy, onToggleConfirm, onRemove } = props;

  return (
    <div className="cb-api-modal-panel">
      <section className="cb-api-modal-section cb-api-modal-remove-section">
        <AlertTriangle size={28} className="cb-api-remove-icon" />
        <h4>确认移除 CodeBuddy API 服务？</h4>
        <p className="cb-api-hint">
          移除后，已配置的客户端 Key、账号选择和端口绑定都会被清除。
          {state?.collection.apiKeys && state.collection.apiKeys.length > 0 && (
            <>
              <br />
              当前共 <strong>{state.collection.apiKeys.length}</strong> 个客户端 Key 将失效。
            </>
          )}
        </p>
        <label className="cb-api-checkbox-row cb-api-remove-confirm">
          <input
            type="checkbox"
            checked={confirmRemove}
            onChange={onToggleConfirm}
          />
          <span>我已了解此操作不可撤销</span>
        </label>
      </section>

      <footer className="cb-api-modal-footer">
        <button type="button" className="cb-api-btn danger" onClick={onRemove} disabled={!confirmRemove || busy}>
          <Trash2 size={13} /> 确认移除
        </button>
      </footer>
    </div>
  );
}
