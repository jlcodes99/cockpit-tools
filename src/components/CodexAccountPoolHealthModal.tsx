import { useEffect, useMemo, useRef, useState } from "react";
import { CircleAlert, RefreshCw, ShieldCheck, X } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
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
  /** 手动恢复后仍在抑制窗口内的账号，暂时不显示这些账号的异常行。 */
  recoverySuppressedAccountIds?: string[];
  actionBusy: boolean;
  maskAccountText?: (value: string) => string;
  onClose: () => void;
  onRecover: (accountId: string) => Promise<void>;
  onRecoverAll: (accountIds: string[]) => Promise<void>;
  /** 凭据类失败（auth_unavailable / 401）需要重新走官方登录，恢复调度状态无法修复。 */
  onReauthorize?: (accountId: string) => void;
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

function isRecoverableReason(reason: string | null | undefined): boolean {
  return ![
    "disabled",
    "missing",
    "quota_reserved",
    "model_excluded",
    "model_disabled",
    "model_not_supported",
    "model_not_available",
    "not_found",
    "image_policy_blocked",
    "pool_unavailable",
    "auth_not_found",
  ].includes(reason?.trim().toLowerCase() ?? "");
}

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
  recoverySuppressedAccountIds,
  actionBusy,
  maskAccountText,
  onClose,
  onRecover,
  onRecoverAll,
  onReauthorize,
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
  const submittedRecoveryAccountIdsRef = useRef<Set<string>>(new Set());
  const suppressedAccountIds = useMemo(
    () =>
      new Set(
        (recoverySuppressedAccountIds ?? [])
          .map((accountId) => accountId.trim())
          .filter(Boolean),
      ),
    [recoverySuppressedAccountIds],
  );
  // A selection failure without member diagnostics belongs to the pool. It
  // does not prove that any selected account supports or attempted the model.
  const poolMemberStatuses = useMemo(
    () =>
      accountPoolHealth.map((pool) =>
        (pool.accountStatuses ?? []).filter((member) => member.accountId.trim()),
      ),
    [accountPoolHealth],
  );
  const unattributedPoolIssues = accountPoolHealth.filter(
    (_, index) => poolMemberStatuses[index].length === 0,
  );
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
  // 恢复现在会立即返回、重活在后台继续跑；后台失败必须回到当前弹框内提示，
  // 不能只停留在日志或提示“已提交”。
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<{ accountIds?: string[]; status?: string; error?: string }>(
      "codex-local-access-recovery-result",
      (event) => {
        const payload = event.payload ?? {};
        const accountIds = Array.isArray(payload.accountIds)
          ? payload.accountIds.filter((accountId) => typeof accountId === "string")
          : [];
        const submitted = submittedRecoveryAccountIdsRef.current;
        const matched = accountIds.filter((accountId) => submitted.has(accountId));
        if (matched.length === 0) return;
        if (payload.status === "failed") {
          const errorText =
            typeof payload.error === "string" ? payload.error.trim() : "";
          setRecoveryError(
            t("messages.actionFailed", {
              action: t(
                "codex.localAccess.accountPoolHealth.recover",
                "恢复账号状态",
              ),
              error: errorText || t("common.failed", "失败"),
              defaultValue: "{{action}}失败：{{error}}",
            }),
          );
          return;
        }
        setRecoverySuccess(
          t("codex.localAccess.accountPoolHealth.recoverSuccess", {
            count: matched.length,
            defaultValue: "已提交 {{count}} 个账号的恢复操作",
          }),
        );
      },
    ).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [t, setRecoveryError, setRecoverySuccess]);

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
      case "model_not_available":
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

  const poolMemberAccountIds = new Set(
    poolMemberStatuses.flatMap((members) =>
      members
        .map((member) => member.accountId.trim())
        .filter(Boolean),
    ),
  );
  const visibleAccountIssues = issues.filter(
    (issue) =>
      !poolMemberAccountIds.has(issue.accountId) &&
      !suppressedAccountIds.has(issue.accountId),
  );
  const hasVisibleIssues =
    unattributedPoolIssues.length > 0 ||
    visibleAccountIssues.length > 0 ||
    poolMemberStatuses.some((members) =>
      members.some((member) => !suppressedAccountIds.has(member.accountId.trim())),
    );
  const recoverableAccountIds = visibleAccountIssues
    .filter(
      (issue) =>
        issue.kind !== "missing" &&
        !(issue.kind === "auth" && onReauthorize) &&
        isRecoverableReason(issue.health?.schedulerReason),
    )
    .map((issue) => issue.accountId);
  const isRecoverable = (issue: HealthIssue) =>
    recoverableAccountIds.includes(issue.accountId);
  const isPoolMemberRecoverable = (
    member: CodexLocalAccessAccountPoolHealth["accountStatuses"][number],
  ): boolean => {
    return (
      Boolean(member.accountId.trim()) &&
      !member.available &&
      !suppressedAccountIds.has(member.accountId.trim()) &&
      !(poolMemberIssueKind(member) === "auth" && onReauthorize) &&
      isRecoverableReason(member.reasonCode)
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
    for (const accountId of normalizedAccountIds) {
      submittedRecoveryAccountIdsRef.current.add(accountId);
    }
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
                "显示账号池与账号异常；只有明确归属账号的故障才提供相应操作。",
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
          {!hasVisibleIssues ? (
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
              {unattributedPoolIssues.map((health, index) => (
                <div
                  className="codex-account-pool-health-item is-unavailable"
                  key={`pool:${index}:${health.apiKeyId}:${health.model}`}
                >
                  <div className="codex-account-pool-health-item-primary">
                    <div className="codex-account-pool-health-item-identity">
                      <strong>
                        {t(
                          "codex.localAccess.accountPoolHealth.dialog.poolUnavailable",
                          "账号池无可用账号",
                        )}
                      </strong>
                      {health.model.trim() && (
                        <span className="codex-account-pool-health-model-pill">
                          {health.model.trim()}
                        </span>
                      )}
                      {health.errorCode.trim() && <code>{health.errorCode}</code>}
                    </div>
                  </div>
                  <p className="codex-account-pool-health-item-detail">
                    {health.apiKeyLabel.trim()
                      ? (maskAccountText ? maskAccountText(health.apiKeyLabel) : health.apiKeyLabel)
                      : t("codex.localAccess.accountPoolHealth.dialog.unscopedApiKey", "当前 API Key")}
                    {" · "}
                    {t("codex.localAccess.accountPoolHealth.dialog.poolUnavailableDetail", {
                      model: health.model.trim() || t("common.unknown", "未知模型"),
                      defaultValue: "模型 {{model}} 的请求没有选出可用账号",
                    })}
                  </p>
                  <p className="codex-account-pool-health-item-detail">
                    {t(
                      "codex.localAccess.accountPoolHealth.dialog.poolUnattributedDetail",
                      "未收到逐账号诊断，无法归属到具体账号。请检查此 API Key 的账号范围及模型配置。",
                    )}
                  </p>
                  {health.diagnosticAvailable && (
                    <p className="codex-account-pool-health-item-detail">
                      {t("codex.localAccess.accountPoolHealth.dialog.poolDiagnosticDetail", {
                        model: health.model.trim() || t("common.unknown", "未知模型"),
                        candidate: health.candidateAuths,
                        scoped: health.scopedAuths,
                        available: health.availableAuths,
                        unavailable: health.unavailableAuths,
                        modelExcluded: health.modelExcludedAuths,
                        quotaReserved: health.quotaReservedAuths,
                        imageBlocked: health.imagePolicyBlockedAuths,
                      })}
                    </p>
                  )}
                </div>
              ))}
              {accountPoolHealth.flatMap((health, healthIndex) =>
                (poolMemberStatuses[healthIndex] ?? [])
                  .filter(
                    (member) =>
                      !suppressedAccountIds.has(member.accountId.trim()),
                  )
                  .map((member) => {
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
                          {memberKind === "auth" && onReauthorize ? (
                            <button
                              type="button"
                              className="btn btn-secondary btn-sm"
                              onClick={() => onReauthorize(member.accountId)}
                              title={t(
                                "codex.localAccess.accountPoolHealth.dialog.reauthorizeHint",
                                "账号凭据已失效，需要重新走官方登录；仅恢复调度状态无法修复。",
                              )}
                            >
                              <RefreshCw size={13} />
                              {t("common.reauthorize", "重新授权")}
                            </button>
                          ) : recoverable ? (
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
                          ) : null}
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
                    {issue.kind === "auth" && onReauthorize ? (
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => onReauthorize(issue.accountId)}
                        title={t(
                          "codex.localAccess.accountPoolHealth.dialog.reauthorizeHint",
                          "账号凭据已失效，需要重新走官方登录；仅恢复调度状态无法修复。",
                        )}
                      >
                        <RefreshCw size={13} />
                        {t("common.reauthorize", "重新授权")}
                      </button>
                    ) : isRecoverable(issue) ? (
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
                    ) : null}
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
