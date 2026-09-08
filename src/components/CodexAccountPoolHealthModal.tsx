import { useMemo, useRef, useState } from "react";
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

function hasQuotaCooldown(
  health: CodexLocalAccessAccountHealth | null,
): boolean {
  return Boolean(
    health?.cooldowns.some((cooldown) =>
      cooldown.reason.trim().toLowerCase().includes("quota"),
    ) || health?.schedulerReason?.trim().toLowerCase().includes("quota"),
  );
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

type PoolMemberStatus =
  CodexLocalAccessAccountPoolHealth["accountStatuses"][number];

function poolMemberIssueKind(member: PoolMemberStatus): HealthIssueKind {
  if (member.available) return "unavailable";
  const code = member.reasonCode.trim().toLowerCase();
  if (code.includes("cooldown")) return "cooldown";
  if (
    code.includes("auth") ||
    code.includes("token") ||
    code.includes("unauthorized")
  ) {
    return "auth";
  }
  if (code.includes("quota")) return "quota";
  if (code === "missing") return "missing";
  return "unavailable";
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
  const [recoveringAccountIds, setRecoveringAccountIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [recoveringAll, setRecoveringAll] = useState(false);
  const [recoverySuccess, setRecoverySuccess] = useState<string | null>(null);
  const recoveryInFlightRef = useRef(false);
  const poolMemberStatuses = useMemo(() => {
    const healthById = new Map(
      accountHealth.map((health) => [health.accountId, health]),
    );
    const accountById = new Map(accounts.map((account) => [account.id, account]));
    return accountPoolHealth.map((pool) => {
      const reported = (pool.accountStatuses ?? []).filter((member) =>
        member.accountId.trim(),
      );
      if (reported.length > 0) {
        return reported;
      }
      // Older sidecars did not send member diagnostics. Keep the account-level
      // view useful by synthesizing one row per selected account. Do not require
      // the account list to have finished loading: the account ID is enough to
      // show the failure and invoke recovery.
      return accountIds
        .map((accountId) => {
          const health = healthById.get(accountId);
          const model = pool.model.trim().toLowerCase();
          const cooldown = health?.cooldowns.find(
            (item) => !model || item.modelId.trim().toLowerCase() === model,
          );
          return {
            accountId,
            accountEmail: accountById.get(accountId)?.email ?? "",
            available: false,
            reasonCode: cooldown?.modelId
              ? "model_cooldown"
              : health?.schedulerReason || "pool_unavailable",
            reasonMessage: cooldown
              ? formatCooldown(cooldown, t)
              : t(
                  "codex.localAccess.accountPoolHealth.dialog.poolUnavailableDetail",
                  {
                    model: pool.model.trim() || t("common.unknown", "未知模型"),
                    defaultValue: "模型 {{model}} 的请求没有选出可用账号",
                  },
                ),
          };
        });
    });
  }, [accountHealth, accountIds, accountPoolHealth, accounts, t]);
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
      if (hasQuotaCooldown(issue.health)) {
        return t(
          "codex.localAccess.accountPoolHealth.dialog.quotaDetail",
          "账号额度暂时不可用，请等待额度恢复或检查套餐状态",
        );
      }
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

  const recoverableAccountIds = issues
    .filter(
      (issue) =>
        issue.kind !== "missing" &&
        issue.health?.schedulerReason !== "disabled",
    )
    .map((issue) => issue.accountId);
  const isRecoverable = (issue: HealthIssue) =>
    recoverableAccountIds.includes(issue.accountId);
  const poolMemberAccountIds = new Set(
    poolMemberStatuses.flatMap((members) =>
      members
        .map((member) => member.accountId.trim())
        .filter(Boolean),
    ),
  );
  const visibleAccountIssues = issues.filter(
    (issue) => !poolMemberAccountIds.has(issue.accountId),
  );
  const isPoolMemberRecoverable = (
    member: CodexLocalAccessAccountPoolHealth["accountStatuses"][number],
  ): boolean => {
    const code = member.reasonCode.trim().toLowerCase();
    return (
      Boolean(member.accountId.trim()) &&
      !member.available &&
      code !== "disabled" &&
      code !== "missing" &&
      code !== "quota_reserved" &&
      !["model_excluded", "model_disabled", "image_policy_blocked"].includes(
        code,
      )
    );
  };
  const poolMemberRecoveryIds = Array.from(
    new Set(
      poolMemberStatuses.flatMap((members) =>
        members
          .filter(isPoolMemberRecoverable)
          .map((member) => member.accountId.trim())
          .filter(Boolean),
      ),
    ),
  );
  const runRecovery = async (
    accountIds: string[],
    source: "single" | "all",
  ) => {
    const normalizedAccountIds = Array.from(
      new Set(accountIds.map((accountId) => accountId.trim()).filter(Boolean)),
    );
    if (normalizedAccountIds.length === 0 || recoveryInFlightRef.current) {
      return;
    }
    recoveryInFlightRef.current = true;
    setRecoveryError(null);
    setRecoverySuccess(null);
    setRecoveringAccountIds(new Set(normalizedAccountIds));
    setRecoveringAll(source === "all");
    try {
      if (normalizedAccountIds.length === 1) {
        await onRecover(normalizedAccountIds[0]);
      } else {
        await onRecoverAll(normalizedAccountIds);
      }
      setRecoverySuccess(
        t("codex.localAccess.accountPoolHealth.recoverSuccess", {
          count: normalizedAccountIds.length,
          defaultValue: "已提交 {{count}} 个账号的恢复操作",
        }),
      );
    } catch (error) {
      setRecoveryError(String(error).replace(/^Error:\s*/, ""));
    } finally {
      recoveryInFlightRef.current = false;
      setRecoveringAccountIds(new Set());
      setRecoveringAll(false);
    }
  };
  const handleClose = () => {
    setRecoveryError(null);
    setRecoverySuccess(null);
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
                  "账号状态",
                )}
              </h3>
            </div>
            <p>
              {t(
                "codex.localAccess.accountPoolHealth.dialog.description",
                "以下状态按账号显示，包含对应模型、不可用原因和可执行的恢复操作。",
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
          {recoverySuccess && (
            <div
              className="codex-account-pool-health-success"
              role="status"
              aria-live="polite"
            >
              <ShieldCheck size={15} />
              <span>{recoverySuccess}</span>
            </div>
          )}
          {poolMemberAccountIds.size === 0 && issues.length === 0 ? (
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
              {accountPoolHealth.flatMap((health, healthIndex) =>
                (poolMemberStatuses[healthIndex] ?? []).map((member) => {
                  const account = accounts.find(
                    (item) => item.id === member.accountId,
                  );
                  const rawName = resolveIssueDisplayName(
                    account,
                    null,
                    member.accountEmail || member.accountId,
                  );
                  const displayName = maskAccountText
                    ? maskAccountText(rawName)
                    : rawName;
                  const presentation = account
                    ? buildCodexAccountPresentation(account, t)
                    : null;
                  const recoverable = isPoolMemberRecoverable(member);
                  const recovering = recoveringAccountIds.has(member.accountId);
                  const memberKind = poolMemberIssueKind(member);
                  return (
                    <div
                      className={`codex-account-pool-health-item is-${member.available ? "available" : memberKind}`}
                      key={`${healthIndex}:${health.apiKeyId || "__unscoped__"}:${health.model}:${member.accountId}`}
                    >
                      <div className="codex-account-pool-health-item-primary">
                        <div className="codex-account-pool-health-item-identity">
                          <strong title={displayName}>{displayName}</strong>
                          {presentation?.planLabel && (
                            <span
                              className={`tier-badge ${presentation.planClass || "unknown"}`}
                            >
                              {presentation.planLabel}
                            </span>
                          )}
                          {health.model.trim() && (
                            <span className="codex-account-pool-health-model-pill">
                              {health.model.trim()}
                            </span>
                          )}
                          <span className="codex-account-pool-health-item-status">
                            {member.available
                              ? t("codex.apiService.health.availableAccounts", "可用")
                              : issueLabel(memberKind)}
                          </span>
                          {member.reasonCode.trim() && (
                            <code>{member.reasonCode}</code>
                          )}
                        </div>
                        {recoverable && (
                          <button
                            type="button"
                            className="btn btn-secondary btn-sm"
                            onClick={() =>
                              void runRecovery([member.accountId], "single")
                            }
                            disabled={actionBusy || recoveringAccountIds.size > 0}
                          >
                            <RefreshCw
                              size={13}
                              className={recovering ? "loading-spinner" : undefined}
                            />
                            {recovering
                              ? t(
                                  "codex.localAccess.accountPoolHealth.dialog.recovering",
                                  "恢复中…",
                                )
                              : t(
                                  "codex.localAccess.accountPoolHealth.dialog.recover",
                                  "恢复",
                                )}
                          </button>
                        )}
                      </div>
                      {member.reasonMessage.trim() && (
                        <p className="codex-account-pool-health-item-detail">
                          {member.reasonMessage}
                        </p>
                      )}
                    </div>
                  );
                }),
              )}
              {visibleAccountIssues.map((issue) => (
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
                        onClick={() =>
                          void runRecovery([issue.accountId], "single")
                        }
                        disabled={actionBusy || recoveringAccountIds.size > 0}
                      >
                        <RefreshCw
                          size={14}
                          className={
                            recoveringAccountIds.has(issue.accountId)
                              ? "loading-spinner"
                              : undefined
                          }
                        />
                        {recoveringAccountIds.has(issue.accountId)
                          ? t(
                              "codex.localAccess.accountPoolHealth.dialog.recovering",
                              "恢复中…",
                            )
                          : t(
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
            poolMemberRecoveryIds.length > 0) && (
            <button
              type="button"
              className="btn btn-primary"
              onClick={() =>
                void runRecovery(
                  Array.from(
                    new Set([...recoverableAccountIds, ...poolMemberRecoveryIds]),
                  ),
                  "all",
                )
              }
              disabled={actionBusy || recoveringAccountIds.size > 0}
            >
              <RefreshCw
                size={15}
                className={recoveringAll ? "loading-spinner" : undefined}
              />
              {recoveringAll
                ? t(
                    "codex.localAccess.accountPoolHealth.dialog.recovering",
                    "恢复中…",
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
