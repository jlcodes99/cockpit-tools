import { Copy, Eye, EyeOff, X } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

export interface CodexServicePanelMetricItem {
  key: string;
  label: string;
  value: string;
  meta?: string;
  rawKey?: string;
}

export interface CodexServicePanelActionItem {
  key: string;
  label: string;
  variant?: "primary" | "secondary";
  icon?: ReactNode;
  disabled?: boolean;
  onClick: () => void;
}

interface CodexServicePanelModalProps {
  open: boolean;
  title: string;
  subtitle?: string;
  baseUrl: string;
  apiKeyDisplay: string;
  rawApiKey?: string;
  apiKeyVisible?: boolean;
  onClose: () => void;
  onToggleApiKeyVisible?: () => void;
  coreMetrics: CodexServicePanelMetricItem[];
  detailMetrics: CodexServicePanelMetricItem[];
  actions: CodexServicePanelActionItem[];
  connectionExtra?: ReactNode;
  emptyDetailText?: string;
}

export function CodexServicePanelModal({
  open,
  title,
  subtitle,
  baseUrl,
  apiKeyDisplay,
  rawApiKey,
  apiKeyVisible = false,
  onClose,
  onToggleApiKeyVisible,
  coreMetrics,
  detailMetrics,
  actions,
  connectionExtra,
  emptyDetailText,
}: CodexServicePanelModalProps) {
  const { t } = useTranslation();
  if (!open) return null;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div
        className="modal-content cockpit-api-panel-modal"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="modal-header cockpit-api-panel-header">
          <div>
            <h2>{title}</h2>
            {subtitle ? (
              <span className="cockpit-api-panel-subtitle">{subtitle}</span>
            ) : null}
          </div>
          <button
            className="modal-close"
            onClick={onClose}
            aria-label={t("common.close", "关闭")}
          >
            <X />
          </button>
        </div>

        <div className="cockpit-api-panel-body">
          <section className="cockpit-api-connection-card">
            <div className="cockpit-api-connection-row">
              <span>{t("codex.localAccess.baseUrl", "地址")}</span>
              <code title={baseUrl}>{baseUrl}</code>
              <button
                type="button"
                className="folder-icon-btn cockpit-api-icon-btn"
                onClick={() =>
                  void navigator.clipboard.writeText(baseUrl).catch(() => {})
                }
                title={t("common.copy", "复制")}
              >
                <Copy size={14} />
              </button>
            </div>
            <div className="cockpit-api-connection-row">
              <span>{t("codex.localAccess.apiKey", "密钥")}</span>
              <code title={apiKeyVisible ? rawApiKey || "" : ""}>
                {apiKeyDisplay}
              </code>
              <div className="cockpit-api-connection-actions">
                {onToggleApiKeyVisible ? (
                  <button
                    type="button"
                    className="folder-icon-btn cockpit-api-icon-btn"
                    onClick={onToggleApiKeyVisible}
                    title={
                      apiKeyVisible
                        ? t("codex.localAccess.hideKey", "隐藏密钥")
                        : t("codex.localAccess.showKey", "显示密钥")
                    }
                  >
                    {apiKeyVisible ? <EyeOff size={14} /> : <Eye size={14} />}
                  </button>
                ) : null}
                <button
                  type="button"
                  className="folder-icon-btn cockpit-api-icon-btn"
                  onClick={() =>
                    void navigator.clipboard.writeText(rawApiKey || "").catch(() => {})
                  }
                  title={t("common.copy", "复制")}
                  disabled={!rawApiKey}
                >
                  <Copy size={14} />
                </button>
              </div>
            </div>
            {connectionExtra}
          </section>

          <section className="cockpit-api-summary-grid compact">
            {coreMetrics.map((item) => (
              <div
                className="cockpit-api-stat-card cockpit-api-stat-card-center"
                key={item.key}
              >
                <span className="cockpit-api-card-label">{item.label}</span>
                <strong>{item.value}</strong>
                {item.meta ? <small>{item.meta}</small> : null}
              </div>
            ))}
          </section>

          <section className="cockpit-api-panel-section">
            <div className="cockpit-api-section-head">
              <strong>
                {t("codex.modelProviders.usage.rawFields", "服务数据")}
              </strong>
            </div>
            <div className="cockpit-api-usage-card-grid">
              {detailMetrics.length > 0 ? (
                detailMetrics.map((item) => (
                  <div className="cockpit-api-usage-card" key={item.key}>
                    <span className="cockpit-api-card-label">{item.label}</span>
                    <strong>{item.value}</strong>
                    {item.rawKey ? <small>{item.rawKey}</small> : null}
                  </div>
                ))
              ) : (
                <div className="cockpit-api-empty-row">
                  {emptyDetailText || t("codex.cockpitApi.noStats", "暂无统计")}
                </div>
              )}
            </div>
          </section>
        </div>

        <div className="modal-footer cockpit-api-panel-footer">
          {actions.map((action) => (
            <button
              key={action.key}
              className={`btn ${action.variant === "primary" ? "btn-primary" : "btn-secondary"}`}
              onClick={action.onClick}
              disabled={action.disabled}
            >
              {action.icon}
              {action.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
