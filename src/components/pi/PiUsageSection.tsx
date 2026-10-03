import { useCallback, useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ModelProviderUsagePanel } from "../model-provider/ModelProviderUsagePanel";
import {
  getPiUsageState,
  loadPiAccountUsage,
  subscribePiUsage,
} from "../../services/piUsageCache";

function usedClass(used: number) {
  if (used >= 90) return "critical";
  if (used >= 70) return "low";
  if (used >= 40) return "medium";
  return "high";
}

function formatReset(seconds?: number) {
  if (!seconds) return "";
  try {
    return new Date(seconds * 1000).toLocaleString();
  } catch {
    return "";
  }
}

interface Props {
  accountId: string;
  variant: "card" | "table";
}

export function PiUsageSection({ accountId, variant }: Props) {
  const { t } = useTranslation();
  const [, setTick] = useState(0);

  useEffect(() => {
    const unsubscribe = subscribePiUsage(accountId, () => setTick((n) => n + 1));
    void loadPiAccountUsage(accountId);
    return unsubscribe;
  }, [accountId]);

  const refresh = useCallback(() => {
    void loadPiAccountUsage(accountId, true);
  }, [accountId]);

  const state = getPiUsageState(accountId);
  const items = (state?.data ?? []).filter((item) => !item.unsupported);

  const renderError = (error: string) =>
    error.includes("TOKEN_EXPIRED")
      ? t("pi.usage.expired", "令牌已失效且自动刷新失败，请重新登录")
      : error;

  return (
    <div className={`pi-usage-section ${variant}`}>
      <div className="pi-usage-header">
        <span>{t("pi.usage.title", "用量")}</span>
        <button
          type="button"
          className="pi-usage-refresh"
          onClick={refresh}
          disabled={state?.loading}
          title={t("pi.usage.refresh", "刷新用量")}
          aria-label={t("pi.usage.refresh", "刷新用量")}
        >
          <RefreshCw size={12} className={state?.loading ? "spinning" : ""} />
        </button>
      </div>
      {state?.error && <div className="pi-usage-error">{state.error}</div>}
      {!state?.loading && state?.data && items.length === 0 && (
        <div className="pi-usage-empty">
          {t("pi.usage.unsupported", "当前凭据暂不支持用量查询")}
        </div>
      )}
      {items.map((item) => (
        <div key={item.provider} className="pi-usage-provider">
          <div className="pi-usage-provider-name">
            {item.provider}
            {item.plan && <span className="pi-provider-kind">{item.plan}</span>}
            {item.balance && (
              <span className="pi-usage-balance">
                {t("pi.usage.spent", "已用")} {item.balance}
              </span>
            )}
          </div>
          {item.error && (
            <div className="pi-usage-error">{renderError(item.error)}</div>
          )}
          {item.gateway && (
            <ModelProviderUsagePanel summary={item.gateway} variant={variant} />
          )}
          {item.windows.map((w) => {
            const used = Math.round(w.used_percent);
            const left = Math.max(0, 100 - used);
            const reset = formatReset(w.reset_at);
            return (
              <div key={w.key} className="pi-usage-window">
                <div className="pi-usage-window-line">
                  <span>{w.label}</span>
                  <span>
                    {w.detail ? `${w.detail} · ` : ""}
                    {t("common.shared.quota.leftPercent", {
                      value: left,
                      defaultValue: "剩余 {{value}}%",
                    })}
                  </span>
                </div>
                <div className="quota-progress-track">
                  <div
                    className={`quota-progress-bar ${usedClass(used)}`}
                    style={{ width: `${left}%` }}
                  />
                </div>
                {reset && (
                  <div className="pi-usage-reset">
                    {t("pi.usage.resetAt", "重置于")} {reset}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      ))}
    </div>
  );
}
