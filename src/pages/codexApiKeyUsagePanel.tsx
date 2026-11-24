import { getCockpitApiStatsRecord, toCockpitApiRecord } from "./codexAccountsControllerModel";
import { CodexRemoteServicePanel } from '../components/codex/CodexRemoteServicePanel';
import { isOpenAIOfficialUsageAccount } from "../utils/openaiOfficialUsage";
import { OpenAIOfficialUsagePanel } from "../components/codex/OpenAIOfficialUsagePanel";
import type { ReactElement } from "react";
import type { TFunction } from "i18next";
import { Database } from "lucide-react";
import { isCodexChatCompletionsApiKeyAccount, type CodexAccount } from "../types/codex";
import { getClaudeQuotaClass } from "../types/claude";
import { isCodexApiKeyUsageQueryEligible, isDeepSeekAccount } from "../utils/codexDeepSeekAccess";
import { ModelProviderUsagePanel } from "../components/model-provider/ModelProviderUsagePanel";
import { CodexQuotaMiniRows } from "../components/codex/CodexQuotaMiniRows";
import {
  formatCockpitApiInteger,
  formatCockpitApiTokenCount,
  resolveApiKeyUsageMode,
} from "./codexAccountsControllerModel";
import {
  presentNewApiQuota,
  resolveNewApiQuotaSnapshot,
} from "../services/modelProviderUsageService";
import type {
  CodexModelProvider,
  CodexModelProviderUsageSummary,
} from "../services/codexModelProviderService";
import type { CodexApiKeyUsageState } from "../services/codexApiKeyUsageRefreshService";

/** API Key / 模型供应商账号的额度面板入参（从 useCodexAccountsAccessController 拆出）。 */
export interface CodexApiKeyUsagePanelOptions {
  account: CodexAccount;
  provider: CodexModelProvider | null;
  variant?: "card" | "table";
  usageState: CodexApiKeyUsageState | undefined;
  onRefresh?: () => void;
  t: TFunction;
  formatApiKeyUsagePercent: (summary?: CodexModelProviderUsageSummary) => number | null;
  formatApiKeyUsageMoney: (value?: number | null, unit?: string | null) => string;
  formatApiKeyUsageQuotaValue: (
    summary: CodexModelProviderUsageSummary | undefined,
    value?: number | null,
  ) => string;
  formatApiKeyUsageDetailLabel: (key: string, fallback: string) => string;
  formatApiKeyUsageDetailValue: (
    item: { key: string; value: string },
    unit?: string | null,
  ) => string;
  formatApiKeyUsageDetailByKey: (
    summary: CodexModelProviderUsageSummary | undefined,
    key: string,
  ) => string;
  findApiKeyUsageDetail: (
    summary: CodexModelProviderUsageSummary | undefined,
    key: string,
  ) => { key: string; label: string; value: string } | undefined;
}

export function resolveCodexAccountDailyUsage(account: CodexAccount, summary?: CodexModelProviderUsageSummary): CodexModelProviderUsageSummary | undefined {
  if (summary) return summary;
  const stats = getCockpitApiStatsRecord(account);
  if (!stats) return undefined;
  const requests = toCockpitApiRecord(stats.requests);
  const todayRequests = typeof requests?.today === "number" ? requests.today : undefined;
  return { todayRequests, modelStatsCount: 0, latencyMs: 0 };
}

