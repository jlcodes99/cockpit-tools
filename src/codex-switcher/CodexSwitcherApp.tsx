import { FormEvent, useCallback, useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  BadgeCheck,
  CheckCircle2,
  ExternalLink,
  KeyRound,
  Loader2,
  RefreshCw,
  RotateCw,
  ShieldCheck,
  Trash2,
  UserRound,
} from "lucide-react";
import {
  deleteAccount,
  fetchRemoteAd,
  getSettings,
  isDesktopRuntime,
  importAccessToken,
  listAccounts,
  refreshAccountQuota,
  refreshAllQuotas,
  switchAccount,
  updateSettings,
} from "./service";
import type {
  CodexSwitcherAccount,
  CodexSwitcherAdBlock,
  CodexSwitcherQuota,
  CodexSwitcherRemoteAd,
  CodexSwitcherSettings,
} from "./types";
import "./CodexSwitcherApp.css";

type BusyAction =
  | "load"
  | "import"
  | "refreshAll"
  | "settings"
  | `switch:${string}`
  | `refresh:${string}`
  | `delete:${string}`;

const defaultSettings: Required<CodexSwitcherSettings> = {
  restart_app_after_switch: false,
  delete_banned_accounts: false,
};

function getAccountName(account: CodexSwitcherAccount): string {
  return account.name || account.account_name || account.email || account.user_id || "未命名账号";
}

function isAccountCurrent(account: CodexSwitcherAccount, currentAccountId: string | null): boolean {
  return account.id === currentAccountId || Boolean(account.is_current || account.current);
}

function isAccountUnavailable(account: CodexSwitcherAccount): boolean {
  return Boolean(account.requires_reauth || account.banned || account.disabled);
}

function quotaValue(value?: number | null): number | null {
  if (typeof value !== "number" || Number.isNaN(value)) return null;
  return Math.max(0, Math.min(100, value));
}

function bestQuotaScore(account: CodexSwitcherAccount): number {
  const effectiveQuota = effectiveQuotaValues(account.quota);
  const values = [effectiveQuota.hourly, effectiveQuota.weekly].filter((value): value is number => value !== null);
  return values.length > 0 ? Math.min(...values) : -1;
}

function effectiveQuotaValues(quota?: CodexSwitcherQuota | null): {
  hourly: number | null;
  weekly: number | null;
  weeklyBlocksHourly: boolean;
} {
  if (!quota) return { hourly: null, weekly: null, weeklyBlocksHourly: false };
  const hasPresence = quota.hourly_window_present != null || quota.weekly_window_present != null;
  const hourlyPresent = !hasPresence || quota.hourly_window_present === true;
  const weeklyPresent = !hasPresence || quota.weekly_window_present === true;
  const hourly = hourlyPresent ? quotaValue(quota.hourly_percentage) : null;
  const weekly = weeklyPresent ? quotaValue(quota.weekly_percentage) : null;
  const weeklyBlocksHourly = weekly === 0 && hourly !== null;
  return {
    hourly: weeklyBlocksHourly ? 0 : hourly,
    weekly,
    weeklyBlocksHourly,
  };
}

function formatDateTime(timestamp?: number | null): string {
  if (!timestamp) return "未知";
  const millis = timestamp > 10_000_000_000 ? timestamp : timestamp * 1000;
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(millis));
}

function formatPlan(account: CodexSwitcherAccount): string {
  return account.plan_type || "未识别套餐";
}

function getErrorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  try {
    return JSON.stringify(error);
  } catch {
    return "操作失败";
  }
}

function isSafeRemoteUrl(rawUrl?: string): boolean {
  if (!rawUrl) return false;
  try {
    const url = new URL(rawUrl);
    return url.protocol === "https:";
  } catch {
    return false;
  }
}

function renderMarkdownText(markdown: string): string {
  return markdown
    .replace(/\*\*(.*?)\*\*/g, "$1")
    .replace(/__(.*?)__/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\[(.*?)\]\((.*?)\)/g, "$1")
    .trim();
}

