import { useRef } from "react";
import { useTranslation } from "react-i18next";
import { X } from "lucide-react";
import { useModalFocusTrap } from "../../hooks/useModalFocusTrap";
import { useEscCloseTopmost } from "../../hooks/useEscClose";
import { getCodexLocalAccessRiskNoticeConfirmLabel, type CodexLocalAccessRiskNoticeAction } from "../../utils/codexLocalAccessRiskNotice";

export function CodexLocalAccessRiskNoticeDialog({ action, remember, onRememberChange, onClose }: {
  action: CodexLocalAccessRiskNoticeAction;
  remember: boolean;
  onRememberChange: (remember: boolean) => void;
  onClose: (accepted: boolean) => void;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  useModalFocusTrap(dialog, true);
  useEscCloseTopmost(true, () => onClose(false), dialog);
  return (
    <div className="modal-overlay codex-local-access-hide-confirm-overlay codex-local-access-risk-notice-overlay">
    <div
      ref={dialog} tabIndex={-1} role="dialog" aria-modal="true" aria-label={t("codex.localAccess.riskNotice.title")}
      className="modal codex-local-access-hide-confirm-modal codex-local-access-risk-notice-modal"
      onClick={(event) => event.stopPropagation()}
    >
      <div className="modal-header">
        <h2>
          {t("codex.localAccess.riskNotice.title", "使用风险提示")}
        </h2>
        <button
          className="modal-close"
          onClick={() => onClose(false)}
          aria-label={t("common.close", "关闭")}
        >
          <X />
        </button>
      </div>
      <div className="modal-body">
        <p className="codex-local-access-hide-confirm-desc">
          {t(
            "codex.localAccess.riskNotice.message",
            "当前 Codex API 服务相关功能，本质上属于代理转发使用方式。就目前情况看，官方暂未对此类行为进行明确管控，但后续政策、规则或可用性是否发生变化，仍存在不确定性。继续使用该功能，即表示您已知悉相关情况，并愿意自行承担可能产生的风险。",
          )}
        </p>
        <div className="codex-local-access-hide-confirm-points codex-local-access-risk-notice-points">
          <label className="codex-local-access-risk-notice-remember">
            <input
              type="checkbox"
              checked={remember}
              onChange={(event) => {
                onRememberChange(
                  event.target.checked,
                );
              }}
            />
            <span>
              {t(
                "codex.localAccess.riskNotice.remember",
                "我已知晓，不再提示",
              )}
            </span>
          </label>
        </div>
      </div>
      <div className="modal-footer">
        <button
          className="btn btn-secondary"
          onClick={() => onClose(false)}
        >
          {t("common.cancel", "取消")}
        </button>
        <button
          className="btn btn-primary"
          onClick={() => onClose(true)}
        >
          {getCodexLocalAccessRiskNoticeConfirmLabel(
            action,
            t,
          )}
        </button>
      </div>
    </div>
  </div>
  );
}
