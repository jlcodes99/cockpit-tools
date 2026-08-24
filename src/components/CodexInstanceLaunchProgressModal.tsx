import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { AlertTriangle, Check, Circle, Info, Minus, RefreshCw, X } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import * as codexInstanceService from "../services/codexInstanceService";
import * as codexLocalAccessService from "../services/codexLocalAccessService";
import { useCodexAccountStore } from "../stores/useCodexAccountStore";
import { useCodexInstanceStore } from "../stores/useCodexInstanceStore";
import type {
  CodexInstanceAccountConflict,
  CodexInstanceRuntimeOwner,
} from "../utils/codexInstanceLaunchConflict";
import { conciseCodexCredentialFailure } from "../utils/codexCredentialProgress";
import { requestCodexOpenAddAccount } from "../utils/codexAddAccountRequest";
import type { CodexSwitchAuthFailure } from "../utils/codexSwitchAuthFailure";
import { parseCodexSwitchAuthFailure } from "../utils/codexSwitchAuthFailure";
import { presentWindowsOperationError } from "../utils/windowsOperationDialog";
import { parseWindowsOperationError } from "../utils/windowsOperationError";
import "./CodexSwitchProgressModal.css";
import "./CodexInstanceLaunchProgressModal.css";

type LaunchStepId =
  | "checkInstance"
  | "checkAccount"
  | "checkOccupancy"
  | "stopPrevious"
  | "prepareCredentials"
  | "writeProfile"
  | "startClient";
type LaunchStepStatus =
  | "pending"
  | "running"
  | "completed"
  | "warning"
  | "skipped"
  | "error";
type LaunchStatus = "running" | "conflict" | "completed" | "error";

interface LaunchProgressPayload {
  type?: "start" | "conflict" | "complete" | "error";
  instanceId?: string;
  instanceName?: string;
  isDefault?: boolean;
  progress?: number;
  step?: LaunchStepId;
  stepStatus?: LaunchStepStatus;
  details?: Record<string, unknown>;
  conflict?: CodexInstanceAccountConflict;
  error?: string;
  authFailure?: CodexSwitchAuthFailure | null;
}

interface LaunchStepState {
  id: LaunchStepId;
  status: LaunchStepStatus;
  details: Record<string, unknown>;
}

interface LaunchProgressState {
  instanceId: string;
  instanceName: string;
  isDefault: boolean;
  progress: number;
  status: LaunchStatus;
  steps: LaunchStepState[];
  conflict?: CodexInstanceAccountConflict;
  error?: string;
  authFailure?: CodexSwitchAuthFailure | null;
}

const STEP_IDS: LaunchStepId[] = [
  "checkInstance",
  "checkAccount",
  "checkOccupancy",
  "stopPrevious",
  "prepareCredentials",
  "writeProfile",
  "startClient",
];

function createState(payload: LaunchProgressPayload): LaunchProgressState {
  return {
    instanceId: payload.instanceId || "",
    instanceName: payload.instanceName || "",
    isDefault: payload.isDefault === true,
    progress: payload.progress ?? 2,
    status: "running",
    steps: STEP_IDS.map((id) => ({ id, status: "pending", details: {} })),
  };
}