function AdBlock({ block }: { block: CodexSwitcherAdBlock }) {
  if (block.type === "text" && block.text.trim()) {
    return <p className="cs-ad-text">{block.text}</p>;
  }

  if (block.type === "markdown" && block.markdown.trim()) {
    return <p className="cs-ad-text">{renderMarkdownText(block.markdown)}</p>;
  }

  if (block.type === "image" && isSafeRemoteUrl(block.src)) {
    const image = <img className="cs-ad-image" src={block.src} alt={block.alt || "推广图片"} />;
    return isSafeRemoteUrl(block.href) ? (
      <a className="cs-ad-image-link" href={block.href} target="_blank" rel="noreferrer">
        {image}
      </a>
    ) : (
      image
    );
  }

  if (block.type === "video" && isSafeRemoteUrl(block.src)) {
    return (
      <video
        className="cs-ad-video"
        src={block.src}
        poster={isSafeRemoteUrl(block.poster) ? block.poster : undefined}
        title={block.title || "推广视频"}
        controls
        preload="metadata"
      />
    );
  }

  if (block.type === "button" && block.label.trim() && isSafeRemoteUrl(block.href)) {
    return (
      <a className="cs-button cs-button-secondary cs-ad-button" href={block.href} target="_blank" rel="noreferrer">
        {block.label}
        <ExternalLink size={14} />
      </a>
    );
  }

  return null;
}

function RemoteAd({ ad }: { ad: CodexSwitcherRemoteAd | null }) {
  const blocks = ad?.blocks?.filter(Boolean) ?? [];
  if (!ad || blocks.length === 0) return null;

  return (
    <aside className="cs-panel cs-ad-panel" aria-label="安全广告位">
      {ad.title ? <h2>{ad.title}</h2> : null}
      <div className="cs-ad-blocks">
        {blocks.map((block, index) => (
          <AdBlock block={block} key={`${block.type}-${index}`} />
        ))}
      </div>
    </aside>
  );
}

function QuotaBar({
  label,
  value,
  resetTime,
  hint,
}: {
  label: string;
  value?: number | null;
  resetTime?: number | null;
  hint?: string | null;
}) {
  const remaining = quotaValue(value);

  return (
    <div className="cs-quota">
      <div className="cs-quota-header">
        <span>{label}</span>
        <strong>{remaining === null ? "未知" : `剩余 ${remaining.toFixed(0)}%`}</strong>
      </div>
      <div className="cs-quota-track">
        <div className="cs-quota-fill" style={{ width: `${remaining ?? 0}%` }} />
      </div>
      <span className="cs-quota-reset">{hint || `重置：${formatDateTime(resetTime)}`}</span>
    </div>
  );
}

function AccountCard({
  account,
  currentAccountId,
  restartAfterSwitch,
  busy,
  onSwitch,
  onRefresh,
  onDelete,
  desktopRuntime,
}: {
  account: CodexSwitcherAccount;
  currentAccountId: string | null;
  restartAfterSwitch: boolean;
  busy: BusyAction | null;
  onSwitch: (accountId: string) => void;
  onRefresh: (accountId: string) => void;
  onDelete: (accountId: string) => void;
  desktopRuntime: boolean;
}) {
  const current = isAccountCurrent(account, currentAccountId);
  const unavailable = isAccountUnavailable(account);
  const quota = account.quota ?? {};
  const effectiveQuota = effectiveQuotaValues(account.quota);

  return (
    <article className={`cs-account-card${current ? " cs-account-card-current" : ""}`}>
      <div className="cs-account-main">
        <div className="cs-account-avatar">
          <UserRound size={20} />
        </div>
        <div className="cs-account-title">
          <div className="cs-account-name-row">
            <h3>{getAccountName(account)}</h3>
            {current ? (
              <span className="cs-badge cs-badge-current">
                <CheckCircle2 size={13} />
                当前
              </span>
            ) : null}
            {unavailable ? (
              <span className="cs-badge cs-badge-warning">
                <AlertTriangle size={13} />
                需处理
              </span>
            ) : null}
          </div>
          <p>{account.email || account.user_id || account.id}</p>
        </div>
      </div>

      <div className="cs-account-meta">
        <span>套餐：{formatPlan(account)}</span>
        <span>最近使用：{formatDateTime(account.last_used)}</span>
        {account.organization_id ? <span>组织：{account.organization_id}</span> : null}
      </div>

      {account.quota_error ? (
        <div className="cs-inline-warning">
          <AlertTriangle size={15} />
          {account.quota_error.message}
        </div>
      ) : null}

      {account.requires_reauth && account.reauth_reason ? (
        <div className="cs-inline-warning">
          <AlertTriangle size={15} />
          {account.reauth_reason}
        </div>
      ) : null}

      <div className="cs-quota-grid">
        <QuotaBar
          label={`${quota.hourly_window_minutes || 300} 分钟额度`}
          value={effectiveQuota.hourly}
          resetTime={quota.hourly_reset_time}
          hint={effectiveQuota.weeklyBlocksHourly ? "周额度为 0，5 小时额度已不可用" : null}
        />
        <QuotaBar
          label={`${quota.weekly_window_minutes ? `${quota.weekly_window_minutes} 分钟` : "周"}额度`}
          value={effectiveQuota.weekly}
          resetTime={quota.weekly_reset_time}
        />
      </div>

      <div className="cs-card-actions">
        <button
          className="cs-button cs-button-primary"
          disabled={!desktopRuntime || current || unavailable || busy !== null}
          onClick={() => onSwitch(account.id)}
          type="button"
          title={restartAfterSwitch ? "切换后会请求重启应用" : "只切换本机 Codex 登录态"}
        >
          {busy === `switch:${account.id}` ? <Loader2 className="cs-spin" size={16} /> : <RotateCw size={16} />}
          手动切换
        </button>
        <button
          className="cs-button cs-button-secondary"
          disabled={!desktopRuntime || busy !== null}
          onClick={() => onRefresh(account.id)}
          type="button"
        >
          {busy === `refresh:${account.id}` ? <Loader2 className="cs-spin" size={16} /> : <RefreshCw size={16} />}
          刷新额度
        </button>
        <button
          className="cs-icon-button cs-danger"
          disabled={!desktopRuntime || busy !== null}
          onClick={() => onDelete(account.id)}
          type="button"
          title="删除本机保存的账号"
        >
          {busy === `delete:${account.id}` ? <Loader2 className="cs-spin" size={16} /> : <Trash2 size={17} />}
        </button>
      </div>
    </article>
  );
}

