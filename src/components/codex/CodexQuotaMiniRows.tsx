import type { UnifiedQuotaMetric } from "../../presentation/platformAccountPresentation";
import {
  formatCodexWindowCostAmount,
  formatCodexWindowRequestCount,
  formatCodexWindowTokenCount,
  type CodexWindowStats,
} from "../../utils/codexWindowStats";

const EMPTY_WINDOW_STATS: CodexWindowStats = {
  requestCount: 0,
  inputTokens: 0,
  cachedInputTokens: 0,
  outputTokens: 0,
  totalTokens: 0,
  estimatedCostUsd: 0,
};

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
  const stats =
    item.windowStats ??
    (item.key === "primary" || item.key === "secondary"
      ? EMPTY_WINDOW_STATS
      : undefined);
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
              "从本窗口满额到现在，该账号走 API 服务的请求数、token 和账号计费",
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
                "账号计费（按模型价格估算）",
              )}
            >
              A ${formatCodexWindowCostAmount(stats.estimatedCostUsd)}
            </span>
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
        {item.showProgress !== false ? (
          <div className="codex-quota-mini-track" aria-hidden="true">
            <div
              className={`codex-quota-mini-bar ${item.quotaClass}`}
              style={{
                width: `${Math.max(0, Math.min(100, item.progressPercent ?? item.percentage))}%`,
              }}
            />
          </div>
        ) : null}
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