function optionalTimestamp(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function optionalBoolean(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}

export function CodexInstanceLaunchProgressModal() {
  const { t, i18n } = useTranslation();
  const [state, setState] = useState<LaunchProgressState | null>(null);
  const [actionBusy, setActionBusy] = useState<"locate" | "transfer" | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [apiActionBusy, setApiActionBusy] = useState(false);
  const accounts = useCodexAccountStore((store) => store.accounts);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<LaunchProgressPayload>(
      "codex:instance-launch-progress",
      (event) => {
        if (disposed || !event.payload.instanceId) return;
        const payload = event.payload;
        setState((previous) => {
          if (payload.type === "start") {
            setActionError(null);
            return createState(payload);
          }
          const base: LaunchProgressState =
            previous && previous.instanceId === payload.instanceId
              ? previous
              : createState(payload);
          const steps = payload.step
            ? base.steps.map((step) =>
                step.id === payload.step
                  ? {
                      ...step,
                      status: payload.stepStatus || "running",
                      details: payload.details || step.details,
                    }
                  : step,
              )
            : base.steps;
          if (payload.type === "conflict") {
            return {
              ...base,
              progress: payload.progress ?? base.progress,
              status: "conflict",
              steps,
              conflict: payload.conflict,
            };
          }
          if (payload.type === "complete") {
            return {
              ...base,
              progress: 100,
              status: "completed",
              steps,
            };
          }
          if (payload.type === "error") {
            const authFailure =
              payload.authFailure ?? parseCodexSwitchAuthFailure(payload.error);
            const markedSteps = steps.map((step) =>
              step.status === "running"
                ? {
                    ...step,
                    status: "error" as const,
                    details: {
                      ...step.details,
                      error: payload.error || t("common.failed", "失败"),
                    },
                  }
                : step,
            );
            return {
              ...base,
              status: "error",
              steps: markedSteps,
              error: payload.error || t("common.failed", "失败"),
              authFailure,
            };
          }
          return {
            ...base,
            progress: payload.progress ?? base.progress,
            status: "running",
            steps,
          };
        });
      },
    ).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [t]);

  useEffect(() => {
    if (state?.status !== "completed") return;
    setActionBusy(null);
    void useCodexInstanceStore.getState().refreshInstances();
    const timer = window.setTimeout(() => setState(null), 1600);
    return () => window.clearTimeout(timer);
  }, [state?.status]);

  const relativeTime = useMemo(
    () =>
      new Intl.RelativeTimeFormat(i18n.resolvedLanguage || i18n.language, {
        numeric: "auto",
      }),
    [i18n.language, i18n.resolvedLanguage],
  );

  if (!state) return null;

  const instanceLabel = state.isDefault
    ? t("instances.defaultName", "默认实例")
    : state.instanceName || state.instanceId;
  const formatExpiry = (expiresAt: number | null) => {
    if (expiresAt === null) {
      return t("codex.switchProgress.detail.expiryUnknown");
    }
    const diffSeconds = expiresAt - Math.floor(Date.now() / 1000);
    const absoluteSeconds = Math.abs(diffSeconds);
    let unit: Intl.RelativeTimeFormatUnit = "minute";
    let divisor = 60;
    if (absoluteSeconds >= 36 * 60 * 60) {
      unit = "day";
      divisor = 24 * 60 * 60;
    } else if (absoluteSeconds >= 90 * 60) {
      unit = "hour";
      divisor = 60 * 60;
    }
    const relativeValue =
      diffSeconds < 0
        ? Math.floor(diffSeconds / divisor)
        : Math.max(1, Math.ceil(diffSeconds / divisor));
    return t("codex.switchProgress.detail.expiresAt", {
      time: new Date(expiresAt * 1000).toLocaleString(),
      relative: relativeTime.format(relativeValue, unit),
    });
  };
  const ownerLabel = (owner: CodexInstanceRuntimeOwner) =>
    owner.isDefault
      ? t("instances.defaultName", "默认实例")
      : owner.instanceName || owner.instanceId;
  const authFailure = state.status === "error" ? state.authFailure : null;
  const windowsOperationError = state.error
    ? parseWindowsOperationError(state.error)
    : null;
  const isApiOnlyAuthFailure = authFailure?.apiOnlyAvailable === true;
  const accountId =
    authFailure?.accountId ||
    (typeof state.steps.find((step) => step.id === "checkAccount")?.details
      .accountId === "string"
      ? String(
          state.steps.find((step) => step.id === "checkAccount")?.details
            .accountId,
        )
      : null);
  const account = accountId
    ? accounts.find((item) => item.id === accountId)
    : null;
  const accountLabel = account?.account_name || account?.email || accountId;
  const authReason = authFailure
    ? authFailure.reasonCode === "refresh_token_reused"
      ? t("codex.authError.refreshTokenReused")
      : authFailure.reasonCode === "refresh_token_expired"
        ? t("codex.authError.refreshTokenExpired")
        : authFailure.reasonCode === "refresh_token_invalidated"
          ? t("codex.authError.refreshTokenInvalidated")
          : authFailure.reasonCode === "id_token_unavailable"
            ? t("codex.switchAuth.idTokenUnavailable")
            : t("codex.authError.invalidGrant")
    : "";
  const authTechnicalReason = authFailure
    ? conciseCodexCredentialFailure(authFailure.message)
    : "";
  const stepDetails = (step: LaunchStepState): string[] => {
    if (step.status === "pending") {
      return [t("codex.switchProgress.detail.waiting")];
    }
    switch (step.id) {
      case "checkInstance":
        return step.details.userDataDir
          ? [String(step.details.userDataDir)]
          : [t("instances.accountLease.detail.checkingInstance")];
      case "checkAccount": {
        if (step.status === "skipped") {
          return [t("instances.accountLease.detail.noOAuthAccount")];
        }
        const lines = step.details.accountEmail
          ? [String(step.details.accountEmail)]
          : [t("instances.accountLease.detail.checkingAccount")];
        const accessExpiry = optionalTimestamp(step.details.accessTokenExpiresAt);
        const idExpiry = optionalTimestamp(step.details.idTokenExpiresAt);
        if (accessExpiry !== null) {
          lines.push(`access_token：${formatExpiry(accessExpiry)}`);
        }
        if (idExpiry !== null) {
          lines.push(`id_token：${formatExpiry(idExpiry)}`);
        }
        if (optionalBoolean(step.details.accessTokenRefreshDue) === true) {
          lines.push(
            `access_token：${t("codex.switchProgress.detail.refreshNeeded")}`,
          );
        }
        if (optionalBoolean(step.details.idTokenRefreshDue) === true) {
          lines.push(
            `id_token：${t("codex.switchProgress.detail.refreshNeeded")}`,
          );
        }
        if (optionalBoolean(step.details.knownRefreshFailure) === true) {
          lines.push(t("codex.switchProgress.detail.refreshFailed"));
          const failure = conciseCodexCredentialFailure(
            step.details.refreshFailure,
          );
          if (failure) {
            lines.push(`${t("codex.switchAuth.reasonLabel")}：${failure}`);
          }
        }
        return lines;
      }
      case "checkOccupancy":
        return step.status === "warning"
          ? [t("instances.accountLease.detail.accountInUse")]
          : [t("instances.accountLease.detail.accountAvailable")];
      case "stopPrevious": {
        const owners = Array.isArray(step.details.owners)
          ? (step.details.owners as CodexInstanceRuntimeOwner[])
          : [];
        if (step.status === "skipped") {
          return [t("instances.accountLease.detail.transferSkipped")];
        }
        return owners.length
          ? [
              t("instances.accountLease.detail.stoppingOwners", {
                owners: owners.map(ownerLabel).join(" · "),
              }),
            ]
          : [t("instances.accountLease.detail.stoppingPrevious")];
      }
      case "prepareCredentials": {
        const refreshRequired =
          optionalBoolean(step.details.refreshRequired) === true;
        const knownRefreshFailure =
          optionalBoolean(step.details.knownRefreshFailure) === true;
        const failure = conciseCodexCredentialFailure(
          step.details.error || step.details.refreshFailure,
        );
        if (step.status === "error" || knownRefreshFailure) {
          return [
            t("codex.switchProgress.detail.refreshFailed"),
            ...(failure
              ? [`${t("codex.switchAuth.reasonLabel")}：${failure}`]
              : []),
          ];
        }
        if (step.status === "running") {
          return [
            refreshRequired
              ? t("codex.switchProgress.detail.refreshingTokens")
              : t("codex.switchProgress.detail.tokenValid"),
          ];
        }
        const lines = [
          refreshRequired
            ? optionalBoolean(step.details.tokenGenerationChanged) === true
              ? t("codex.switchProgress.detail.refreshCompleted")
              : t("codex.switchProgress.detail.refreshResultReused")
            : t("codex.switchProgress.detail.tokenValid"),
        ];
        const accessExpiry = optionalTimestamp(step.details.accessTokenExpiresAt);
        const idExpiry = optionalTimestamp(step.details.idTokenExpiresAt);
        if (accessExpiry !== null) {
          lines.push(`access_token：${formatExpiry(accessExpiry)}`);
        }
        if (idExpiry !== null) {
          lines.push(`id_token：${formatExpiry(idExpiry)}`);
        }
        return lines;
      }
      case "writeProfile":
        return [t("instances.accountLease.detail.profileReady")];
      case "startClient":
        return step.details.launchMode === "cli"
          ? [t("instances.accountLease.detail.cliPrepared")]
          : [t("instances.accountLease.detail.startingClient")];
    }
  };
  const statusIcon = (status: LaunchStepStatus) => {
    if (status === "running") return <RefreshCw size={13} className="loading-spinner" />;
    if (status === "completed") return <Check size={14} />;
    if (status === "warning") return <AlertTriangle size={13} />;
    if (status === "error") return <X size={13} />;
    if (status === "skipped") return <Minus size={13} />;
    return <Circle size={9} />;
  };
  const title = authFailure
    ? authFailure.apiOnlyAvailable
      ? t("codex.switchAuth.apiOnlyTitle")
      : t("codex.switchAuth.reauthorizeTitle")
    : state.status === "conflict"
      ? t("instances.accountLease.conflictTitle")
      : state.status === "completed"
        ? t("instances.accountLease.completedTitle")
        : state.status === "error"
          ? t("instances.accountLease.failedTitle")
          : t("instances.accountLease.title");

  const performLocateOwner = async () => {
    const owner = state.conflict?.owners[0];
    if (!owner) return;
    if (owner.managed) {
      await codexInstanceService.openInstanceWindow(owner.instanceId);
    } else {
      await invoke("codex_focus_runtime_owner", {
        pid: owner.pid,
        userDataDir: owner.userDataDir,
        isDefault: owner.isDefault,
      });
    }
  };
  const locateOwner = async () => {
    setActionBusy("locate");
    setActionError(null);
    try {
      await performLocateOwner();
    } catch (error) {
      if (
        presentWindowsOperationError({
          error,
          operation: "open_path",
          retry: performLocateOwner,
        })
      ) {
        return;
      }
      setActionError(String(error));
    } finally {
      setActionBusy(null);
    }
  };
  const performTransferAccount = async () => {
    const instance = await codexInstanceService.startInstance(state.instanceId, {
      transferConflictingAccount: true,
    });
    window.dispatchEvent(
      new CustomEvent("codex:instance-launch-transferred", {
        detail: { instance },
      }),
    );
  };
  const transferAccount = async () => {
    setActionBusy("transfer");
    setActionError(null);
    try {
      await performTransferAccount();
    } catch (error) {
      setActionBusy(null);
      if (
        presentWindowsOperationError({
          error,
          operation: "launch_app",
          summary: t("instances.accountLease.failedTitle"),
          retry: performTransferAccount,
          manualContinue: performTransferAccount,
        })
      ) {
        return;
      }
      setActionError(String(error).replace(/^Error:\s*/, ""));
    }
  };
  const selectOtherAccount = () => {
    const instanceId = state.instanceId;
    setState(null);
    window.dispatchEvent(
      new CustomEvent("codex:edit-instance-account", {
        detail: { instanceId },
      }),
    );
  };

  const reauthorizeAccount = () => {
    if (!accountId) return;
    setState(null);
    window.dispatchEvent(
      new CustomEvent("app-request-navigate", { detail: "codex" }),
    );
    requestCodexOpenAddAccount({
      tab: "oauth",
      targetAccountId: accountId,
      retryInstanceLaunchAfterOAuth: true,
      retryInstanceId: state.instanceId,
    });
  };

  const addAccountToApiService = async () => {
    if (!authFailure?.apiOnlyAvailable || !accountId || apiActionBusy) return;
    setApiActionBusy(true);
    setActionError(null);
    try {
      const result = await codexLocalAccessService.appendCodexLocalAccessAccounts([
        accountId,
      ]);
      if (!result.syncedAccountIds.includes(accountId)) {
        const skipped = result.skippedAccounts.find(
          (item) => item.accountId === accountId,
        );
        throw new Error(skipped?.reason || "not_available");
      }
      setState(null);
      window.dispatchEvent(
        new CustomEvent("app-request-navigate", { detail: "codex-api-service" }),
      );
    } catch (error) {
      setActionError(
        t("codex.switchAuth.apiServiceAddFailed", {
          error: String(error).replace(/^Error:\s*/, ""),
        }),
      );
    } finally {
      setApiActionBusy(false);
    }
  };

  return createPortal(
    <div className="modal-overlay codex-switch-progress-overlay">
      <div className="modal-content codex-switch-progress-modal codex-instance-launch-modal">
        <div className="codex-switch-progress-header">
          <div
            className={`codex-switch-progress-icon ${
              state.status === "error"
                ? isApiOnlyAuthFailure
                  ? "warning"
                  : "error"
                : state.status === "completed"
                  ? "completed"
                  : ""
            }`}
          >
            {state.status === "error" || state.status === "conflict" ? (
              state.status === "error" && !isApiOnlyAuthFailure ? (
                <X size={19} />
              ) : (
                <AlertTriangle size={19} />
              )
            ) : state.status === "completed" ? (
              <Check size={19} />
            ) : (
              <RefreshCw size={18} className="loading-spinner" />
            )}
          </div>
          <div className="codex-switch-progress-heading">
            <h2>{title}</h2>
            <p>{authFailure ? accountLabel : instanceLabel}</p>
          </div>
        </div>

        <div className="codex-switch-progress-overview">
          <div className="codex-switch-progress-stage-row">
            <span>{t("instances.accountLease.progress")}</span>
            <span>{Math.round(state.progress)}%</span>
          </div>
          <div className="codex-switch-progress-track">
            <div
              className={`codex-switch-progress-bar ${state.status === "error" ? (isApiOnlyAuthFailure ? "warning" : "error") : ""}`}
              style={{ width: `${state.progress}%` }}
            />
          </div>
        </div>

        <div className="codex-switch-progress-body">
          {state.status === "conflict" && state.conflict && (
            <div className="codex-instance-account-conflict">
              <Info size={16} />
              <div>
                <strong>
                  {t("instances.accountLease.conflictAccount", {
                    account: state.conflict.accountEmail,
                  })}
                </strong>
                <p>
                  {t("instances.accountLease.conflictDescription", {
                    owners: state.conflict.owners.map(ownerLabel).join(" · "),
                  })}
                </p>
              </div>
            </div>
          )}
          <div className="codex-switch-step-list">
            {state.steps.map((step) => (
              <div key={step.id} className={`codex-switch-step ${step.status}`}>
                <div className="codex-switch-step-rail">
                  <span className="codex-switch-step-icon">
                    {statusIcon(step.status)}
                  </span>
                </div>
                <div className="codex-switch-step-content">
                  <div className="codex-switch-step-title-row">
                    <strong>{t(`instances.accountLease.steps.${step.id}`)}</strong>
                    <span>{t(`codex.switchProgress.status.${step.status}`)}</span>
                  </div>
                  <div className="codex-switch-step-details">
                    {stepDetails(step).map((line, index) => (
                      <span key={`${step.id}-${index}`}>{line}</span>
                    ))}
                  </div>
                </div>
              </div>
            ))}
          </div>
          {authFailure && (
            <div className="codex-switch-auth-summary">
              <p>
                {authFailure.apiOnlyAvailable
                  ? t("codex.switchAuth.apiOnlyDescription")
                  : t("codex.switchAuth.reauthorizeDescription")}
              </p>
              <div className="codex-switch-auth-reason" role="alert">
                <strong>{t("codex.switchAuth.reasonLabel")}</strong>
                <span>{authReason}</span>
                {authTechnicalReason && (
                  <small className="codex-switch-auth-technical-reason">
                    {authTechnicalReason}
                  </small>
                )}
              </div>
              {authFailure.apiOnlyAvailable && authFailure.accessTokenExpiresAt && (
                <div className="codex-switch-auth-expiry">
                  {t("codex.switchAuth.accessTokenExpiry", {
                    time: new Date(
                      authFailure.accessTokenExpiresAt * 1000,
                    ).toLocaleString(),
                  })}
                </div>
              )}
              {actionError && (
                <div className="codex-switch-progress-error" role="alert">
                  {actionError}
                </div>
              )}
            </div>
          )}
          {!authFailure && (state.error || actionError) && (
            <div className="codex-switch-progress-error">
              {actionError || windowsOperationError?.originalReason || state.error}
            </div>
          )}
        </div>

        {(state.status === "conflict" || state.status === "error") && (
          <div className="codex-switch-progress-footer codex-instance-launch-footer">
            {authFailure ? (
              <>
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => setState(null)}
                  disabled={apiActionBusy}
                >
                  {t("common.cancel", "取消")}
                </button>
                {authFailure.apiOnlyAvailable && (
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={() => void addAccountToApiService()}
                    disabled={apiActionBusy}
                  >
                    {apiActionBusy
                      ? t("common.loading", "加载中...")
                      : t("codex.localAccess.entryAction", "添加至 API 服务")}
                  </button>
                )}
                <button
                  type="button"
                  className="btn btn-primary"
                  onClick={reauthorizeAccount}
                  disabled={apiActionBusy || !accountId}
                >
                  {t("common.reauthorize", "重新授权")}
                </button>
              </>
            ) : state.status === "conflict" ? (
              <>
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => void locateOwner()}
                  disabled={actionBusy !== null}
                >
                  {actionBusy === "locate" && (
                    <RefreshCw size={14} className="loading-spinner" />
                  )}
                  {t("instances.accountLease.locate")}
                </button>
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={selectOtherAccount}
                  disabled={actionBusy !== null}
                >
                  {t("instances.accountLease.selectOther")}
                </button>
                <button
                  type="button"
                  className="btn btn-primary"
                  onClick={() => void transferAccount()}
                  disabled={actionBusy !== null}
                >
                  {actionBusy === "transfer" && (
                    <RefreshCw size={14} className="loading-spinner" />
                  )}
                  {t("instances.accountLease.transfer")}
                </button>
              </>
            ) : (
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => setState(null)}
              >
                {t("common.close", "关闭")}
              </button>
            )}
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}