export default function CodexSwitcherApp() {
  const [accounts, setAccounts] = useState<CodexSwitcherAccount[]>([]);
  const [currentAccountId, setCurrentAccountId] = useState<string | null>(null);
  const [settings, setSettings] = useState<Required<CodexSwitcherSettings>>(defaultSettings);
  const [ad, setAd] = useState<CodexSwitcherRemoteAd | null>(null);
  const [accessToken, setAccessToken] = useState("");
  const [busy, setBusy] = useState<BusyAction | null>("load");
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const desktopRuntime = isDesktopRuntime();

  const sortedAccounts = useMemo(() => {
    return [...accounts].sort((left, right) => {
      const leftCurrent = isAccountCurrent(left, currentAccountId) ? 1 : 0;
      const rightCurrent = isAccountCurrent(right, currentAccountId) ? 1 : 0;
      if (leftCurrent !== rightCurrent) return rightCurrent - leftCurrent;
      const leftAvailable = isAccountUnavailable(left) ? 0 : 1;
      const rightAvailable = isAccountUnavailable(right) ? 0 : 1;
      if (leftAvailable !== rightAvailable) return rightAvailable - leftAvailable;
      const scoreDelta = bestQuotaScore(right) - bestQuotaScore(left);
      if (scoreDelta !== 0) return scoreDelta;
      const lastUsedDelta = (right.last_used ?? 0) - (left.last_used ?? 0);
      if (lastUsedDelta !== 0) return lastUsedDelta;
      return getAccountName(left).localeCompare(getAccountName(right), "zh-CN") || left.id.localeCompare(right.id);
    });
  }, [accounts, currentAccountId]);

  const loadData = useCallback(async () => {
    setBusy("load");
    setError(null);
    if (!isDesktopRuntime()) {
      setAccounts([]);
      setCurrentAccountId(null);
      setSettings(defaultSettings);
      setAd(null);
      setMessage("当前是网页预览模式。本机账号导入、切换和额度刷新需要在桌面应用内运行。");
      setBusy(null);
      return;
    }
    try {
      const [listResponse, loadedSettings, remoteAd] = await Promise.all([
        listAccounts(),
        getSettings().catch(() => defaultSettings),
        fetchRemoteAd().catch(() => null),
      ]);
      setAccounts(listResponse.accounts);
      setCurrentAccountId(listResponse.current_account_id ?? null);
      setSettings({ ...defaultSettings, ...loadedSettings });
      setAd(remoteAd);
    } catch (loadError) {
      setError(getErrorMessage(loadError));
    } finally {
      setBusy(null);
    }
  }, []);

  useEffect(() => {
    void loadData();
  }, [loadData]);

  async function runAction(
    action: BusyAction,
    task: () => Promise<string | void>,
    successMessage: string,
    options?: { reloadOnError?: boolean },
  ) {
    setBusy(action);
    setError(null);
    setMessage(null);
    try {
      const actionMessage = await task();
      setMessage(actionMessage || successMessage);
    } catch (actionError) {
      setError(getErrorMessage(actionError));
      if (options?.reloadOnError) {
        try {
          const listResponse = await listAccounts();
          setAccounts(listResponse.accounts);
          setCurrentAccountId(listResponse.current_account_id ?? null);
        } catch {
          // Keep the original action error visible.
        }
      }
    } finally {
      setBusy(null);
    }
  }

  function handleImport(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const token = accessToken.trim();
    if (!desktopRuntime) {
      setError("当前是网页预览模式。本机账号导入、切换和额度刷新需要在桌面应用内运行。");
      return;
    }
    if (!token) {
      setError("请先粘贴你自己合法持有的 Codex access token。");
      return;
    }

    void runAction(
      "import",
      async () => {
        await importAccessToken(token);
        setAccessToken("");
        const listResponse = await listAccounts();
        setAccounts(listResponse.accounts);
        setCurrentAccountId(listResponse.current_account_id ?? null);
      },
      "已导入本机账号。",
    );
  }

  function handleRefreshAll() {
    if (!desktopRuntime) {
      setError("当前是网页预览模式。本机账号导入、切换和额度刷新需要在桌面应用内运行。");
      return;
    }
    void runAction(
      "refreshAll",
      async () => {
        const successCount = await refreshAllQuotas();
        const listResponse = await listAccounts();
        setAccounts(listResponse.accounts);
        setCurrentAccountId(listResponse.current_account_id ?? null);
        if (typeof successCount === "number") {
          return `已刷新 ${successCount} 个账号额度。`;
        }
      },
      "已刷新全部额度。",
      { reloadOnError: true },
    );
  }

  function handleSwitch(accountId: string) {
    if (!desktopRuntime) {
      setError("当前是网页预览模式。本机账号导入、切换和额度刷新需要在桌面应用内运行。");
      return;
    }
    void runAction(
      `switch:${accountId}`,
      async () => {
        await switchAccount(accountId, settings.restart_app_after_switch);
        const listResponse = await listAccounts();
        setAccounts(listResponse.accounts);
        setCurrentAccountId(listResponse.current_account_id ?? accountId);
      },
      "账号已切换。",
    );
  }

  function handleRefresh(accountId: string) {
    if (!desktopRuntime) {
      setError("当前是网页预览模式。本机账号导入、切换和额度刷新需要在桌面应用内运行。");
      return;
    }
    void runAction(
      `refresh:${accountId}`,
      async () => {
        const quota: CodexSwitcherQuota = await refreshAccountQuota(accountId);
        setAccounts((items) =>
          items.map((account) => (account.id === accountId ? { ...account, quota, quota_error: null } : account)),
        );
      },
      "额度已刷新。",
      { reloadOnError: true },
    );
  }

  function handleDelete(accountId: string) {
    if (!desktopRuntime) {
      setError("当前是网页预览模式。本机账号导入、切换和额度刷新需要在桌面应用内运行。");
      return;
    }
    const account = accounts.find((item) => item.id === accountId);
    if (!window.confirm(`确定删除本机保存的账号“${account ? getAccountName(account) : accountId}”？`)) {
      return;
    }

    void runAction(
      `delete:${accountId}`,
      async () => {
        await deleteAccount(accountId);
        const listResponse = await listAccounts();
        setAccounts(listResponse.accounts);
        setCurrentAccountId(listResponse.current_account_id ?? null);
      },
      "账号已删除。",
    );
  }

  function handleSettingChange(patch: CodexSwitcherSettings) {
    const previousSettings = settings;
    if (!desktopRuntime) {
      setSettings(previousSettings);
      setError("当前是网页预览模式。本机账号导入、切换和额度刷新需要在桌面应用内运行。");
      return;
    }
    const nextSettings = { ...settings, ...patch };
    setSettings(nextSettings);
    void runAction(
      "settings",
      async () => {
        try {
          const saved = await updateSettings(nextSettings);
          setSettings({ ...defaultSettings, ...saved });
        } catch (settingsError) {
          setSettings(previousSettings);
          throw settingsError;
        }
      },
      "设置已保存。",
      { reloadOnError: false },
    );
  }

  return (
    <main className="cs-app">
      <section className="cs-header">
        <div>
          <span className="cs-kicker">
            <ShieldCheck size={16} />
            Codex 官方账号切换器
          </span>
          <h1>管理本机 Codex 登录态</h1>
          <p>仅用于导入、查看和切换你自己合法持有的 Codex access token 或本机登录态。</p>
        </div>
        <div className="cs-header-actions">
          <button className="cs-button cs-button-secondary" disabled={busy !== null} onClick={loadData} type="button">
            {busy === "load" ? <Loader2 className="cs-spin" size={16} /> : <RefreshCw size={16} />}
            重新加载
          </button>
        </div>
      </section>

      <RemoteAd ad={ad} />

      <section className="cs-grid">
        <div className="cs-panel cs-status-panel">
          <div className="cs-panel-title">
            <h2>状态与设置</h2>
            {busy === "settings" ? <Loader2 className="cs-spin" size={16} /> : null}
          </div>
          <div className="cs-stats">
            <div>
              <span>本机账号</span>
              <strong>{accounts.length}</strong>
            </div>
            <div>
              <span>当前账号</span>
              <strong>{currentAccountId ? getAccountName(accounts.find((item) => item.id === currentAccountId) ?? { id: currentAccountId }) : "未切换"}</strong>
            </div>
          </div>
          <label className="cs-toggle">
            <input
              checked={settings.restart_app_after_switch}
              disabled={!desktopRuntime || busy !== null}
              onChange={(event) => handleSettingChange({ restart_app_after_switch: event.target.checked })}
              type="checkbox"
            />
            <span>
              <strong>切换后请求重启应用</strong>
              <small>关闭时只更新本机 Codex 登录态。</small>
            </span>
          </label>
          <label className="cs-toggle">
            <input
              checked={settings.delete_banned_accounts}
              disabled={!desktopRuntime || busy !== null}
              onChange={(event) => handleSettingChange({ delete_banned_accounts: event.target.checked })}
              type="checkbox"
            />
            <span>
              <strong>封禁账号自动删除</strong>
              <small>仅作为设置开关，不做后台自动切号。</small>
            </span>
          </label>
        </div>

        <form className="cs-panel cs-import-panel" onSubmit={handleImport}>
          <div className="cs-panel-title">
            <h2>导入 access token</h2>
            <KeyRound size={18} />
          </div>
          <label className="cs-field">
            <span>Codex access token</span>
            <textarea
              autoComplete="off"
              disabled={!desktopRuntime || busy !== null}
              onChange={(event) => setAccessToken(event.target.value)}
              placeholder="粘贴你自己账号的 access token"
              rows={4}
              value={accessToken}
            />
          </label>
          <button className="cs-button cs-button-primary cs-full-button" disabled={!desktopRuntime || busy !== null} type="submit">
            {busy === "import" ? <Loader2 className="cs-spin" size={16} /> : <BadgeCheck size={16} />}
            导入到本机
          </button>
          <p className="cs-help-text">不会提供购买、分发第三方账号或绕过额度的功能。</p>
        </form>

        <div className="cs-panel cs-disabled-panel">
          <div className="cs-panel-title">
            <h2>企业内部分发占位</h2>
            <AlertTriangle size={18} />
          </div>
          <label className="cs-field">
            <span>激活码</span>
            <input disabled placeholder="暂未启用" type="text" />
          </label>
          <button className="cs-button cs-button-secondary cs-full-button" disabled type="button">
            暂未启用
          </button>
          <p className="cs-help-text">该区域仅作为企业内部合规分发预留，不连接兑换接口。</p>
        </div>
      </section>

      {message ? <div className="cs-toast cs-toast-success">{message}</div> : null}
      {error ? <div className="cs-toast cs-toast-error">{error}</div> : null}

      <section className="cs-panel cs-accounts-panel">
        <div className="cs-list-header">
          <div>
            <h2>账号列表</h2>
            <p>按当前账号和剩余额度排序，所有操作都需要手动触发。</p>
          </div>
          <button className="cs-button cs-button-secondary" disabled={!desktopRuntime || busy !== null || accounts.length === 0} onClick={handleRefreshAll} type="button">
            {busy === "refreshAll" ? <Loader2 className="cs-spin" size={16} /> : <RefreshCw size={16} />}
            刷新全部额度
          </button>
        </div>

        {busy === "load" ? (
          <div className="cs-empty">
            <Loader2 className="cs-spin" size={24} />
            正在读取本机账号...
          </div>
        ) : sortedAccounts.length === 0 ? (
          <div className="cs-empty">
            <UserRound size={24} />
            暂无账号。请导入你自己合法持有的 Codex access token。
          </div>
        ) : (
          <div className="cs-account-list">
            {sortedAccounts.map((account) => (
              <AccountCard
                account={account}
                busy={busy}
                currentAccountId={currentAccountId}
                desktopRuntime={desktopRuntime}
                key={account.id}
                onDelete={handleDelete}
                onRefresh={handleRefresh}
                onSwitch={handleSwitch}
                restartAfterSwitch={settings.restart_app_after_switch}
              />
            ))}
          </div>
        )}
      </section>
    </main>
  );
}
