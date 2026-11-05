import { useTranslation } from "react-i18next";
import { Clock } from "lucide-react";
import { useUiConfigStore } from "../stores/useUiConfigStore";

interface AccountLastUsedProps {
  /** 账号唯一 id，用于单卡片临时覆盖（store 内按 id 记录） */
  accountId: string;
  lastUsed: number;
  createdAt: number;
  formatDate: (timestamp: number) => string;
  /** 刷新时间时间戳（来自配额查询）。仅 CodeBuddy 系有；其它平台不传 → "刷新时间"显示「暂无」并禁用。 */
  refreshAt?: number | null;
}

// 时间区统一渲染为 <select> 下拉，可直接在显示区切换「切换时间 / 刷新时间」。
// - 全局模式来自 useUiConfigStore.timeDisplayMode；单卡片临时覆盖记录在 store.cardTimeModes（不持久化）。
// - 有 last_used 时"切换时间"可选，否则禁用；有 refreshAt 时"刷新时间"可选，否则禁用。
// - 两者皆不可用时回退纯文本"从未切换"，避免空下拉。
export function AccountLastUsed({
  accountId,
  lastUsed,
  createdAt,
  formatDate,
  refreshAt,
}: AccountLastUsedProps) {
  const { t } = useTranslation();
  const timeDisplayMode = useUiConfigStore((s) => s.timeDisplayMode);
  const cardTimeMode = useUiConfigStore((s) => s.cardTimeModes.get(accountId));
  const setCardTimeMode = useUiConfigStore((s) => s.setCardTimeMode);

  const hasRefresh = typeof refreshAt === "number" && refreshAt > 0;
  const switched = lastUsed && lastUsed > 0;
  const mode = cardTimeMode ?? timeDisplayMode;
  const showRefresh = mode === "refresh" && hasRefresh;

  if (!switched && !hasRefresh) {
    return (
      <span className="card-date account-last-used">
        <Clock size={12} />
        <span>{t("accounts.neverSwitched", "从未切换")}</span>
      </span>
    );
  }

  return (
    <span className="card-date account-last-used">
      <Clock size={12} />
      <select
        className="alu-select"
        value={showRefresh ? "refresh" : "switch"}
        onChange={(e) => setCardTimeMode(accountId, e.target.value as "switch" | "refresh")}
        title={t("accounts.toggleTimeMode", "切换显示切换时间 / 刷新时间")}
      >
        <option value="refresh" disabled={!hasRefresh}>
          {hasRefresh
            ? t("accounts.refreshTimeFull", "刷新时间：{{time}}", { time: formatDate(refreshAt as number) })
            : `${t("accounts.refreshTime", "刷新时间")} (${t("common.none", "暂无")})`}
        </option>
        <option value="switch" disabled={!switched}>
          {switched
            ? t("accounts.lastSwitchTimeFull", "切换时间：{{time}}", { time: formatDate(lastUsed) })
            : t("accounts.neverSwitched", "从未切换")}
        </option>
      </select>
    </span>
  );
}
