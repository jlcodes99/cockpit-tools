import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { RefreshCw } from "lucide-react";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import {
  getCodexLocalAccessState,
  updateCodexLocalAccessGatewayMode,
} from "../../services/codexLocalAccessService";
import type { CodexLocalAccessState } from "../../types/codexLocalAccess";
import "./CodexGatewayModeControl.css";

interface Props {
  state: CodexLocalAccessState | null;
  disabled?: boolean;
  onState: (state: CodexLocalAccessState) => void;
  onError: (error: string) => void;
  onNotice: (notice: string) => void;
  onSavingChange?: (saving: boolean) => void;
}

const gatewayErrorKeys: Record<string, string> = {
  NATIVE_API_BUSY: "codex.localAccess.gatewayEngineRequestsBusy",
  NATIVE_API_SWITCH_BUSY: "codex.localAccess.gatewayEngineOperationBusy",
  NATIVE_API_SWITCH_FAILED: "codex.localAccess.gatewayEngineSwitchRestored",
  NATIVE_API_ROLLBACK_FAILED: "codex.localAccess.gatewayEngineRestoreFailed",
  NATIVE_API_STOPPED: "codex.localAccess.gatewayEngineStopped",
  CHATGPT_PLAN_NATIVE_REQUIRED: "chatgptPlan.error.nativeSwitch",
};

export function formatCodexGatewayModeError(t: TFunction, error: unknown): string {
  const raw = String(error).replace(/^Error:\s*/, "").trim();
  const match = /^(NATIVE_API_[A-Z_]+|CHATGPT_PLAN_NATIVE_REQUIRED)(?::\s*([\s\S]*))?$/.exec(raw);
  const key = match && gatewayErrorKeys[match[1]];
  if (!key) return raw;
  return `${t(key)}${match?.[2] && match[1].startsWith("NATIVE_API_") ? `\n${match[2]}` : ""}`;
}

/** Shared by the full API service page and its quick panel. */
export function CodexGatewayModeControl({
  state, disabled, onState, onError, onNotice, onSavingChange,
}: Props) {
  const { t } = useTranslation();
  const pending = useRef(false);
  const [saving, setSaving] = useState(false);
  const [readbackFailed, setReadbackFailed] = useState(false);
  const [error, setError] = useState("");
  const errorRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!error) return;
    errorRef.current?.scrollIntoView({ block: "nearest" });
    errorRef.current?.focus({ preventScroll: true });
  }, [error]);
  const mode = state?.collection?.gatewayMode ?? "native";
  const describeError = (error: unknown) => formatCodexGatewayModeError(t, error);

  const readBack = async () => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      const next = await Promise.race([
        getCodexLocalAccessState(),
        new Promise<never>((_, reject) => {
          timer = setTimeout(() => reject(new Error(t("codex.localAccess.gatewayEngineReadbackTimeout"))), 5000);
        }),
      ]);
      onState(next);
      setReadbackFailed(false);
      return "";
    } catch (error) {
      setReadbackFailed(true);
      return t("codex.localAccess.gatewayEngineReadbackFailed", { error: describeError(error) });
    } finally {
      clearTimeout(timer);
    }
  };

  const changeMode = async (value: string) => {
    if (pending.current || disabled || !state?.collection || readbackFailed
      || value === mode || (value !== "native" && value !== "sidecar")) return;
    pending.current = true;
    setSaving(true);
    onSavingChange?.(true);
    onError("");
    setError("");
    onNotice("");
    try {
      // Keep the last confirmed selection until the backend transaction succeeds.
      onState(await updateCodexLocalAccessGatewayMode(value));
      onNotice(t("codex.localAccess.gatewayModeSaveSuccess"));
    } catch (error) {
      const failure = t("codex.localAccess.gatewayEngineSwitchFailed", { error: describeError(error) });
      setError(failure);
      const readbackError = await readBack();
      setError(readbackError ? `${failure}\n${readbackError}` : failure);
    } finally {
      pending.current = false;
      setSaving(false);
      onSavingChange?.(false);
    }
  };

  const retryReadBack = async () => {
    if (pending.current) return;
    pending.current = true;
    setSaving(true);
    onError("");
    setError("");
    onNotice("");
    try {
      setError(await readBack());
    } finally {
      pending.current = false;
      setSaving(false);
    }
  };

  return (
    <div className="codex-gateway-mode-control" aria-busy={saving}>
      <div className="codex-gateway-mode-row">
        <span>{t("codex.localAccess.gatewayModeLabel")}</span>
        <SingleSelectDropdown
          value={readbackFailed ? "" : mode}
          options={[
            { value: "native", label: t("codex.localAccess.gatewayEngineNative") },
            { value: "sidecar", label: t("codex.localAccess.gatewayEngineSidecar") },
          ]}
          placeholder={readbackFailed
            ? t("codex.apiService.logs.gatewayModeUnknown")
            : t("codex.localAccess.gatewayModeLegacyShort")}
          onChange={(value) => void changeMode(value)}
          disabled={disabled || saving || readbackFailed || !state?.collection || state.preparing || state.recovering}
          ariaLabel={t("codex.localAccess.gatewayModeLabel")}
        />
      </div>
      <p>{t("codex.localAccess.gatewayEngineHint")}</p>
      {saving && <div className="codex-gateway-mode-progress" role="status">
        <RefreshCw size={14} className="loading-spinner" />
        <span>{t("codex.localAccess.gatewayEngineSwitching")}</span>
      </div>}
      {error && <div className="codex-gateway-mode-error" role="alert" tabIndex={-1} ref={errorRef}>
        {error}
      </div>}
      {readbackFailed && <button type="button" className="btn btn-secondary btn-sm"
        disabled={saving || disabled} onClick={() => void retryReadBack()}>
        {t("common.retry")}
      </button>}
    </div>
  );
}