/** 渲染额度面板；非 API Key / 供应商账号返回空节点。 */
export function renderCodexApiKeyUsagePanel({
  account,
  provider,
  variant = "card",
  usageState,
  onRefresh,
  t,
  formatApiKeyUsagePercent,
  formatApiKeyUsageMoney,
  formatApiKeyUsageQuotaValue,
  formatApiKeyUsageDetailLabel,
  formatApiKeyUsageDetailValue,
  formatApiKeyUsageDetailByKey,
  findApiKeyUsageDetail,
}: CodexApiKeyUsagePanelOptions): ReactElement {
  if (usageState?.summary?.cockpit) return <CodexRemoteServicePanel state={usageState} variant={variant} t={t} />;
  if (isOpenAIOfficialUsageAccount(account, provider?.baseUrl)) {
    return <OpenAIOfficialUsagePanel provider={provider} apiKey={account.openai_api_key ?? ""} sourceBinding={usageState?.sourceBinding} onRefresh={onRefresh} summary={resolveCodexAccountDailyUsage(account, usageState?.summary)} loading={usageState?.loading} error={usageState?.error} variant={variant} t={t} />;
  }
  if (
    isCodexChatCompletionsApiKeyAccount(account) &&
    !isCodexApiKeyUsageQueryEligible(account, provider?.customUsage ? "custom" : provider?.integrationType, usageState?.summary)
  ) {
    return <></>;
  }
  const summary = usageState?.summary;
  const loading = usageState?.loading === true;
  const apiKey = (account.openai_api_key || "").trim();
  const baseUrl =
    provider?.baseUrl.trim() || (account.api_base_url || "").trim();
  const canRefresh = Boolean(apiKey && baseUrl);
  const usageMode = resolveApiKeyUsageMode(summary);
  if (usageMode === "custom") {
    return <ModelProviderUsagePanel summary={summary} variant={variant} />;
  }
  const isDeepSeekUsage =
    isDeepSeekAccount(account) || usageMode === "deepseek";
  const isNewApiUsage = usageMode === "new_api";
  const isSub2ApiUsage = usageMode === "sub2api";
  const isTokenPlanUsage = usageMode === "token_plan";
  const usedPercent = formatApiKeyUsagePercent(summary);
  const newApiQuota = resolveNewApiQuotaSnapshot(summary);
  if (isDeepSeekUsage) {
    return (
      <div className={`codex-api-key-usage-panel ${variant} sub2api`}>
        <div className="codex-api-key-usage-grid">
          <div>
            <span>
              {t(
                "codex.modelProviders.usage.fields.totalBalance",
                "总余额",
              )}
            </span>
            <strong>
              {formatApiKeyUsageMoney(summary?.balance, summary?.unit)}
            </strong>
          </div>
          <div>
            <span>
              {t(
                "codex.modelProviders.usage.fields.grantedBalance",
                "赠金余额",
              )}
            </span>
            <strong>
              {formatApiKeyUsageDetailByKey(summary, "grantedBalance")}
            </strong>
          </div>
          <div>
            <span>
              {t(
                "codex.modelProviders.usage.fields.toppedUpBalance",
                "充值余额",
              )}
            </span>
            <strong>
              {formatApiKeyUsageDetailByKey(summary, "toppedUpBalance")}
            </strong>
          </div>
        </div>
        {!summary && usageState?.error ? (
          <div className="codex-api-key-usage-empty">
            {t("common.shared.quota.queryFailed", "配额查询失败")}
          </div>
        ) : null}
      </div>
    );
  }
  if (variant === "card" && summary && isNewApiUsage) {
    const quota = presentNewApiQuota(
      summary,
      t("codex.newApi.quota.unlimited", "不限量"),
      t("dashboard.noData", "暂无数据"),
    );
    const expiresText =
      quota.expiresAt != null
        ? formatApiKeyUsageDetailValue({
            key: "expiresAt",
            value: String(quota.expiresAt),
          })
        : "-";
    return (
      <div
        className="quota-item codex-api-key-quota-item new-api"
        title={`${t("codex.cockpitApi.balance", "额度")}：${quota.text}`}
      >
        <div className="quota-header">
          <Database size={14} />
          <span className="quota-label">
            {t("codex.cockpitApi.balance", "额度")}
          </span>
          <span className="quota-pct high">{quota.text}</span>
        </div>
        {quota.percent != null && (
          <div className="quota-bar-track">
            <div className="quota-bar high" style={{ width: `${quota.percent}%` }} />
          </div>
        )}
        {expiresText !== "-" && (
          <span className="quota-reset">
            {t("codex.modelProviders.usage.fields.expiresAt", "过期时间")}：
            {expiresText}
          </span>
        )}
      </div>
    );
  }
  if (variant === "card" && summary && isTokenPlanUsage) {
    const windows = summary.quotaUnlimited ? [] : [
      { key: "interval", label: t("codex.localAccess.quotaPool.hourlyShort") },
      { key: "weekly", label: t("codex.localAccess.quotaPool.weeklyShort") },
      { key: "monthly", label: t("codex.localAccess.statsRange.monthly") },
    ].flatMap(({ key, label }) => {
      const value = findApiKeyUsageDetail(summary, `${key}RemainingPercent`)?.value.trim();
      const remaining = value ? Number(value) : NaN;
      if (!Number.isFinite(remaining)) return [];
      const percentage = Math.max(0, Math.min(100, Math.round(remaining)));
      const reset = findApiKeyUsageDetail(summary, `${key}ExpiresAt`);
      const resetText = reset ? formatApiKeyUsageDetailValue(reset, summary.unit) : undefined;
      return [{
        key,
        label,
        percentage,
        quotaClass: getClaudeQuotaClass(100 - percentage),
        valueText: formatApiKeyUsageMoney(percentage, "%"),
        resetText: resetText !== "-" ? resetText : undefined,
      }];
    });
    if (windows.length > 1) {
      return (
        <div className="codex-api-key-quota-item token-plan" title={summary.planName ?? undefined}>
          <CodexQuotaMiniRows items={windows} t={t} />
        </div>
      );
    }
    const remainingQuota = summary.quotaRemaining ?? summary.remaining;
    const quotaLimit = summary.quotaLimit;
    let remainingPercent: number | null = null;
    if (!summary.quotaUnlimited && remainingQuota != null && Number.isFinite(remainingQuota)) {
      const percent = summary.unit?.trim() === "%"
        ? remainingQuota
        : quotaLimit != null && Number.isFinite(quotaLimit) && quotaLimit > 0
          ? (remainingQuota / quotaLimit) * 100
          : null;
      if (percent != null) {
        remainingPercent = Math.max(0, Math.min(100, Math.round(percent)));
      }
    }
    const quotaBarClass = remainingPercent == null ? "" : getClaudeQuotaClass(100 - remainingPercent);
    const resetDetail =
      findApiKeyUsageDetail(summary, "intervalExpiresAt") ??
      findApiKeyUsageDetail(summary, "weeklyExpiresAt") ??
      findApiKeyUsageDetail(summary, "expiresAt");
    // 智谱等 coding plan 是「5 小时 + 每周」双窗：主数字是 5 小时窗剩余，
    // 把每周窗剩余并排在旁边，避免用户只看到一个数字而误以为没有周限。
    const weeklyRemainingDetail = findApiKeyUsageDetail(summary, "weeklyRemainingPercent");
    const weeklyRemaining = Number.parseFloat(weeklyRemainingDetail?.value ?? "");
    return (
      <div
        className="quota-item codex-api-key-quota-item token-plan"
        title={`${t(
          "codex.modelProviders.usage.fields.remaining",
          "Remaining",
        )}: ${formatApiKeyUsageQuotaValue(summary, remainingQuota)}`}
      >
        <div className="quota-header">
          <Database size={14} />
          <span className="quota-label">
            {t("codex.modelProviders.usage.fields.planName", "Token Plan")}
          </span>
          <span className={`quota-pct ${quotaBarClass}`}>
            {formatApiKeyUsageQuotaValue(summary, remainingQuota)}
            {Number.isFinite(weeklyRemaining) && (
              <span>
                {" · "}
                {t("codex.modelProviders.usage.fields.weeklyShort", "Weekly")}{" "}
                {formatApiKeyUsageMoney(weeklyRemaining, "%")}
              </span>
            )}
          </span>
        </div>
        {remainingPercent != null && (
          <div className="quota-bar-track">
            <div
              className={`quota-bar ${quotaBarClass}`}
              style={{ width: `${remainingPercent}%` }}
            />
          </div>
        )}
        {(summary.planName || resetDetail) && (
          <span className="quota-reset">
            {summary.planName || "Token Plan"}
            {resetDetail
              ? ` · ${formatApiKeyUsageDetailValue(resetDetail, summary.unit)}`
              : ""}
          </span>
        )}
      </div>
    );
  }
  if (variant === "card" && summary && isSub2ApiUsage) {
    return (
      <div className="codex-api-key-usage-panel sub2api">
        <div className="codex-api-key-usage-grid">
          <div>
            <span>
              {t("codex.modelProviders.usage.accountBalance", "账户余额")}
            </span>
            <strong>
              {formatApiKeyUsageQuotaValue(
                summary,
                summary.remaining ??
                  summary.balance ??
                  summary.quotaRemaining,
              )}
            </strong>
          </div>
          <div>
            <span>
              {t(
                "codex.modelProviders.usage.fields.todayRequests",
                "今日请求",
              )}
            </span>
            <strong>
              {formatCockpitApiInteger(summary.todayRequests ?? 0)}
            </strong>
          </div>
          <div>
            <span>
              {t(
                "codex.modelProviders.usage.fields.todayTokens",
                "今日 Token",
              )}
            </span>
            <strong>
              {formatCockpitApiTokenCount(summary.todayTotalTokens ?? 0)}
            </strong>
          </div>
        </div>
      </div>
    );
  }
  if (summary && !usageMode) {
    return <></>;
  }
  return (
    <div
      className={`codex-api-key-usage-panel ${variant} ${summary ? "" : "empty"}`}
    >
      {summary ? (
        <>
          <div className="codex-api-key-usage-grid">
            {isDeepSeekUsage ? (
              <>
                {[
                  ["totalBalance", "总余额"],
                  ["grantedBalance", "赠金余额"],
                  ["toppedUpBalance", "充值余额"],
                ].map(([key, fallback]) => (
                  <div key={key}>
                    <span>
                      {formatApiKeyUsageDetailLabel(key, fallback)}
                    </span>
                    <strong>
                      {formatApiKeyUsageDetailByKey(summary, key)}
                    </strong>
                  </div>
                ))}
              </>
            ) : isNewApiUsage ? (
              <>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.totalGranted",
                      "授予额度",
                    )}
                  </span>
                  <strong>
                    {formatApiKeyUsageMoney(newApiQuota.granted, summary.unit)}
                  </strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.totalAvailable",
                      "可用额度",
                    )}
                  </span>
                  <strong>
                    {formatApiKeyUsageMoney(newApiQuota.available, summary.unit)}
                  </strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.expiresAt",
                      "过期时间",
                    )}
                  </span>
                  <strong>
                    {formatApiKeyUsageDetailByKey(summary, "expiresAt")}
                  </strong>
                </div>
              </>
            ) : isTokenPlanUsage ? (
              <>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.remaining",
                      "Remaining",
                    )}
                  </span>
                  <strong>
                    {formatApiKeyUsageQuotaValue(
                      summary,
                      summary.quotaRemaining ?? summary.remaining,
                    )}
                  </strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.planName",
                      "Plan",
                    )}
                  </span>
                  <strong>{summary.planName || "-"}</strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.expiresAt",
                      "Next Reset",
                    )}
                  </span>
                  <strong>
                    {formatApiKeyUsageDetailByKey(
                      summary,
                      findApiKeyUsageDetail(summary, "intervalExpiresAt")
                        ? "intervalExpiresAt"
                        : findApiKeyUsageDetail(summary, "weeklyExpiresAt")
                          ? "weeklyExpiresAt"
                          : "expiresAt",
                    )}
                  </strong>
                </div>
              </>
            ) : isSub2ApiUsage ? (
              <>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.accountBalance",
                      "账户余额",
                    )}
                  </span>
                  <strong>
                    {formatApiKeyUsageQuotaValue(
                      summary,
                      summary.remaining ??
                        summary.balance ??
                        summary.quotaRemaining,
                    )}
                  </strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.todayRequests",
                      "今日请求",
                    )}
                  </span>
                  <strong>
                    {formatCockpitApiInteger(summary.todayRequests ?? 0)}
                  </strong>
                </div>
                <div>
                  <span>
                    {t(
                      "codex.modelProviders.usage.fields.todayTokens",
                      "今日 Token",
                    )}
                  </span>
                  <strong>
                    {formatCockpitApiTokenCount(
                      summary.todayTotalTokens ?? 0,
                    )}
                  </strong>
                </div>
              </>
            ) : null}
          </div>
          {isNewApiUsage ? (
            <div className="codex-api-key-usage-progress">
              <div className="cockpit-api-progress-track">
                <div
                  className="cockpit-api-progress-bar"
                  style={{ width: `${usedPercent}%` }}
                />
              </div>
              <span>{usedPercent}%</span>
            </div>
          ) : null}
        </>
      ) : (
        <div className="codex-api-key-usage-empty">
          {loading
            ? t("codex.modelProviders.usage.loading", "正在查询额度...")
            : usageState?.error
              ? t("common.shared.quota.queryFailed", "配额查询失败")
              : canRefresh && !usageState?.unavailable
                ? t("codex.modelProviders.usage.pending", "等待查询额度")
                : t("codex.modelProviders.usage.noKey", "暂无可查询额度")}
        </div>
      )}
    </div>
  );
}
