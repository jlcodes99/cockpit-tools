import { useTranslation } from "react-i18next";
import { Clock } from "lucide-react";
import { useUiConfigStore } from "../stores/useUiConfigStore";
import { SingleSelectDropdown } from "./SingleSelectDropdown";

interface AccountLastUsedProps {
  /** 账号唯一 id，用于单卡片临时覆盖（store 内按 id 记录） */
  accountId: string;
  lastSwitchedAt?: number | null;
  createdAt: number;
  formatDate: (timestamp: number) => string;
  /** 来自配额查询的刷新时间。未提供时禁用「刷新时间」选项。 */
  refreshAt?: number | null;
}

// 时间区统一渲染为轻量自定义下拉，可直接在显示区切换「切换时间 / 刷新时间」。
// - 全局模式来自 useUiConfigStore.timeDisplayMode；单卡片临时覆盖记录在 store.cardTimeModes（不持久化）。
// - 有 last_switched_at 时"切换时间"可选，否则禁用；有 refreshAt 时"刷新时间"可选，否则禁用。
// - 两者皆不可用时回退纯文本"暂无"，避免空下拉。
export function AccountLastUsed({
  accountId,
  lastSwitchedAt,
  formatDate,
  refreshAt,
}: AccountLastUsedProps) {
  const { t } = useTranslation();
  const timeDisplayMode = useUiConfigStore((s) => s.timeDisplayMode);
  const cardTimeMode = useUiConfigStore((s) => s.cardTimeModes.get(accountId));
  const setCardTimeMode = useUiConfigStore((s) => s.setCardTimeMode);

  const hasRefresh = typeof refreshAt === "number" && refreshAt > 0;
  const switched = typeof lastSwitchedAt === "number" && lastSwitchedAt > 0;
  const mode = cardTimeMode ?? timeDisplayMode;
  const showRefresh = hasRefresh && (mode === "refresh" || !switched);

  if (!switched && !hasRefresh) {
    return (
      <span className="card-date account-last-used" onClick={(event) => event.stopPropagation()}>
        <Clock size={12} />
        <span>{t("common.none", "暂无")}</span>
      </span>
    );
  }

  return (
    <span
      className="card-date account-last-used"
      title={t("accounts.toggleTimeMode", "切换显示切换时间 / 刷新时间")}
      onClick={(event) => event.stopPropagation()}
    >
      <Clock size={12} />
      <SingleSelectDropdown
        className="alu-dropdown"
        menuWidth={220}
        value={showRefresh ? "refresh" : "switch"}
        ariaLabel={t("accounts.toggleTimeMode", "切换显示切换时间 / 刷新时间")}
        options={[
          {
            value: "refresh",
            disabled: !hasRefresh,
            label: hasRefresh
              ? t("accounts.refreshTimeFull", "刷新时间：{{time}}", { time: formatDate(refreshAt as number) })
              : `${t("accounts.refreshTime", "刷新时间")} (${t("common.none", "暂无")})`,
          },
          {
            value: "switch",
            disabled: !switched,
            label: switched
              ? t("accounts.lastSwitchTimeFull", "切换时间：{{time}}", { time: formatDate(lastSwitchedAt!) })
              : t("common.none", "暂无"),
          },
        ]}
        onChange={(value) => setCardTimeMode(accountId, value as "switch" | "refresh")}
      />
    </span>
  );
}
