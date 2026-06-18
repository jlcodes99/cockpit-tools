import { useCallback, useState } from "react";
import { RefreshCw, RotateCcw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { confirm } from "@tauri-apps/plugin-dialog";
import * as codexService from "../../services/codexService";
import type { CodexResetCreditUsage } from "../../types/codex";

type CodexResetCreditControlProps = {
  accountId: string;
  usage: CodexResetCreditUsage | null;
  variant?: "card" | "table";
  onUsageChange: (accountId: string, usage: CodexResetCreditUsage) => void;
  onChanged?: () => void | Promise<void>;
};

function normalizeError(error: unknown): string {
  return String(error).replace(/^Error:\s*/, "");
}

export function CodexResetCreditControl({
  accountId,
  usage,
  variant = "card",
  onUsageChange,
  onChanged,
}: CodexResetCreditControlProps) {
  const { t } = useTranslation();
  const [querying, setQuerying] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [notice, setNotice] = useState<{
    tone: "success" | "error";
    text: string;
  } | null>(null);

  const availableCount = usage?.available_count ?? null;
  const hasCredits = (availableCount ?? 0) > 0;
  const busy = querying || resetting;

  const queryCredits = useCallback(async () => {
    setQuerying(true);
    setNotice(null);
    try {
      const nextUsage = await codexService.queryCodexResetCredits(accountId);
      onUsageChange(accountId, nextUsage);
    } catch (error) {
      setNotice({
        tone: "error",
        text: t("codex.resetCredits.queryFailed", {
          defaultValue: "查询失败：{{error}}",
          error: normalizeError(error),
        }),
      });
    } finally {
      setQuerying(false);
    }
  }, [accountId, onUsageChange, t]);

  const resetCredit = useCallback(async () => {
    const confirmed = await confirm(
      t(
        "codex.resetCredits.confirm",
        "将消耗 1 次 ChatGPT reset credit 执行重置，是否继续？",
      ),
      {
        title: t("codex.resetCredits.confirmTitle", "执行重置"),
        kind: "warning",
      },
    );
    if (!confirmed) return;

    setResetting(true);
    setNotice(null);
    try {
      const result = await codexService.consumeCodexResetCredit(accountId);
      const nextUsage = await codexService.queryCodexResetCredits(accountId);
      onUsageChange(accountId, nextUsage);
      await onChanged?.();
      setNotice({
        tone: result.quota_refresh_error ? "error" : "success",
        text: result.quota_refresh_error
          ? t("codex.resetCredits.resetRefreshFailed", {
              defaultValue: "已重置，配额刷新失败：{{error}}",
              error: result.quota_refresh_error,
            })
          : t("codex.resetCredits.resetSuccess", "已执行重置"),
      });
    } catch (error) {
      setNotice({
        tone: "error",
        text: t("codex.resetCredits.resetFailed", {
          defaultValue: "重置失败：{{error}}",
          error: normalizeError(error),
        }),
      });
    } finally {
      setResetting(false);
    }
  }, [accountId, onChanged, onUsageChange, t]);

  return (
    <div className={`codex-reset-credit-control ${variant}`}>
      <div className="codex-reset-credit-main">
        <span className="codex-reset-credit-label">
          {t("codex.resetCredits.label", "Reset Credit")}
        </span>
        <strong className={hasCredits ? "available" : ""}>
          {availableCount == null ? "--" : availableCount}
        </strong>
      </div>
      <div className="codex-reset-credit-actions">
        <button
          type="button"
          className="codex-reset-credit-btn"
          onClick={() => void queryCredits()}
          disabled={busy}
          title={t("codex.resetCredits.query", "查询次数")}
          aria-label={t("codex.resetCredits.query", "查询次数")}
        >
          <RefreshCw size={13} className={querying ? "loading-spinner" : ""} />
        </button>
        <button
          type="button"
          className="codex-reset-credit-btn primary"
          onClick={() => void resetCredit()}
          disabled={busy || !hasCredits}
          title={t("codex.resetCredits.reset", "执行重置")}
        >
          <RotateCcw size={13} className={resetting ? "loading-spinner" : ""} />
          <span>{t("codex.resetCredits.resetShort", "重置")}</span>
        </button>
      </div>
      {notice && (
        <span className={`codex-reset-credit-notice ${notice.tone}`}>
          {notice.text}
        </span>
      )}
    </div>
  );
}
