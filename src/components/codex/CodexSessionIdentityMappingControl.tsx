import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useEscCloseTopmost } from "../../hooks/useEscClose";
import { useModalFocusTrap } from "../../hooks/useModalFocusTrap";
import { useModalScrollLock } from "../../hooks/useModalScrollLock";
import "./CodexSessionIdentityMappingControl.css";

// This is a service setting, shared by the API Service page and member dialog.
// General Settings configures desktop clients and has no service routing scope.
export function CodexSessionIdentityMappingControl({ checked, disabled, onChange, compact = false }: {
  checked: boolean;
  disabled: boolean;
  onChange: (enabled: boolean) => void;
  compact?: boolean;
}) {
  const { t } = useTranslation();
  const titleId = useId();
  const descriptionId = useId();
  const dialog = useRef<HTMLDivElement>(null);
  const [pending, setPending] = useState<boolean | null>(null);
  const pendingRef = useRef<boolean | null>(null);
  const latest = useRef({ checked, disabled, onChange });
  latest.current = { checked, disabled, onChange };
  const close = () => { pendingRef.current = null; setPending(null); };
  const open = pending !== null && !disabled && pending !== checked;
  useEffect(() => { close(); }, [checked, disabled]);
  useEffect(() => () => { pendingRef.current = null; }, []);
  useEscCloseTopmost(open, close, dialog);
  useModalFocusTrap(dialog, open);
  useModalScrollLock(open);

  const confirm = () => {
    const next = pendingRef.current;
    if (next === null || latest.current.disabled || latest.current.checked === next) return;
    close();
    latest.current.onChange(next);
  };
  return (
    <div className={`codex-session-identity-mapping${compact ? " codex-session-identity-mapping-compact" : ""}`}>
      <label>
        <span>{t("codex.apiService.routing.sessionIdentityMapping")}</span>
        <input
          type="checkbox"
          role="switch"
          checked={checked}
          disabled={disabled}
          aria-haspopup="dialog"
          onChange={(event) => {
            if (disabled || pendingRef.current !== null || event.target.checked === checked) return;
            pendingRef.current = event.target.checked;
            setPending(event.target.checked);
          }}
        />
      </label>
      {open && createPortal(
        <div className="modal-overlay codex-session-identity-confirm-overlay">
          <div ref={dialog} className="modal codex-session-identity-confirm" role="alertdialog"
            aria-modal="true" aria-labelledby={titleId} aria-describedby={descriptionId} tabIndex={-1}>
            <div className="modal-header">
              <h2 id={titleId}>{t("codex.apiService.routing.sessionIdentityMapping")}</h2>
              <button type="button" className="btn btn-secondary" aria-label={t("common.close")} onClick={close}><X size={16} /></button>
            </div>
            <div className="modal-body" id={descriptionId}>
              <p>{t(pending ? "codex.apiService.routing.sessionIdentityMappingEnableHint" : "codex.apiService.routing.sessionIdentityMappingDisableHint")}</p>
              <p>{t("codex.apiService.routing.sessionIdentityMappingCacheNote")}</p>
              <p>{t("codex.apiService.routing.sessionIdentityMappingSaveNote")}</p>
            </div>
            <div className="modal-footer">
              <button type="button" className="btn btn-secondary" onClick={close}>{t("common.cancel")}</button>
              <button type="button" className="btn btn-primary" onClick={confirm}>{t(pending ? "common.enable" : "common.disable")}</button>
            </div>
          </div>
        </div>, document.body,
      )}
    </div>
  );
}
