import { useCallback, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { RefreshCw, Shuffle, Wrench, X } from "lucide-react";
import { useEscCloseTopmost } from "../../hooks/useEscClose";
import { useModalFocusTrap } from "../../hooks/useModalFocusTrap";
import {
  changeCodexLocalAccessToRandomPort,
  killCodexLocalAccessPort,
} from "../../services/codexLocalAccessService";
import type { CodexLocalAccessState } from "../../types/codexLocalAccess";
import { formatCodexGatewayModeError } from "../../utils/codexGatewayError";
import { parseWindowsOperationError } from "../../utils/windowsOperationError";
import { ModalErrorMessage, useModalErrorState } from "../ModalErrorMessage";
import "./CodexLocalAccessPortRecovery.css";

type PortAction = "random" | "cleanup";

export function useCodexLocalAccessPortRecovery(
  port: number | undefined,
  onSuccess: (state: CodexLocalAccessState, message: string) => void,
) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState<PortAction | null>(null);
  const running = useRef(false);
  const dialogRef = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const descriptionId = useId();
  const error = useModalErrorState();
  const { clear } = error;

  const requestPortRecovery = useCallback(() => {
    if (port == null || running.current) return;
    clear();
    setOpen(true);
  }, [clear, port]);
  const close = () => {
    if (running.current) return;
    error.clear();
    setOpen(false);
  };

  useModalFocusTrap(dialogRef, open);
  useEscCloseTopmost(open, close, dialogRef);

  const execute = async (action: PortAction) => {
    if (running.current) return;
    running.current = true;
    setBusy(action);
    error.clear();
    try {
      if (action === "random") {
        const state = await changeCodexLocalAccessToRandomPort();
        onSuccess(state, t("codex.localAccess.randomPortSuccess", { port: state.collection?.port }));
      } else {
        const { state, killedCount } = await killCodexLocalAccessPort();
        onSuccess(state, killedCount > 0
          ? t("codex.localAccess.killPortSuccess", { count: killedCount })
          : t("codex.localAccess.killPortSuccessUnknown"));
      }
      setOpen(false);
    } catch (cause) {
      error.report(
        parseWindowsOperationError(cause)?.originalReason
          || formatCodexGatewayModeError(t, cause),
      );
    } finally {
      running.current = false;
      setBusy(null);
    }
  };

  const portRecoveryDialog = open ? createPortal(
    <div className="modal-overlay codex-port-recovery-overlay">
      <div
        className="modal codex-port-recovery-modal"
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={descriptionId}
        tabIndex={-1}
      >
        <div className="modal-header">
          <h2 id={titleId}>{t("codex.localAccess.portRecoveryTitle")}</h2>
          <button type="button" className="btn btn-secondary btn-sm" aria-label={t("common.close")}
            onClick={close} disabled={busy !== null}>
            <X size={16} />
          </button>
        </div>
        <div className="modal-body">
          <p id={descriptionId}>{t("codex.localAccess.portRecoveryCurrent", { port })}</p>
          <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />
          <div className="codex-port-recovery-options" aria-busy={busy !== null}>
            <div>
              <button type="button" className="btn btn-primary" onClick={() => void execute("random")}
                disabled={busy !== null}>
                {busy === "random" ? <RefreshCw size={15} className="loading-spinner" /> : <Shuffle size={15} />}
                {t("codex.localAccess.randomPortAction")}
              </button>
              <p>{t("codex.localAccess.randomPortDescription")}</p>
            </div>
            <div>
              <button type="button" className="btn btn-secondary" onClick={() => void execute("cleanup")}
                disabled={busy !== null}>
                {busy === "cleanup" ? <RefreshCw size={15} className="loading-spinner" /> : <Wrench size={15} />}
                {t("codex.localAccess.killPortAction")}
              </button>
              <p>{t("codex.localAccess.keepPortDescription")}</p>
            </div>
          </div>
          {busy && <p role="status">{t("common.loading")}</p>}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" onClick={close} disabled={busy !== null}>
            {t("common.cancel")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  ) : null;

  return { requestPortRecovery, portRecoveryDialog, portRecoveryBusy: busy !== null };
}
