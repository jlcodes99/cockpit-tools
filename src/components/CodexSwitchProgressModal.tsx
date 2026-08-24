import { useEffect, useMemo, useState } from "react";
import { AlertTriangle, Check, Circle, Minus, RefreshCw, X } from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
import { useCodexAccountStore } from "../stores/useCodexAccountStore";
import * as codexLocalAccessService from "../services/codexLocalAccessService";
import { requestCodexOpenAddAccount } from "../utils/codexAddAccountRequest";
import { conciseCodexCredentialFailure } from "../utils/codexCredentialProgress";
import type { CodexSwitchAuthFailure } from "../utils/codexSwitchAuthFailure";
import { parseWindowsOperationError } from "../utils/windowsOperationError";
import "./CodexSwitchProgressModal.css";

type SwitchStage = "preparing" | "credentials" | "writing" | "starting" | "completed";
type SwitchStatus = "running" | "completed" | "error";
type SwitchStepId =
  | "credentials"
  | "accessToken"
  | "idToken"
  | "stopRuntime"
  | "refreshTokens"
  | "writeCredentials"
  | "syncSettings"
  | "startClient";
type SwitchStepStatus =
  | "pending"
  | "running"
  | "completed"
  | "warning"
  | "skipped"
  | "error";
type SwitchStepDetails = Record<string, unknown>;

interface SwitchProgressPayload {
  type?: "start" | "error" | "complete";
  accountId?: string;
  stage?: SwitchStage;
  progress?: number;
  error?: string;
  authFailure?: CodexSwitchAuthFailure | null;
  step?: SwitchStepId;
  stepStatus?: SwitchStepStatus;
  details?: SwitchStepDetails;
}

interface SwitchStepState {
  id: SwitchStepId;
  status: SwitchStepStatus;
  details: SwitchStepDetails;
}

interface SwitchProgressState {
  accountId: string;
  progress: number;
  status: SwitchStatus;
  steps: SwitchStepState[];
  error?: string;
  authFailure?: CodexSwitchAuthFailure | null;
}

const EVENT_NAME = "codex-switch-progress";
const STEP_IDS: SwitchStepId[] = [
  "credentials",
  "accessToken",
  "idToken",
  "refreshTokens",
  "stopRuntime",
  "writeCredentials",
  "syncSettings",
  "startClient",
];

function createProgressState(accountId: string): SwitchProgressState {
  return {
    accountId,
    progress: 4,
    status: "running",
    steps: STEP_IDS.map((id) => ({ id, status: "pending", details: {} })),
  };
}

function normalizeProgress(value: unknown, fallback: number) {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.max(0, Math.min(100, value))
    : fallback;
}

function optionalBoolean(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}

