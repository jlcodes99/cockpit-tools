import { useEffect, useRef, useState } from "react";
import { Copy, Search } from "lucide-react";
import { useTranslation } from "react-i18next";
import { checkSessionHistoryHealth, createHistoryRecoveryCopy } from "../../services/codexInstanceService";
import type { CodexHistoryHealth, CodexHistoryRecoveryCopy } from "../../types/codex";
import "./CodexHistoryHealthPanel.css";

export function CodexHistoryHealthPanel({ instanceId, threadId, threadTitle, disabled }: {
  instanceId: string; threadId: string; threadTitle?: string; disabled: boolean;
}) {
  const { t } = useTranslation();
  const revision = useRef(0);
  const [busy, setBusy] = useState(false);
  const [health, setHealth] = useState<CodexHistoryHealth | null>(null);
  const [copy, setCopy] = useState<CodexHistoryRecoveryCopy | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    revision.current += 1;
    setHealth(null); setCopy(null); setError(null); setBusy(false);
    return () => { revision.current += 1; };
  }, [instanceId, threadId]);
  async function run(createCopy: boolean) {
    if (busy || disabled || !instanceId) return;
    const version = revision.current;
    setBusy(true); setError(null);
    try {
      if (createCopy && health) {
        const result = await createHistoryRecoveryCopy(instanceId, threadId, health.sourceSha256);
        if (version === revision.current) setCopy(result);
      } else {
        const result = await checkSessionHistoryHealth(instanceId, threadId);
        if (version === revision.current) { setHealth(result); setCopy(null); }
      }
    } catch (reason) {
      if (version === revision.current) {
        setError(String(reason).replace(/^Error:\s*/, ""));
        setHealth(null);
        setCopy(null);
      }
    } finally {
      if (version === revision.current) setBusy(false);
    }
  }
  const plan = health?.recoveryPlan;
  const needsReview = health?.issues.some((issue) => issue.blocksRewrite || issue.code === "all_records_inherited");
  return <section className="codex-history-health" aria-busy={busy}>
    <div className="codex-history-health-toolbar">
      <div className="codex-history-health-heading">
        <span>{t("codex.historyHealth.userTitle", "会话历史检查")}</span>
        <h3>{threadTitle?.trim() || t("codex.historyHealth.untitled", "未命名会话")}</h3>
      </div>
      <button type="button" className="btn btn-secondary" disabled={busy || disabled || !instanceId} onClick={() => void run(false)}>
        <Search size={14} />{t("codex.historyHealth.inspect", "检查历史")}
      </button>
    </div>
    {error && <p role="alert">{error === "source_changed_during_scan" || error === "rollout_changed_since_plan"
      ? t("codex.historyHealth.sourceUpdating", "会话正在更新，请稍后重试。")
      : t("codex.historyHealth.checkFailed", "暂时无法完成操作，请重试。")}</p>}
    {health && <>
      <p role="status" className="codex-history-health-summary">{copy
        ? t("codex.historyHealth.prepared", "修复副本已准备好，原会话尚未修改。")
        : plan ? t("codex.historyHealth.repairable", "发现历史显示异常，可以先准备修复副本。")
        : needsReview ? t("codex.historyHealth.review", "这段历史需要进一步检查，暂不支持自动修复。")
        : health.issues.length ? t("codex.historyHealth.unconfirmed", "暂未确认历史是否完整，请稍后重新检查。")
        : t("codex.historyHealth.normal", "未发现历史记录异常。")}</p>
      {plan && !copy && <button type="button" className="btn btn-secondary" disabled={busy || disabled} onClick={() => void run(true)}>
        <Copy size={14}/>{t("codex.historyHealth.prepare", "准备修复副本")}
      </button>}
      {plan && !copy && <p>{t("codex.historyHealth.safeCopy", "仅生成本地副本，不修改原会话。副本包含私有历史，请勿上传。")}</p>}
    </>}
    {(health || error) && <details className="codex-history-health-details">
      <summary>{t("codex.historyHealth.details", "技术详情")}</summary>
      {error && <p>{error}</p>}
      {health && <>
      <dl>
        <div><dt>{t("codex.historyHealth.records", "Rollout 记录")}</dt><dd>{health.records}</dd></div>
        <div><dt>{t("codex.historyHealth.turns", "已投影轮数")}</dt><dd>{health.projectedTurns ?? "-"}</dd></div>
        <div><dt>{t("codex.historyHealth.offset", "字节游标 / 文件长度")}</dt><dd>{health.cursorOffset ?? "-"} / {health.rolloutBytes}</dd></div>
        <div><dt>{t("codex.historyHealth.ordinal", "下一个 ordinal")}</dt><dd>{health.cursorOrdinal ?? "-"}</dd></div>
      </dl>
      <ul>{health.issues.length ? health.issues.map((issue, index) =>
        <li key={index}>{t(`codex.historyHealth.issue.${issue.code}`, issue.code)}{issue.line != null ? ` (${issue.line})` : ""}</li>
      ) : <li>{t("codex.historyHealth.noAnomaly", "未发现结构异常")}</li>}</ul>
      {plan && <p>{t("codex.historyHealth.plan", "候选游标：{{from}} → {{to}}；{{count}} 条元数据记录", {
        from: plan.fromOffset, to: plan.toOffset, count: plan.skippedLines,
      })}</p>}
      {copy && <p><code>{copy.directory}</code></p>}
      </>}
    </details>}
  </section>;
}
