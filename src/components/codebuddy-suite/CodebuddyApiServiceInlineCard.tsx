import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Check,
  Copy,
  Database,
  ExternalLink,
  Eye,
  EyeOff,
  FolderPlus,
  Play,
  Power,
  RefreshCw,
  Terminal,
  Wrench,
  X,
  Zap,
} from "lucide-react";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import * as codebuddyLocalAccessService from "../../services/codebuddyLocalAccessService";
import type {
  CodebuddyLocalAccessCollection,
  CodebuddyLocalAccessScope,
  CodebuddyLocalAccessState,
} from "../../types/codebuddyLocalAccess";
import "./CodebuddyApiServiceInlineCard.css";

export interface CodebuddyApiServiceInlineCardProps {
  /** 平台来源：intl / cn；决定成员账号计数来源 */
  platformRegion: "intl" | "cn";
  /** 跳转完整 API 服务页 */
  onOpenFullPage: () => void;
  /** 添加账号 */
  onAddAccount?: () => void;
  /** 关闭/隐藏卡片 */
  onClose?: () => void;
  /** 布局模式（默认 grid） */
  layoutMode?: "grid" | "list";
}

/**
 * CodeBuddy API 服务小卡片入口。
 *
 * 与 Codex 账号页的「API 服务-新」卡片同款结构（复用 codex-local-access-* 类名），
 * 在 CodeBuddy intl/cn 账号页的账号 list 顶部展示。
 *
 * - 标题行：CodeBuddy API 服务 + 站点 · 账号数摘要
 * - 状态徽章：运行中 / 未运行 / 已停用
 * - 展开区：地址（scope 下拉 + Base URL）+ 密钥 + 端口 + 账号池摘要
 * - 底部：监听范围 + icon-only 按钮（添加账号 / 打开完整页 / 刷新 / 启停 / 清理端口）
 */