function optionalTimestamp(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function CodexSwitchProgressModal() {
  const { t, i18n } = useTranslation();
  const [state, setState] = useState<SwitchProgressState | null>(null);
  const [actionBusy, setActionBusy] = useState<"api" | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const accounts = useCodexAccountStore((store) => store.accounts);

  useEffect(() => {
    const handleWindowEvent = (event: Event) => {
      const detail = (event as CustomEvent<SwitchProgressPayload>).detail;
      if (!detail?.accountId || typeof detail.accountId !== "string") return;
      const accountId = detail.accountId;
      const isEmptyBackendError =
        detail.type === "error" && !detail.error && detail.authFailure === undefined;
      if (!isEmptyBackendError) {
        setActionBusy(null);
        setActionError(null);
      }

      setState((previous) => {
        if (detail.type === "start") return createProgressState(accountId);
        const base =
          previous?.accountId === accountId ? previous : createProgressState(accountId);
        if (detail.type === "error") {
          const errorText =
            detail.error || base.error || t("common.failed", "失败");
          const steps = base.steps.map((step) => ({ ...step }));
          let targetIndex = steps.findIndex((step) => step.status === "running");
          if (targetIndex < 0) {
            targetIndex = steps.findIndex((step) => step.status === "error");
          }
          if (targetIndex < 0) {
            targetIndex = steps.findIndex((step) => step.status === "pending");
          }
          if (targetIndex >= 0) {
            const target = steps[targetIndex];
            steps[targetIndex] = {
              ...target,
              status: "error",
              details: {
                ...target.details,
                error: errorText,
              },
            };
          }
          return {
            ...base,
            accountId,
            steps,
            status: "error",
            error: errorText,
            authFailure:
              detail.authFailure === undefined ? base.authFailure : detail.authFailure,
          };
        }
        if (detail.type === "complete" || detail.stage === "completed") {
          return { ...base, accountId, progress: 100, status: "completed" };
        }
        const steps = detail.step
          ? base.steps.map((step) =>
              step.id === detail.step
                ? {
                    ...step,
                    status: detail.stepStatus || "running",
                    details: detail.details || step.details,
                  }
                : step,
            )
          : base.steps;
        return {
          ...base,
          accountId,
          steps,
          progress: normalizeProgress(detail.progress, base.progress),
          status: "running",
          error: undefined,
          authFailure: undefined,
        };
      });
    };

    window.addEventListener(EVENT_NAME, handleWindowEvent as EventListener);
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<SwitchProgressPayload>("codex:switch-progress", (event) => {
      if (!disposed) {
        window.dispatchEvent(new CustomEvent(EVENT_NAME, { detail: event.payload }));
      }
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });

    return () => {
      disposed = true;
      window.removeEventListener(EVENT_NAME, handleWindowEvent as EventListener);
      unlisten?.();
    };
  }, [t]);

  useEffect(() => {
    if (!state || state.status !== "completed") return;
    const timer = window.setTimeout(() => setState(null), 1600);
    return () => window.clearTimeout(timer);
  }, [state]);

  const relativeTime = useMemo(
    () =>
      new Intl.RelativeTimeFormat(i18n.resolvedLanguage || i18n.language, {
        numeric: "auto",
      }),
    [i18n.language, i18n.resolvedLanguage],
  );

  if (!state) return null;

  const account = accounts.find((item) => item.id === state.accountId);
  const accountLabel = account?.account_name || account?.email || state.accountId;
  const isError = state.status === "error";
  const windowsOperationError = isError
    ? parseWindowsOperationError(state.error)
    : null;
  const authFailure = isError ? state.authFailure : null;
  const isApiOnlyAuthFailure = authFailure?.apiOnlyAvailable === true;
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

  const formatExpiry = (
    expiresAt: number | null,
    present: boolean | null,
    opaque = false,
  ) => {
    if (present === false) return t("codex.switchProgress.detail.missing");
    if (expiresAt === null) {
      return opaque
        ? t("codex.switchProgress.detail.opaqueToken")
        : t("codex.switchProgress.detail.expiryUnknown");
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

  const stepLabel = (id: SwitchStepId) => t(`codex.switchProgress.steps.${id}`);

  const stepDetailLines = (step: SwitchStepState): string[] => {
    const { details, status } = step;
    switch (step.id) {
      case "credentials": {
        if (status === "running") {
          return [t("codex.switchProgress.detail.readingCredentials")];
        }
        if (details.accountKind !== "oauth") {
          return [t("codex.switchProgress.detail.nonOAuthAccount")];
        }
        const available = [
          optionalBoolean(details.hasAccessToken) === true ? "access_token" : null,
          optionalBoolean(details.hasIdToken) === true ? "id_token" : null,
          optionalBoolean(details.hasRefreshToken) === true ? "refresh_token" : null,
        ].filter(Boolean);
        return [
          t("codex.switchProgress.detail.credentialsFound", {
            tokens: available.length ? available.join(" · ") : t("common.none", "暂无"),
          }),
        ];
      }
      case "accessToken":
      case "idToken":
        if (status === "skipped") {
          return [t("codex.switchProgress.detail.notApplicable")];
        }
        return [
          formatExpiry(
            optionalTimestamp(details.expiresAt),
            optionalBoolean(details.present),
            details.opaque === true,
          ),
          details.refreshDue === true
            ? t("codex.switchProgress.detail.refreshNeeded")
            : t("codex.switchProgress.detail.tokenValid"),
        ];
      case "stopRuntime":
        return [
          status === "running"
            ? t("codex.switchProgress.detail.stoppingRuntime")
            : status === "completed"
              ? t("codex.switchProgress.detail.runtimeStopped")
              : t("codex.switchProgress.detail.waiting"),
        ];
      case "refreshTokens": {
        if (status === "pending") return [t("codex.switchProgress.detail.waiting")];
        if (status === "running") {
          return [t("codex.switchProgress.detail.refreshingTokens")];
        }
        if (status === "skipped") {
          return [t("codex.switchProgress.detail.refreshSkipped")];
        }
        if (status === "error") {
          const failure = conciseCodexCredentialFailure(
            details.error || state.error,
          );
          return [
            t("codex.switchProgress.detail.refreshFailed"),
            ...(failure
              ? [`${t("codex.switchAuth.reasonLabel")}：${failure}`]
              : []),
          ];
        }
        const lines = [
          details.tokenGenerationChanged === false
            ? t("codex.switchProgress.detail.refreshResultReused")
            : t("codex.switchProgress.detail.refreshCompleted"),
        ];
        const accessExpiry = optionalTimestamp(details.accessTokenExpiresAt);
        const idExpiry = optionalTimestamp(details.idTokenExpiresAt);
        if (accessExpiry !== null) {
          lines.push(`access_token：${formatExpiry(accessExpiry, true)}`);
        }
        if (idExpiry !== null) lines.push(`id_token：${formatExpiry(idExpiry, true)}`);
        return lines;
      }
      case "writeCredentials":
        return [
          status === "completed"
            ? t("codex.switchProgress.detail.credentialsWritten")
            : status === "running"
              ? t("codex.switchProgress.detail.writingCredentials")
              : t("codex.switchProgress.detail.waiting"),
        ];
      case "syncSettings":
        return [
          status === "completed"
            ? t("codex.switchProgress.detail.settingsSynced")
            : status === "running"
              ? t("codex.switchProgress.detail.syncingSettings")
              : t("codex.switchProgress.detail.waiting"),
        ];
      case "startClient":
        if (status === "skipped") {
          return [t("codex.switchProgress.detail.launchDisabled")];
        }
        if (status === "warning") {
          return [t("codex.switchProgress.detail.clientStartWarning")];
        }
        return [
          status === "completed"
            ? t("codex.switchProgress.detail.clientStarted")
            : status === "running"
              ? t("codex.switchProgress.detail.startingClient")
              : t("codex.switchProgress.detail.waiting"),
        ];
    }
  };

  const activeStep =
    state.steps.find((step) => step.status === "running") ||
    [...state.steps].reverse().find((step) => step.status === "error") ||
    [...state.steps]
      .reverse()
      .find((step) => step.status === "completed" || step.status === "warning");
  const statusLabel =
    state.status === "completed"
      ? t("codex.switchProgress.completed")
      : state.status === "error"
        ? t("codex.switchProgress.needsAttention")
        : activeStep
          ? stepLabel(activeStep.id)
          : t("common.loading", "加载中...");

  const handleReauthorize = () => {
    const accountId = state.accountId;
    setState(null);
    window.dispatchEvent(new CustomEvent("app-request-navigate", { detail: "codex" }));
    requestCodexOpenAddAccount({
      tab: "oauth",
      targetAccountId: accountId,
      retrySwitchAfterOAuth: true,
    });
  };

  const handleUseForApiService = async () => {
    if (!authFailure?.apiOnlyAvailable || actionBusy) return;
    setActionBusy("api");
    setActionError(null);
    try {
      const result = await codexLocalAccessService.appendCodexLocalAccessAccounts([
        state.accountId,
      ]);
      if (!result.syncedAccountIds.includes(state.accountId)) {
        const skipped = result.skippedAccounts.find(
          (item) => item.accountId === state.accountId,
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
      setActionBusy(null);
    }
  };

  const renderStepIcon = (step: SwitchStepState) => {
    if (step.status === "running") {
      return <RefreshCw size={14} className="loading-spinner" />;
    }
    if (step.status === "completed") return <Check size={14} />;
    if (step.status === "warning") return <AlertTriangle size={14} />;
    if (step.status === "error") return <X size={14} />;
    if (step.status === "skipped") return <Minus size={14} />;
    return <Circle size={10} />;
  };

  return (
    <div className="modal-overlay codex-switch-progress-overlay">
      <div
        className="modal codex-switch-progress-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="codex-switch-progress-title"
      >
        <div className="codex-switch-progress-header">
          <div
            className={`codex-switch-progress-icon ${isError ? (isApiOnlyAuthFailure ? "warning" : "error") : ""} ${state.status === "completed" ? "completed" : ""}`}
          >
            {isError ? (
              isApiOnlyAuthFailure ? <AlertTriangle size={20} /> : <X size={20} />
            ) : state.status === "completed" ? (
              <Check size={20} />
            ) : (
              <RefreshCw size={20} className="loading-spinner" />
            )}
          </div>
          <div className="codex-switch-progress-heading">
            <h2 id="codex-switch-progress-title">
              {authFailure
                ? authFailure.apiOnlyAvailable
                  ? t("codex.switchAuth.apiOnlyTitle")
                  : t("codex.switchAuth.reauthorizeTitle")
                : t("codex.switch", "切换账号")}
            </h2>
            <p>{accountLabel}</p>
          </div>
        </div>

        <div className="codex-switch-progress-overview">
          <div className="codex-switch-progress-stage-row">
            <span>{statusLabel}</span>
            <span>{Math.round(state.progress)}%</span>
          </div>
          <div
            className="codex-switch-progress-track"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={state.progress}
          >
            <div
              className={`codex-switch-progress-bar ${isError ? (isApiOnlyAuthFailure ? "warning" : "error") : ""}`}
              style={{ width: `${state.progress}%` }}
            />
          </div>
        </div>

        <div className="codex-switch-progress-body">
          <div className="codex-switch-step-list">
            {state.steps.map((step) => {
              const detailLines = stepDetailLines(step);
              return (
                <div key={step.id} className={`codex-switch-step ${step.status}`}>
                  <div className="codex-switch-step-rail" aria-hidden="true">
                    <span className="codex-switch-step-icon">{renderStepIcon(step)}</span>
                  </div>
                  <div className="codex-switch-step-content">
                    <div className="codex-switch-step-title-row">
                      <strong>{stepLabel(step.id)}</strong>
                      <span>{t(`codex.switchProgress.status.${step.status}`)}</span>
                    </div>
                    {step.status !== "pending" && (
                      <div className="codex-switch-step-details">
                        {detailLines.map((line, index) => (
                          <span key={`${step.id}-${index}`}>{line}</span>
                        ))}
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
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
          {isError && !authFailure && (
            <div className="codex-switch-progress-error" role="alert">
              {windowsOperationError?.originalReason || state.error}
            </div>
          )}
        </div>

        {isError && authFailure && (
          <div className="codex-switch-progress-footer codex-switch-auth-footer">
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => setState(null)}
              disabled={actionBusy !== null}
            >
              {t("common.cancel", "取消")}
            </button>
            {authFailure.apiOnlyAvailable && (
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => void handleUseForApiService()}
                disabled={actionBusy !== null}
              >
                {actionBusy === "api"
                  ? t("common.loading", "加载中...")
                  : t("codex.localAccess.entryAction", "添加至 API 服务")}
              </button>
            )}
            <button
              type="button"
              className="btn btn-primary"
              onClick={handleReauthorize}
              disabled={actionBusy !== null}
            >
              {t("common.reauthorize", "重新授权")}
            </button>
          </div>
        )}

        {isError && !authFailure && (
          <div className="codex-switch-progress-footer">
            <button type="button" className="btn btn-secondary" onClick={() => setState(null)}>
              {t("common.close", "关闭")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
