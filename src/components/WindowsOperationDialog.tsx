import { AlertTriangle, Check, ChevronDown, ChevronUp, Copy, FolderOpen, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useEscClose } from "../hooks/useEscClose";
import { useWindowsOperationDialogStore } from "../stores/useWindowsOperationDialogStore";
import {
  parseWindowsOperationError,
  redactWindowsOperationError,
} from "../utils/windowsOperationError";
import "./WindowsOperationDialog.css";

type ActionKind = "retry" | "manual" | "authorize" | "open";

export function WindowsOperationDialog() {
  const { t } = useTranslation();
  const request = useWindowsOperationDialogStore((state) => state.request);
  const close = useWindowsOperationDialogStore((state) => state.close);
  const replaceError = useWindowsOperationDialogStore((state) => state.replaceError);
  const [busy, setBusy] = useState<ActionKind | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    setBusy(null);
    setActionError(null);
    setDetailsOpen(false);
    setCopied(false);
  }, [request]);

  const safeClose = useCallback(() => {
    if (!busy) close();
  }, [busy, close]);
  useEscClose(Boolean(request && !busy), safeClose);

  const description = useMemo(() => {
    if (!request) return "";
    switch (request.error.code) {
      case "access_denied":
        return t("common.windowsOperation.accessDeniedDescription");
      case "file_in_use":
        return t("common.windowsOperation.fileInUseDescription");
      case "program_not_found":
        return t("common.windowsOperation.programNotFoundDescription");
      case "port_denied":
        return t("common.windowsOperation.portDeniedDescription");
      default:
        return t("common.windowsOperation.genericDescription");
    }
  }, [request, t]);

  if (!request) return null;

  const { error } = request;
  const runAction = async (
    kind: ActionKind,
    action: (() => void | Promise<void>) | undefined,
    closeOnSuccess = true,
  ) => {
    if (!action || busy) return;
    setBusy(kind);
    setActionError(null);
    try {
      await Promise.resolve(action());
      if (closeOnSuccess) {
        await Promise.resolve(request.onResolved?.());
        close();
      }
    } catch (nextError) {
      if (String(nextError).includes("WINDOWS_ELEVATION_CANCELLED")) {
        setActionError(t("common.windowsOperation.authorizationCancelled"));
      } else {
        const parsed = parseWindowsOperationError(nextError, {
          operation: error.operation,
          target: error.target,
          summary: error.summary,
        });
        if (parsed) {
          replaceError(parsed);
          setActionError(null);
        } else {
          setActionError(
            redactWindowsOperationError(String(nextError).replace(/^Error:\s*/, "")),
          );
        }
      }
    } finally {
      setBusy(null);
    }
  };

  const copyDetails = async () => {
    const text = [
      `${t("common.windowsOperation.operationLabel")}: ${error.operation}`,
      `${t("common.windowsOperation.originalReason")}: ${error.originalReason}`,
      error.target ? `${t("common.windowsOperation.targetLabel")}: ${error.target}` : "",
      error.pids.length ? `PID: ${error.pids.join(", ")}` : "",
      error.attemptedRecoveries.length
        ? `${t("common.windowsOperation.attemptedRecoveries")}: ${error.attemptedRecoveries.join(" · ")}`
        : "",
    ].filter(Boolean).join("\n");
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch (copyError) {
      setActionError(redactWindowsOperationError(String(copyError)));
    }
  };

  const manualAction = request.manualContinue ?? request.retry;

  return createPortal(
    <div className="modal-overlay windows-operation-overlay">
      <div
        className="modal windows-operation-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="windows-operation-dialog-title"
      >
        <div className="modal-header windows-operation-header">
          <div className="windows-operation-title-wrap">
            <span className="windows-operation-icon"><AlertTriangle size={20} /></span>
            <div>
              <h2 id="windows-operation-dialog-title">
                {t("common.windowsOperation.title")}
              </h2>
            </div>
          </div>
          <button
            type="button"
            className="modal-close"
            onClick={safeClose}
            disabled={Boolean(busy)}
            aria-label={t("common.close")}
          >
            <X />
          </button>
        </div>

        <div className="modal-body windows-operation-body">
          <p className="windows-operation-description">{description}</p>
          <div className="windows-operation-reason" role="alert">
            <strong>{t("common.windowsOperation.originalReason")}</strong>
            <span>{error.originalReason}</span>
          </div>

          <div className="windows-operation-inline-actions">
            <button type="button" className="btn btn-secondary" onClick={() => void copyDetails()}>
              {copied ? <Check size={15} /> : <Copy size={15} />}
              {copied ? t("common.copied") : t("common.windowsOperation.copyError")}
            </button>
            {request.openTarget && (
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => void runAction("open", request.openTarget, false)}
                disabled={Boolean(busy)}
              >
                <FolderOpen size={15} />
                {t("common.windowsOperation.openTarget")}
              </button>
            )}
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => setDetailsOpen((value) => !value)}
              aria-expanded={detailsOpen}
            >
              {detailsOpen ? <ChevronUp size={15} /> : <ChevronDown size={15} />}
              {detailsOpen
                ? t("common.windowsOperation.hideDetails")
                : t("common.windowsOperation.viewDetails")}
            </button>
          </div>

          {detailsOpen && (
            <div className="windows-operation-details">
              <div><strong>{t("common.windowsOperation.operationLabel")}</strong><span>{error.operation}</span></div>
              {error.target && <div><strong>{t("common.windowsOperation.targetLabel")}</strong><span>{error.target}</span></div>}
              {error.pids.length > 0 && <div><strong>PID</strong><span>{error.pids.join(", ")}</span></div>}
              {error.attemptedRecoveries.length > 0 && (
                <div>
                  <strong>{t("common.windowsOperation.attemptedRecoveries")}</strong>
                  <span>{error.attemptedRecoveries.join(" · ")}</span>
                </div>
              )}
            </div>
          )}

          {actionError && <div className="windows-operation-action-error" role="alert">{actionError}</div>}
        </div>

        <div className="modal-footer windows-operation-footer">
          <button type="button" className="btn btn-secondary" onClick={safeClose} disabled={Boolean(busy)}>
            {t("common.cancel")}
          </button>
          {error.manualActionAvailable && manualAction && (
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => void runAction("manual", manualAction)}
              disabled={Boolean(busy)}
            >
              {t("common.windowsOperation.manualContinue")}
            </button>
          )}
          {error.retryable && request.retry && (
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => void runAction("retry", request.retry)}
              disabled={Boolean(busy)}
            >
              {t("common.windowsOperation.retry")}
            </button>
          )}
          {request.authorize && (
            <button
              type="button"
              className="btn btn-primary"
              onClick={() => void runAction("authorize", request.authorize)}
              disabled={Boolean(busy)}
            >
              {t("common.windowsOperation.authorizeContinue")}
            </button>
          )}
        </div>
      </div>
    </div>,
    document.body,
  );
}
