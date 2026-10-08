import { useEffect, useState } from "react";
import {
  Check,
  Copy,
  Power,
  RefreshCw,
  Server,
  Waypoints,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import type {
  CodexInstanceGatewayStatus,
  CodexInstanceGatewayView,
} from "../types/codexLocalAccess";
import { summarizeCodexInstanceGateways } from "../utils/codexInstanceGatewaySummary";
import "./CodexInstanceGatewaysModal.css";

interface CodexInstanceGatewaysModalProps {
  isOpen: boolean;
  gateways: CodexInstanceGatewayView[];
  loading: boolean;
  error: string;
  maskAccountText?: (value: string) => string;
  onRefresh: () => void;
  onStopGateway?: (gateway: CodexInstanceGatewayView) => Promise<boolean>;
  onRestartGateway?: (gateway: CodexInstanceGatewayView) => Promise<boolean>;
  onClose: () => void;
}

const STATUS_TONE: Record<CodexInstanceGatewayStatus, string> = {
  running: "success",
  unreachable: "error",
  portConflict: "error",
  stopped: "warning",
  notStarted: "muted",
};

/** 实例级本地网关总览弹框（只读展示运行态）。 */
export function CodexInstanceGatewaysModal({
  isOpen,
  gateways,
  loading,
  error,
  maskAccountText,
  onRefresh,
  onStopGateway,
  onRestartGateway,
  onClose,
}: CodexInstanceGatewaysModalProps) {
  const { t } = useTranslation();
  const [copiedGatewayId, setCopiedGatewayId] = useState<string | null>(null);
  const [busyGatewayId, setBusyGatewayId] = useState<string | null>(null);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  useEffect(() => {
    if (!isOpen) {
      setCopiedGatewayId(null);
      setBusyGatewayId(null);
    }
  }, [isOpen]);

  if (!isOpen) {
    return null;
  }

  const { total, running, issues } = summarizeCodexInstanceGateways(gateways);

  const kindLabel = (kind: CodexInstanceGatewayView["kind"]) => {
    switch (kind) {
      case "mixedModel":
        return t("codex.instanceGateways.kindMixedModel", "混合模型路由");
      case "boundOauth":
        return t("codex.instanceGateways.kindBoundOauth", "OAuth 本地网关");
      default:
        return t("codex.instanceGateways.kindProviderGateway", "供应商网关");
    }
  };

  const statusLabel = (status: CodexInstanceGatewayStatus) => {
    switch (status) {
      case "running":
        return t("codex.localAccess.statusRunning", "运行中");
      case "unreachable":
        return t("codex.instanceGateways.statusUnreachable", "已启动但不可用");
      case "portConflict":
        return t("codex.instanceGateways.statusPortConflict", "端口被占用");
      case "notStarted":
        return t("codex.instanceGateways.statusNotStarted", "未启动");
      default:
        return t("codex.localAccess.statusStopped", "未运行");
    }
  };

  const handleCopyAddress = async (
    gateway: CodexInstanceGatewayView,
  ): Promise<void> => {
    const address = gateway.baseUrl?.trim();
    if (!address) return;
    try {
      await navigator.clipboard.writeText(address);
      setCopiedGatewayId(gateway.id);
      window.setTimeout(() => {
        setCopiedGatewayId((current) =>
          current === gateway.id ? null : current,
        );
      }, 1200);
    } catch (copyError) {
      console.error("Failed to copy gateway address:", copyError);
    }
  };

  const handleGatewayAction = async (
    gateway: CodexInstanceGatewayView,
    action?: (gateway: CodexInstanceGatewayView) => Promise<boolean>,
  ): Promise<void> => {
    if (!action || busyGatewayId) return;
    setBusyGatewayId(gateway.id);
    try {
      await action(gateway);
    } finally {
      setBusyGatewayId(null);
    }
  };

  return (
    <div className="modal-overlay codex-instance-gateways-overlay">
      <div
        className="modal codex-instance-gateways-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="codex-instance-gateways-title"
      >
        <div className="modal-header codex-instance-gateways-header">
          <div>
            <div className="codex-instance-gateways-title-row">
              <Waypoints size={18} />
              <h3 id="codex-instance-gateways-title">
                {t("codex.instanceGateways.title", "实例网关")}
              </h3>
            </div>
            <p>
              {t(
                "codex.instanceGateways.subtitle",
                "随 Codex 实例启动的本地网关",
              )}
            </p>
          </div>
          <div className="codex-instance-gateways-header-actions">
            <button
              type="button"
              className="btn btn-secondary btn-sm"
              onClick={onRefresh}
              disabled={loading}
            >
              <RefreshCw
                size={14}
                className={loading ? "loading-spinner" : ""}
              />
              <span>{t("common.refresh", "刷新")}</span>
            </button>
            <button
              type="button"
              className="folder-icon-btn"
              onClick={onClose}
              title={t("common.close", "关闭")}
              aria-label={t("common.close", "关闭")}
            >
              <X size={14} />
            </button>
          </div>
        </div>

        <div className="modal-body codex-instance-gateways-body">
          {total > 0 && (
            <div className="codex-instance-gateways-summary">
              {t("codex.instanceGateways.summary", {
                total,
                running,
                issues,
                defaultValue: "共 {{total}} · 运行中 {{running}} · 异常 {{issues}}",
              })}
            </div>
          )}
          {error && (
            <div className="codex-instance-gateways-error">
              {t("codex.instanceGateways.loadFailed", "读取实例网关失败")}：
              {error}
            </div>
          )}
          {loading && total === 0 && (
            <div className="codex-instance-gateways-empty">
              {t("common.loading", "加载中...")}
            </div>
          )}
          {!loading && !error && total === 0 && (
            <div className="codex-instance-gateways-empty">
              <Server size={22} />
              <strong>
                {t("codex.instanceGateways.empty", "当前没有实例级网关")}
              </strong>
              <span>
                {t(
                  "codex.instanceGateways.emptyHint",
                  "绑定 Chat 协议 API Key 账号或开启混合模型路由后会自动出现。",
                )}
              </span>
            </div>
          )}
          {gateways.map((gateway) => {
            const address = gateway.baseUrl?.trim() || "-";
            const accountLabel = gateway.accountLabel?.trim();
            const displayAccount = accountLabel
              ? maskAccountText
                ? maskAccountText(accountLabel)
                : accountLabel
              : gateway.accountId || "-";
            const instanceLabel = gateway.isDefault
              ? t("instances.defaultName", "Default Instance")
              : gateway.instanceName || gateway.instanceId;
            const models = gateway.upstreamModels.filter(Boolean);
            return (
              <div className="codex-instance-gateways-row" key={gateway.id}>
                <div className="codex-instance-gateways-row-main">
                  <div className="codex-instance-gateways-row-head">
                    <span className="codex-instance-gateways-kind">
                      {kindLabel(gateway.kind)}
                    </span>
                    <strong
                      className="codex-instance-gateways-row-name"
                      title={instanceLabel}
                    >
                      {instanceLabel}
                    </strong>
                    <span
                      className={`codex-instance-gateways-status ${
                        STATUS_TONE[gateway.status] ?? "muted"
                      }`}
                    >
                      {statusLabel(gateway.status)}
                    </span>
                  </div>
                  <dl className="codex-instance-gateways-fields">
                    <div className="codex-instance-gateways-field">
                      <dt>
                        {t("codex.instanceGateways.fieldAccount", "账号")}
                      </dt>
                      <dd title={displayAccount}>{displayAccount}</dd>
                    </div>
                    <div className="codex-instance-gateways-field">
                      <dt>
                        {t("codex.instanceGateways.fieldAddress", "本地地址")}
                      </dt>
                      <dd className="codex-instance-gateways-field-address">
                        <code title={address}>{address}</code>
                        <button
                          type="button"
                          className="folder-icon-btn codex-instance-gateways-copy-btn"
                          onClick={() => void handleCopyAddress(gateway)}
                          disabled={!gateway.baseUrl}
                          title={t("common.copy", "复制")}
                          aria-label={t("common.copy", "复制")}
                        >
                          {copiedGatewayId === gateway.id ? (
                            <Check size={14} />
                          ) : (
                            <Copy size={14} />
                          )}
                        </button>
                      </dd>
                    </div>
                    {gateway.wireApi && (
                      <div className="codex-instance-gateways-field">
                        <dt>
                          {t("codex.instanceGateways.fieldProtocol", "协议")}
                        </dt>
                        <dd>
                          <span className="codex-instance-gateways-chip">
                            {gateway.wireApi}
                          </span>
                        </dd>
                      </div>
                    )}
                    {models.length > 0 && (
                      <div className="codex-instance-gateways-field">
                        <dt>
                          {t("codex.instanceGateways.fieldModels", "上游模型")}
                        </dt>
                        <dd title={models.join(", ")}>
                          {models.slice(0, 2).join(", ")}
                          {models.length > 2 ? ` +${models.length - 2}` : ""}
                        </dd>
                      </div>
                    )}
                  </dl>
                  {gateway.lastError && (
                    <div
                      className="codex-instance-gateways-row-error"
                      title={gateway.lastError}
                    >
                      {gateway.lastError}
                    </div>
                  )}
                </div>
                <div className="codex-instance-gateways-row-actions">
                  {onRestartGateway && (
                    <button
                      type="button"
                      className="btn btn-secondary btn-sm"
                      onClick={() =>
                        void handleGatewayAction(gateway, onRestartGateway)
                      }
                      disabled={Boolean(busyGatewayId) || loading}
                      title={t("codex.instanceGateways.restart", "重启")}
                    >
                      <RefreshCw
                        size={14}
                        className={
                          busyGatewayId === gateway.id ? "loading-spinner" : ""
                        }
                      />
                      <span>
                        {t("codex.instanceGateways.restart", "重启")}
                      </span>
                    </button>
                  )}
                  {onStopGateway && (
                    <button
                      type="button"
                      className="btn btn-outline btn-sm"
                      onClick={() =>
                        void handleGatewayAction(gateway, onStopGateway)
                      }
                      disabled={Boolean(busyGatewayId) || loading}
                      title={t("codex.instanceGateways.stop", "关闭")}
                    >
                      <Power size={14} />
                      <span>{t("codex.instanceGateways.stop", "关闭")}</span>
                    </button>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

export default CodexInstanceGatewaysModal;
