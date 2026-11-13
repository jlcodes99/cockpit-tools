import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Undo2 } from "lucide-react";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import { ModalErrorMessage } from "../ModalErrorMessage";
import type { CodexLocalAccessCollection } from "../../types/codexLocalAccess";
import {
  applyCodexClaudeCodeSettings,
  previewCodexClaudeCodeSettings,
  restoreCodexClaudeCodeSettings,
  type CodexClaudeCodeSettingsPreview,
} from "../../services/codexLocalAccessService";

type Collection = Pick<CodexLocalAccessCollection, "enabled" | "apiKey" | "apiKeys">;

export function CodexClaudeCodeSettings({ collection }: { collection: Collection | null }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [selectedKey, setSelectedKey] = useState("");
  const [preview, setPreview] = useState<(CodexClaudeCodeSettingsPreview & { keyId: string }) | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  const keys = (collection?.apiKeys ?? [])
    .filter((key) => key.enabled)
    .map((key) => ({ value: key.id, label: key.label }));
  if (collection?.apiKey && !collection.apiKeys.some((key) => key.key === collection.apiKey)) {
    keys.push({ value: "legacy", label: t("codex.localAccess.accountPoolHealth.dialog.unscopedApiKey") });
  }
  const keyId = keys.some((key) => key.value === selectedKey) ? selectedKey : keys[0]?.value;
  const currentPreview = preview?.keyId === keyId ? preview : null;

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError("");
    setSaved(false);
    try {
      await action();
    } catch (cause) {
      const detail = String(cause).replace(/^Error:\s*/, "");
      switch (detail) {
        case "CODEX_CLAUDE_CODE_NO_PRESET_MODELS":
          setError(t("codex.apiService.compat.claudeCodeNoModels"));
          break;
        case "CODEX_CLAUDE_CODE_SETTINGS_CHANGED":
          setPreview(null);
          setError(t("codex.apiService.compat.claudeCodeSettingsChanged"));
          break;
        case "CODEX_CLAUDE_CODE_SERVICE_UNAVAILABLE":
          setError(t("codex.apiService.compat.claudeCodeServiceUnavailable"));
          break;
        case "CODEX_CLAUDE_CODE_NO_BACKUP":
          setError(t("codex.apiService.compat.claudeCodeNoBackup"));
          break;
        default:
          setError(detail);
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="codex-api-service-config-list">
      <button type="button" className="btn btn-secondary btn-sm" aria-expanded={open}
        onClick={() => setOpen(!open)} disabled={busy}>
        {t("claude.cli.applyUserConfig")}
      </button>
      {open && (
        <>
          <label>
            <span>{t("codex.localAccess.apiKeysTitle")}</span>
            <SingleSelectDropdown value={keyId ?? ""} options={keys} disabled={busy || !keyId}
              ariaLabel={t("codex.localAccess.apiKeysTitle")}
              onChange={(value) => {
                setSelectedKey(value);
                setPreview(null);
                setError("");
                setSaved(false);
              }} />
          </label>
          <div className="codex-api-service-head-actions">
            <button type="button" className="btn btn-secondary btn-sm" disabled={busy || !keyId || !collection?.enabled}
              onClick={() => {
                if (!keyId) return;
                setPreview(null);
                void run(async () => {
                  const next = await previewCodexClaudeCodeSettings(keyId);
                  setPreview({ ...next, keyId });
                });
              }}>
              {t(busy ? "common.loading" : "common.preview")}
            </button>
            <button type="button" className="btn btn-secondary btn-sm" disabled={busy}
              onClick={() => void run(async () => {
                setPreview(null);
                await restoreCodexClaudeCodeSettings();
                setSaved(true);
              })}>
              <Undo2 size={14} />
              {t("common.recycleBin.restore")}
            </button>
          </div>
          <ModalErrorMessage message={error} />
          {saved && <span role="status">{t("common.success")}</span>}
          {currentPreview && (
            <>
              <small title={currentPreview.path}>{currentPreview.path}</small>
              <code>{JSON.stringify({ env: currentPreview.changes }, null, 2)}</code>
              <small>{t("codex.apiService.compat.claudeCodeSettingsHint")}</small>
              <button type="button" className="btn btn-primary btn-sm" disabled={busy || !currentPreview.changed || !collection?.enabled}
                onClick={() => void run(async () => {
                  await applyCodexClaudeCodeSettings(currentPreview.keyId, currentPreview.revision);
                  setPreview(null);
                  setSaved(true);
                })}>
                <Check size={14} />
                {t("common.save")}
              </button>
            </>
          )}
        </>
      )}
    </div>
  );
}
