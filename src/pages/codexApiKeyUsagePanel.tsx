import type { ReactElement } from "react";
import type { TFunction } from "i18next";
import { Database } from "lucide-react";
import { isCodexChatCompletionsApiKeyAccount, type CodexAccount } from "../types/codex";
import { isCodexTokenPlanAccount, isDeepSeekAccount } from "../utils/codexDeepSeekAccess";
import {
  formatCockpitApiInteger,
  formatCockpitApiTokenCount,
  resolveApiKeyUsageMode,
} from "./codexAccountsControllerModel";
import {
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
  t: TFunction;
  formatApiKeyUsagePercent: (summary?: CodexModelProviderUsageSummary) => number;
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

/** 渲染额度面板；非 API Key / 供应商账号返回空节点。 */
export function renderCodexApiKeyUsagePanel({
  account,
  provider,
  variant = "card",
  usageState,
  t,
  formatApiKeyUsagePercent,
  formatApiKeyUsageMoney,
  formatApiKeyUsageQuotaValue,
  formatApiKeyUsageDetailLabel,
  formatApiKeyUsageDetailValue,
  formatApiKeyUsageDetailByKey,
  findApiKeyUsageDetail,
}: CodexApiKeyUsagePanelOptions): ReactElement {
  if (
    isCodexChatCompletionsApiKeyAccount(account) &&
    !isDeepSeekAccount(account) &&
    !isCodexTokenPlanAccount(account)
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
  const isDeepSeekUsage =
    isDeepSeekAccount(account) || usageMode === "deepseek";
  const isNewApiUsage = usageMode === "new_api";
  const isSub2ApiUsage = usageMode === "sub2api";
  const isTokenPlanUsage = usageMode === "token_plan";
  const usedPercent = formatApiKeyUsagePercent(summary);
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
    const quota = resolveNewApiQuotaSnapshot(summary);
    const grantedText = formatApiKeyUsageMoney(quota.granted, summary.unit);
    const availableText = formatApiKeyUsageMoney(
      quota.available,
      summary.unit,
    );
    const expiresText =
      quota.expiresAt != null
        ? formatApiKeyUsageDetailValue({
            key: "expiresAt",
            value: String(quota.expiresAt),
          })
        : "-";
    const unlimitedText = t("codex.newApi.quota.unlimited", "不限量");
    const quotaValueText =
      summary.quotaUnlimited === true
        ? unlimitedText
        : `${availableText} / ${grantedText}`;
    const quotaBarWidth =
      summary.quotaUnlimited === true ? 100 : usedPercent;
    return (
      <div
        className="quota-item codex-api-key-quota-item new-api"
        title={`${t("codex.cockpitApi.balance", "额度")}：${quotaValueText}`}
      >
        <div className="quota-header">
          <Database size={14} />
          <span className="quota-label">
            {t("codex.cockpitApi.balance", "额度")}
          </span>
          <span className="quota-pct high">{quotaValueText}</span>
        </div>
        <div className="quota-bar-track">
          <div
            className="quota-bar high"
            style={{ width: `${quotaBarWidth}%` }}
          />
        </div>
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
    const resetDetail =
      findApiKeyUsageDetail(summary, "intervalExpiresAt") ??
      findApiKeyUsageDetail(summary, "weeklyExpiresAt") ??
      findApiKeyUsageDetail(summary, "expiresAt");
    return (
      <div
        className="quota-item codex-api-key-quota-item token-plan"
        title={`${t(
          "codex.modelProviders.usage.fields.remaining",
          "Remaining",
        )}: ${formatApiKeyUsageQuotaValue(
          summary,
          summary.quotaRemaining ?? summary.remaining,
        )}`}
      >
        <div className="quota-header">
          <Database size={14} />
          <span className="quota-label">
            {t("codex.modelProviders.usage.fields.planName", "Token Plan")}
          </span>
          <span className="quota-pct high">
            {formatApiKeyUsageQuotaValue(
              summary,
              summary.quotaRemaining ?? summary.remaining,
            )}
          </span>
        </div>
        <div className="quota-bar-track">
          <div
            className="quota-bar high"
            style={{ width: `${Math.max(0, Math.min(100, usedPercent))}%` }}
          />
        </div>
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
                    {(() => {
                      const raw = Number(
                        findApiKeyUsageDetail(summary, "totalGranted")
                          ?.value ?? NaN,
                      );
                      return Number.isFinite(raw)
                        ? formatApiKeyUsageMoney(raw, summary.unit)
                        : formatApiKeyUsageDetailByKey(
                            summary,
                            "totalGranted",
                          );
                    })()}
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
                    {(() => {
                      const raw = Number(
                        findApiKeyUsageDetail(summary, "totalAvailable")
                          ?.value ?? NaN,
                      );
                      return Number.isFinite(raw)
                        ? formatApiKeyUsageMoney(raw, summary.unit)
                        : formatApiKeyUsageDetailByKey(
                            summary,
                            "totalAvailable",
                          );
                    })()}
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
          {isNewApiUsage || isTokenPlanUsage ? (
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
              ? null
              : canRefresh
                ? t("codex.modelProviders.usage.pending", "等待查询额度")
                : t("codex.modelProviders.usage.noKey", "暂无可查询额度")}
        </div>
      )}
    </div>
  );
}
