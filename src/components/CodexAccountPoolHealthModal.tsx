import { useMemo } from "react";
import { CircleAlert, RefreshCw, ShieldCheck, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { CodexAccount } from "../types/codex";
import type {
  CodexLocalAccessAccountHealth,
  CodexLocalAccessAccountPoolHealth,
  CodexLocalAccessAccountCooldown,
} from "../types/codexLocalAccess";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";
import { isBlockingCodexAccountQuotaError } from "../utils/codexQuotaError";
import { resolveCodexHealthIssueDisplayName } from "../utils/codexAccountDisplayName";
import {
  ModalErrorMessage,
  useModalErrorState,
} from "./ModalErrorMessage";
import "./CodexAccountPoolHealthModal.css";

interface CodexAccountPoolHealthModalProps {
  isOpen: boolean;
  accountIds: string[];
  accounts: CodexAccount[];
  accountHealth: CodexLocalAccessAccountHealth[];
  accountPoolHealth: CodexLocalAccessAccountPoolHealth[];
  actionBusy: boolean;
  maskAccountText?: (value: string) => string;
  onClose: () => void;
  onRecover: (accountId: string) => Promise<void>;
  onRecoverAll: (accountIds: string[]) => Promise<void>;
}

type HealthIssueKind =
  | "missing"
  | "cooldown"
  | "auth"
  | "quota"
  | "unavailable";

interface HealthIssue {
  accountId: string;
  displayName: string;
  planLabel: string | null;
  planClass: string | null;
  quotaItems: Array<{ key: string; label: string; valueText: string; quotaClass: string }>;
  kind: HealthIssueKind;
  health: CodexLocalAccessAccountHealth | null;
}

function resolveIssueDisplayName(
  account: CodexAccount | undefined,
  health: CodexLocalAccessAccountHealth | null,
  accountId: string,
): string {
  return resolveCodexHealthIssueDisplayName(
    account?.account_name,
    account?.email,
    health?.email,
    accountId,
  );
}

function issueKindForHealth(
  account: CodexAccount | undefined,
  health: CodexLocalAccessAccountHealth | null,
): HealthIssueKind | null {
  if (!account) return "missing";
  if (health?.cooldowns?.length) return "cooldown";
  if (
    health?.schedulerReason === "unauthorized" ||
    health?.lastFailureCategory === "auth_unavailable" ||
    health?.lastFailureCategory === "auth_refresh_failed"
  ) {
    return "auth";
  }
  if (
    health?.schedulerReason === "quota" ||
    isBlockingCodexAccountQuotaError(account)
  ) {
    return "quota";
  }
  if (health?.schedulerAvailable === false || health?.available === false) {
    return "unavailable";
  }
  return null;
}

function formatCooldown(
  cooldown: CodexLocalAccessAccountCooldown,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  const model = cooldown.modelId.trim() || t("common.unknown", "未知模型");
  if (!cooldown.nextRetryAt) {
    return t("codex.localAccess.accountPoolHealth.dialog.cooldownModel", {
      model,
      defaultValue: "模型 {{model}} 处于冷却状态",
    });
  }
  const time = new Date(cooldown.nextRetryAt).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
  return t("codex.localAccess.accountPoolHealth.dialog.cooldownUntil", {
    model,
    time,
    defaultValue: "模型 {{model}} 冷却至 {{time}}",
  });
}

export function CodexAccountPoolHealthModal({
  isOpen,
  accountIds,
  accounts,
  accountHealth,
  accountPoolHealth,
  actionBusy,
  maskAccountText,
  onClose,
  onRecover,
  onRecoverAll,
}: CodexAccountPoolHealthModalProps) {
  const { t } = useTranslation();
  const {
    message: recoveryError,
    scrollKey: recoveryErrorScrollKey,
    set: setRecoveryError,
  } = useModalErrorState();
  const issues = useMemo<HealthIssue[]>(() => {
    const accountsById = new Map(accounts.map((account) => [account.id, account]));
    const healthById = new Map(
      accountHealth.map((health) => [health.accountId, health]),
    );
    return accountIds.flatMap((accountId) => {
      const account = accountsById.get(accountId);
      const health = healthById.get(accountId) ?? null;
      const kind = issueKindForHealth(account, health);
      if (!kind) return [];
      const rawName = resolveIssueDisplayName(account, health, accountId);
      const displayName = maskAccountText ? maskAccountText(rawName) : rawName;
      const presentation = account
        ? buildCodexAccountPresentation(account, t)
        : null;
      return [{
        accountId,
        displayName,
        planLabel: presentation?.planLabel?.trim() || null,
        planClass: presentation?.planClass || null,
        quotaItems: (presentation?.quotaItems ?? [])
          .filter((item) => item.valueText.trim().length > 0)
          .slice(0, 3)
          .map((item) => ({
            key: item.key,
            label: item.label,
            valueText: item.valueText,
            quotaClass: item.quotaClass,
          })),
        kind,
        health,
      }];
    });
  }, [accountHealth, accountIds, accounts, maskAccountText, t]);

  if (!isOpen) return null;

  const issueLabel = (kind: HealthIssueKind): string => {
    switch (kind) {
      case "missing":
        return t("codex.localAccess.accountPoolHealth.dialog.missing", "账号缺失");
      case "cooldown":
        return t("codex.localAccess.accountPoolHealth.dialog.cooldown", "冷却中");
      case "auth":
        return t("codex.apiService.accountHealth.authError", "鉴权异常");
      case "quota":
        return t("codex.localAccess.accountPoolHealth.dialog.quota", "额度受限");
      default:
        return t("codex.apiService.accountHealth.unavailable", "暂不可用");
    }
  };

  const issueDetails = (issue: HealthIssue): string => {
    if (issue.kind === "missing") {
      return t(
        "codex.localAccess.accountPoolHealth.dialog.missingDetail",
        "账号已不在当前账号列表中",
      );
    }
    if (issue.kind === "cooldown" && issue.health) {
      return issue.health.cooldowns
        .map((cooldown) => formatCooldown(cooldown, t))
        .join(" · ");
    }
    if (issue.kind === "auth") {
      return t(
        "codex.localAccess.accountPoolHealth.dialog.authDetail",
        "OAuth 授权可能已失效，请重新授权后再试",
      );
    }
    if (issue.kind === "quota") {
      return t(
        "codex.localAccess.accountPoolHealth.dialog.quotaDetail",
        "账号额度暂时不可用，请等待额度恢复或检查套餐状态",
      );
    }
    switch (issue.health?.schedulerReason) {
      case "payment_required":
        return t(
          "codex.localAccess.accountPoolHealth.dialog.paymentDetail",
          "账号套餐或付款状态不可用，请检查订阅状态",
        );
      case "not_found":
      case "model_not_supported":
        return t(
          "codex.localAccess.accountPoolHealth.dialog.modelDetail",
          "当前账号不支持请求的模型",
        );
      case "transient_upstream":
        return t(
          "codex.localAccess.accountPoolHealth.dialog.upstreamDetail",
          "上游服务暂时异常，恢复后会重新尝试",
        );
      case "disabled":
        return t(
          "codex.localAccess.accountPoolHealth.dialog.disabledDetail",
          "该账号已被停用，请先启用账号",
        );
    }
    return t(
      "codex.localAccess.accountPoolHealth.dialog.unavailableDetail",
      "Sidecar 当前未将该账号列为可调度账号",
    );
  };

  const poolIssueTitle = (health: CodexLocalAccessAccountPoolHealth): string =>
    health.apiKeyLabel.trim() ||
    health.apiKeyId.trim() ||
    t(
      "codex.localAccess.accountPoolHealth.dialog.unscopedApiKey",
      "当前 API Key",
    );
  const poolIssueDetail = (health: CodexLocalAccessAccountPoolHealth): string => {
    const model = health.model.trim() || t("common.unknown", "未知");
    if (!health.diagnosticAvailable) {
      return t(
        "codex.localAccess.accountPoolHealth.dialog.poolUnavailableDetail",
        {
          model,
          defaultValue: "模型 {{model}} 的请求没有选出可用账号",
        },
      );
    }
    return t(
      "codex.localAccess.accountPoolHealth.dialog.poolDiagnosticDetail",
      {
        model,
        candidate: health.candidateAuths,
        scoped: health.scopedAuths,
        available: health.availableAuths,
        unavailable: health.unavailableAuths,
        modelExcluded: health.modelExcludedAuths,
        quotaReserved: health.quotaReservedAuths,
        imageBlocked: health.imagePolicyBlockedAuths,
        defaultValue:
          "模型 {{model}}：候选 {{candidate}}，范围匹配 {{scoped}}，可用 {{available}}，不可用 {{unavailable}}，模型排除 {{modelExcluded}}，额度保留 {{quotaReserved}}，生图策略拦截 {{imageBlocked}}",
      },
    );
  };

  const recoverableAccountIds = issues
    .filter(
      (issue) =>
        issue.kind !== "missing" &&
        issue.kind !== "quota" &&
        issue.health?.schedulerReason !== "disabled",
    )
    .map((issue) => issue.accountId);
  const isRecoverable = (issue: HealthIssue) =>
    recoverableAccountIds.includes(issue.accountId);
  const poolRecoveryAccountIds = accountIds.filter((accountId) =>
    accounts.some((account) => account.id === accountId),
  );
  const hasPoolIssues = accountPoolHealth.length > 0;
  const runRecovery = async (accountIds: string[]) => {
    setRecoveryError(null);
    try {
      if (accountIds.length === 1) {
        await onRecover(accountIds[0]);
      } else {
        await onRecoverAll(accountIds);
      }
    } catch (error) {
      setRecoveryError(String(error).replace(/^Error:\s*/, ""));
    }
  };
  const handleClose = () => {
    setRecoveryError(null);
    onClose();
  };

  return (
    <div className="modal-overlay codex-account-pool-health-overlay">
      <div
        className="modal codex-account-pool-health-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="codex-account-pool-health-title"
      >
        <div className="modal-header codex-account-pool-health-header">
          <div>
            <div className="codex-account-pool-health-title-row">
              <CircleAlert size={18} />
              <h3 id="codex-account-pool-health-title">
                {t(
                  "codex.localAccess.accountPoolHealth.dialog.title",
                  "异常账号",
                )}
              </h3>
            </div>
            <p>
              {t(
                "codex.localAccess.accountPoolHealth.dialog.description",
                "以下状态来自 Sidecar 账号调度器。恢复操作会清除调度冷却并重新尝试账号。",
              )}
            </p>
          </div>
          <button
            type="button"
            className="modal-close"
            onClick={handleClose}
            aria-label={t("common.close", "关闭")}
          >
            <X size={18} />
          </button>
        </div>

        <div className="modal-body codex-account-pool-health-body">
          <ModalErrorMessage
            message={recoveryError}
            scrollKey={recoveryErrorScrollKey}
          />
          {!hasPoolIssues && issues.length === 0 ? (
            <div className="codex-account-pool-health-empty">
              <ShieldCheck size={24} />
              <span>
                {t(
                  "codex.localAccess.accountPoolHealth.dialog.noIssues",
                  "当前没有异常账号",
                )}
              </span>
            </div>
          ) : (
            <div className="codex-account-pool-health-list">
              {accountPoolHealth.map((health) => (
                <div
                  className="codex-account-pool-health-item is-pool"
                  key={health.apiKeyId || "__unscoped__"}
                >
                  <div className="codex-account-pool-health-item-primary">
                    <div className="codex-account-pool-health-item-identity">
                      <strong title={poolIssueTitle(health)}>
                        {poolIssueTitle(health)}
                      </strong>
                      <span className="codex-account-pool-health-item-status">
                        {t(
                          "codex.localAccess.accountPoolHealth.dialog.poolUnavailable",
                          "账号池无可用账号",
                        )}
                      </span>
                    </div>
                  </div>
                  <p className="codex-account-pool-health-item-detail">
                    {poolIssueDetail(health)}
                  </p>
                  {health.errorMessage.trim() && (
                    <code className="codex-account-pool-health-pool-error">
                      {health.errorCode
                        ? `${health.errorCode}: ${health.errorMessage}`
                        : health.errorMessage}
                    </code>
                  )}
                  {(health.accountStatuses ?? []).length > 0 && (
                    <div className="codex-account-pool-health-members">
                      {(health.accountStatuses ?? []).map((member) => {
                        const account = accounts.find((item) => item.id === member.accountId);
                        const rawName = resolveIssueDisplayName(
                          account,
                          null,
                          member.accountEmail || member.accountId,
                        );
                        const displayName = maskAccountText
                          ? maskAccountText(rawName)
                          : rawName;
                        return (
                          <div
                            className={`codex-account-pool-health-member ${member.available ? "is-available" : "is-unavailable"}`}
                            key={`${health.apiKeyId}:${member.accountId}`}
                          >
                            <div className="codex-account-pool-health-member-primary">
                              <strong title={displayName}>{displayName}</strong>
                              <span className="codex-account-pool-health-item-status">
                                {member.available
                                  ? t("codex.apiService.health.availableAccounts", "可用")
                                  : t("codex.apiService.accountHealth.unavailable", "不可用")}
                              </span>
                              {member.reasonCode.trim() && (
                                <code>{member.reasonCode}</code>
                              )}
                            </div>
                            {member.reasonMessage.trim() && (
                              <p className="codex-account-pool-health-member-detail">
                                {member.reasonMessage}
                              </p>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              ))}
              {issues.map((issue) => (
                <div
                  className={`codex-account-pool-health-item is-${issue.kind}`}
                  key={issue.accountId}
                >
                  <div className="codex-account-pool-health-item-primary">
                    <div className="codex-account-pool-health-item-identity">
                      <strong title={issue.displayName}>{issue.displayName}</strong>
                      {issue.planLabel && (
                        <span
                          className={`tier-badge ${issue.planClass || "unknown"}`}
                        >
                          {issue.planLabel}
                        </span>
                      )}
                      {issue.quotaItems.map((item) => (
                        <span
                          key={item.key}
                          className={`codex-account-pool-health-quota-pill quota-${item.quotaClass}`}
                          title={`${item.label} ${item.valueText}`}
                        >
                          <span className="codex-account-pool-health-quota-label">
                            {item.label}
                          </span>
                          <span className="codex-account-pool-health-quota-value">
                            {item.valueText}
                          </span>
                        </span>
                      ))}
                      <span className="codex-account-pool-health-item-status">
                        {issueLabel(issue.kind)}
                      </span>
                    </div>
                    {isRecoverable(issue) && (
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => void runRecovery([issue.accountId])}
                        disabled={actionBusy}
                      >
                        <RefreshCw size={14} />
                        {t(
                          "codex.localAccess.accountPoolHealth.dialog.recover",
                          "恢复",
                        )}
                      </button>
                    )}
                  </div>
                  <p className="codex-account-pool-health-item-detail">
                    {issueDetails(issue)}
                  </p>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="modal-footer codex-account-pool-health-footer">
          <button
            type="button"
            className="btn btn-secondary"
            onClick={handleClose}
          >
            {t("common.close", "关闭")}
          </button>
          {(recoverableAccountIds.length > 0 ||
            (hasPoolIssues && poolRecoveryAccountIds.length > 0)) && (
            <button
              type="button"
              className="btn btn-primary"
              onClick={() =>
                void runRecovery(
                  hasPoolIssues ? poolRecoveryAccountIds : recoverableAccountIds,
                )
              }
              disabled={actionBusy}
            >
              <RefreshCw size={15} />
              {actionBusy
                ? t(
                    "codex.localAccess.accountPoolHealth.dialog.recovering",
                    "恢复中…",
                  )
                : hasPoolIssues
                  ? t(
                      "codex.localAccess.accountPoolHealth.dialog.resyncPool",
                      "重新同步账号池",
                    )
                  : t(
                      "codex.localAccess.accountPoolHealth.dialog.recoverAll",
                      "全部恢复",
                    )}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
