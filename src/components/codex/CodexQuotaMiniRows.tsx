import type { UnifiedQuotaMetric } from "../../presentation/platformAccountPresentation";
import {
  estimateCodexWindowFullCostUsd,
  formatCodexWindowAccountEstimate,
  formatCodexWindowCostAmount,
  formatCodexWindowRequestCount,
  formatCodexWindowTokenCount,
} from "../../utils/codexWindowStats";

type Translate = {
  (key: string, defaultValue?: string): string;
  (key: string, options?: Record<string, unknown>): string;
};

function CodexQuotaMiniRow({
  item,
  t,
}: {
  item: UnifiedQuotaMetric;
  t: Translate;
}) {
  const stats = item.windowStats;
  const showProgress = item.showProgress !== false;
  // 额度百分比是「剩余可用比例」，折算时用 100 - 剩余 得到已消耗比例。
  const estimatedFullCostUsd = estimateCodexWindowFullCostUsd(
    stats,
    item.percentage,
  );
  if (!showProgress) {
    return (
      <div className="codex-quota-mini" title={item.hintText}>
        <div className="codex-quota-mini-label" title={item.hintText || item.label}>
          {item.valueText}
        </div>
        {item.resetText ? (
          <div className="codex-quota-mini-reset">{item.resetText}</div>
        ) : null}
      </div>
    );
  }
  return (
    <div className="codex-quota-mini" title={item.hintText}>
      <div className="codex-quota-mini-head">
        <span className="codex-quota-mini-label" title={item.hintText || item.label}>
          {item.label}
        </span>
        {stats ? (
          <div
            className="codex-quota-mini-right"
            title={t(
              "codex.quota.windowStatsHint",
              "本额度窗口内经由 API 服务的请求数、Token 和估算用量价值",
            )}
          >
            <span className="codex-quota-mini-chip">
              {formatCodexWindowRequestCount(stats.requestCount)} req
            </span>
            <span className="codex-quota-mini-chip">
              {formatCodexWindowTokenCount(stats.totalTokens)}
            </span>
            <span
              className="codex-quota-mini-chip"
              title={t(
                "codex.quota.windowAccountCostHint",
                "按模型 API 单价估算用量价值。订阅剩余额度以账号返回的百分比为准。",
              )}
            >
              {formatCodexWindowAccountEstimate(stats.estimatedCostUsd)}
            </span>
            {estimatedFullCostUsd != null ? (
              <span
                className="codex-quota-mini-chip"
                title={t(
                  "codex.quota.windowEstimatedFullCostHint",
                  "按已用额度比例估算的窗口满额用量价值",
                )}
              >
                ≈${formatCodexWindowCostAmount(estimatedFullCostUsd)}
              </span>
            ) : null}
            {stats.userCostUsd != null ? (
              <span
                className="codex-quota-mini-chip"
                title={t("codex.quota.windowUserCostHint", "用户计费")}
              >
                U ${formatCodexWindowCostAmount(stats.userCostUsd)}
              </span>
            ) : null}
          </div>
        ) : null}
      </div>
      <div className="codex-quota-mini-meter">
        <div className="codex-quota-mini-track" aria-hidden="true">
          <div
            className={`codex-quota-mini-bar ${item.quotaClass}`}
            style={{
              width: `${Math.max(0, Math.min(100, item.progressPercent ?? item.percentage))}%`,
            }}
          />
        </div>
        <span className={`codex-quota-mini-pct ${item.quotaClass}`}>
          {item.valueText}
        </span>
      </div>
      {item.resetText ? (
        <div className="codex-quota-mini-reset">{item.resetText}</div>
      ) : null}
    </div>
  );
}

export function CodexQuotaMiniRows({
  items,
  t,
}: {
  items: UnifiedQuotaMetric[];
  t: Translate;
}) {
  return (
    <div className="codex-quota-mini-list">
      {items.map((item) => (
        <CodexQuotaMiniRow key={item.key} item={item} t={t} />
      ))}
    </div>
  );
}