export function CodebuddyApiServiceInlineCard(
  props: CodebuddyApiServiceInlineCardProps,
) {
  const {
    platformRegion,
    onOpenFullPage,
    onAddAccount,
    onClose,
    layoutMode = "grid",
  } = props;
  const [state, setState] = useState<CodebuddyLocalAccessState | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copiedKey, setCopiedKey] = useState(false);
  const [keyVisible, setKeyVisible] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refreshState = useCallback(async () => {
    setLoading(true);
    try {
      const next = await codebuddyLocalAccessService.getCodebuddyLocalAccessState();
      setState(next);
      setError(null);
    } catch (err) {
      console.error("拉取 CodeBuddy 本地访问状态失败:", err);
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refreshState();
  }, [refreshState]);

  const toggleEnabled = useCallback(async () => {
    setSaving(true);
    setError(null);
    try {
      const next = await codebuddyLocalAccessService.setCodebuddyLocalAccessEnabled(
        !state?.collection?.enabled,
      );
      setState(next);
    } catch (err) {
      console.error("切换 CodeBuddy 本地访问启用状态失败:", err);
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }, [state?.collection?.enabled]);

  const killPort = useCallback(async () => {
    const port = state?.actualPort ?? state?.collection?.port;
    if (!port) return;
    setSaving(true);
    try {
      await codebuddyLocalAccessService.killCodebuddyLocalAccessPort(port);
      await refreshState();
    } catch (err) {
      console.error("回收 CodeBuddy 反代服务端口失败:", err);
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }, [state?.actualPort, state?.collection?.port, refreshState]);

  const updateScope = useCallback(
    async (nextScope: string) => {
      if (!state?.collection) return;
      setSaving(true);
      setError(null);
      try {
        const updated: CodebuddyLocalAccessCollection = {
          ...state.collection,
          scope: nextScope as CodebuddyLocalAccessScope,
        };
        const next = await codebuddyLocalAccessService.saveCodebuddyLocalAccessCollection(
          updated,
        );
        setState(next);
      } catch (err) {
        console.error("更新 CodeBuddy 访问范围失败:", err);
        setError(String(err));
      } finally {
        setSaving(false);
      }
    },
    [state?.collection],
  );

  const copyBaseUrl = useCallback(async () => {
    const url = state?.baseUrl;
    if (!url) return;
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // ignore
    }
  }, [state?.baseUrl]);

  const copyApiKey = useCallback(async () => {
    const key = state?.collection?.apiKeys?.[0]?.key;
    if (!key) return;
    try {
      await navigator.clipboard.writeText(key);
      setCopiedKey(true);
      setTimeout(() => setCopiedKey(false), 1500);
    } catch {
      // ignore
    }
  }, [state?.collection?.apiKeys]);

  const collection = state?.collection;
  const enabled = collection?.enabled ?? false;
  const running = state?.running ?? false;

  const memberCount = useMemo(() => {
    if (!collection) return 0;
    return platformRegion === "intl"
      ? (collection.intlAccountIds?.length ?? 0)
      : (collection.cnAccountIds?.length ?? 0);
  }, [collection, platformRegion]);

  const firstApiKey = collection?.apiKeys?.[0]?.key ?? "";
  const port = collection?.port ?? state?.actualPort ?? 0;
  const baseUrl = state?.baseUrl ?? "";

  const statusLabel = enabled
    ? running
      ? "运行中"
      : "未运行"
    : "已停用";
  const statusClass = running
    ? "running"
    : enabled
      ? "stopped"
      : "disabled";

  const scopeLabel =
    collection?.scope === "lan" ? "仅本机" : "仅本机";

  const apiKeyDisplay = !firstApiKey
    ? "暂未提供"
    : keyVisible
      ? firstApiKey
      : `${firstApiKey.slice(0, 6)}••••••${firstApiKey.slice(-4)}`;

  const shownError = error ?? state?.lastError ?? null;

  return (
    <div
      className={`codex-account-card folder-inline-card codebuddy-api-inline-card codebuddy-api-inline-card--${layoutMode}`}
    >
      <div className="folder-inline-header codex-local-access-header">
        <div className="folder-inline-info">
          <div className="codex-local-access-title-row">
            <span className="codex-local-access-title-text">
              CodeBuddy API 服务
            </span>
            <span className="codex-local-access-summary-text">
              {platformRegion === "intl" ? "国际站" : "中国站"} · {memberCount}{" "}
              个账号
            </span>
          </div>
        </div>
        <div className="codex-local-access-header-actions">
          <span className={`codex-local-access-status ${statusClass}`}>
            {statusLabel}
          </span>
          {onClose && (
            <button
              type="button"
              className="folder-icon-btn codex-local-access-close-btn"
              onClick={onClose}
              title="关闭"
              aria-label="关闭"
            >
              <X size={12} />
            </button>
          )}
        </div>
      </div>

      <div className="codex-local-access-meta">
        <div className="codex-local-access-row">
          <span className="codex-local-access-label">地址</span>
          <SingleSelectDropdown
            value={collection?.scope === "lan" ? "lan" : "localhost"}
            options={[
              { value: "localhost", label: "本机" },
              { value: "lan", label: "局域网" },
            ]}
            onChange={updateScope}
            disabled={!collection}
            menuClassName="codex-local-access-address-menu"
            menuWidth={88}
            menuMaxHeight={120}
            className="codex-local-access-address-select"
            placeholder="本机"
            ariaLabel="地址类型"
          />
          <code className="codex-local-access-code" title={baseUrl}>
            {baseUrl || "-"}
          </code>
          <div className="codex-local-access-row-actions">
            <button
              type="button"
              className="folder-icon-btn"
              onClick={() => void copyBaseUrl()}
              title="复制"
              disabled={!baseUrl}
            >
              {copied ? <Check size={14} /> : <Copy size={14} />}
            </button>
          </div>
        </div>

        <div className="codex-local-access-row">
          <span className="codex-local-access-label">密钥</span>
          <code
            className="codex-local-access-code"
            title={firstApiKey || "-"}
          >
            {apiKeyDisplay}
          </code>
          <div className="codex-local-access-row-actions">
            <button
              type="button"
              className="folder-icon-btn"
              onClick={() => setKeyVisible((v) => !v)}
              title={keyVisible ? "隐藏密钥" : "显示密钥"}
              disabled={!firstApiKey}
            >
              {keyVisible ? <EyeOff size={14} /> : <Eye size={14} />}
            </button>
            <button
              type="button"
              className="folder-icon-btn"
              onClick={() => void copyApiKey()}
              title="复制"
              disabled={!firstApiKey}
            >
              {copiedKey ? <Check size={14} /> : <Copy size={14} />}
            </button>
          </div>
        </div>

        <div className="codex-local-access-row">
          <span className="codex-local-access-label">端口</span>
          <code className="codex-local-access-code">{port || "-"}</code>
        </div>

        <button
          type="button"
          className="codex-local-access-health-summary"
          title={
            memberCount > 0
              ? `账号池：${memberCount} 个账号`
              : "账号池暂无账号"
          }
        >
          <span className="codex-local-access-health-summary-title">
            账号池
          </span>
          <span
            className={`codex-local-access-health-summary-value${
              memberCount > 0 ? "" : " empty"
            }`}
          >
            {memberCount > 0 ? `全部可用 ${memberCount}` : "暂无账号"}
          </span>
        </button>

        {shownError && (
          <div className="codebuddy-api-error-row">
            <Wrench size={12} />
            <span>{shownError}</span>
            <button
              type="button"
              className="folder-icon-btn"
              onClick={() => void killPort()}
              disabled={saving || !port}
              title="清理端口"
            >
              <Wrench size={13} />
            </button>
          </div>
        )}
      </div>

      <div className="codex-card-bottom">
        <span className="card-date">监听范围：{scopeLabel}</span>
        <div className="card-footer">
          <div className="card-actions">
            {onAddAccount && (
              <button
                type="button"
                className="card-action-btn"
                onClick={onAddAccount}
                title="添加账号"
              >
                <FolderPlus size={14} />
              </button>
            )}
            <button
              type="button"
              className="card-action-btn"
              onClick={onOpenFullPage}
              title="服务面板"
            >
              <Terminal size={14} />
            </button>
            <button
              type="button"
              className="card-action-btn"
              onClick={onOpenFullPage}
              title="日志"
            >
              <Database size={14} />
            </button>
            <button
              type="button"
              className="card-action-btn"
              onClick={onOpenFullPage}
              title="打开完整页"
            >
              <ExternalLink size={14} />
            </button>
            <button
              type="button"
              className="card-action-btn"
              onClick={() => void refreshState()}
              disabled={loading}
              title="刷新配额"
            >
              <RefreshCw
                size={14}
                className={loading ? "loading-spinner" : ""}
              />
            </button>
            <button
              type="button"
              className={`card-action-btn ${enabled ? "danger" : "success"}`}
              onClick={() => void toggleEnabled()}
              disabled={saving || loading}
              title={enabled ? "停用服务" : "启用服务"}
            >
              {saving ? (
                <RefreshCw size={14} className="loading-spinner" />
              ) : enabled ? (
                <Power size={14} />
              ) : (
                <Play size={14} />
              )}
            </button>
            <button
              type="button"
              className="card-action-btn"
              onClick={() => void killPort()}
              disabled={saving || !port}
              title="清理端口"
            >
              <Wrench size={14} />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/* 兼容：未启用时用 Zap 图标作为占位提示（不影响主流程） */
export function CodebuddyApiServiceInlineCardPlaceholder() {
  return (
    <div className="codebuddy-api-inline-card codebuddy-api-inline-card-placeholder">
      <Zap size={14} />
      <span>CodeBuddy API 服务</span>
    </div>
  );
}
