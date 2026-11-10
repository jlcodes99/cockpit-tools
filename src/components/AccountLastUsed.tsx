import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, Clock } from "lucide-react";
import { useUiConfigStore, type TimeDisplayMode } from "../stores/useUiConfigStore";

interface AccountLastUsedProps {
  /** 账号唯一 id，用于单卡片临时覆盖（store 内按 id 记录） */
  accountId: string;
  lastSwitchedAt?: number | null;
  createdAt: number;
  formatDate: (timestamp: number) => string;
  /** 刷新时间时间戳（来自配额查询）。仅 CodeBuddy 系有；其它平台不传 → "刷新时间"显示「暂无」并禁用。 */
  refreshAt?: number | null;
  /** 积分到期时间（最近一笔即将到期的积分）。没有积分体系的平台不传。 */
  expireAt?: number | null;
}

interface TimeOption {
  mode: TimeDisplayMode;
  label: string;
  enabled: boolean;
}

// 时间区渲染为自定义下拉（不用原生 <select>：原生弹出层不跟随暗色主题）。
// - 全局模式来自 useUiConfigStore.timeDisplayMode；单卡片临时覆盖记录在 store.cardTimeModes（不持久化）。
// - 可用项：切换时间（last_switched_at）、刷新时间（配额刷新）、到期时间（积分到期）。
// - 全部不可用时回退纯文本「暂无」，避免空下拉。
export function AccountLastUsed({
  accountId,
  lastSwitchedAt,
  formatDate,
  refreshAt,
  expireAt,
}: AccountLastUsedProps) {
  const { t } = useTranslation();
  const timeDisplayMode = useUiConfigStore((s) => s.timeDisplayMode);
  const cardTimeMode = useUiConfigStore((s) => s.cardTimeModes.get(accountId));
  const setCardTimeMode = useUiConfigStore((s) => s.setCardTimeMode);
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLSpanElement | null>(null);

  const hasRefresh = typeof refreshAt === "number" && refreshAt > 0;
  const switched = typeof lastSwitchedAt === "number" && lastSwitchedAt > 0;
  const hasExpire = typeof expireAt === "number" && expireAt > 0;

  const options = useMemo<TimeOption[]>(() => {
    const list: TimeOption[] = [];
    if (hasRefresh) {
      list.push({
        mode: "refresh",
        label: t("accounts.refreshTimeFull", "刷新时间：{{time}}", {
          time: formatDate(refreshAt as number),
        }),
        enabled: true,
      });
    }
    if (switched) {
      list.push({
        mode: "switch",
        label: t("accounts.lastSwitchTimeFull", "上次切换时间：{{time}}", {
          time: formatDate(lastSwitchedAt!),
        }),
        enabled: true,
      });
    }
    if (hasExpire) {
      list.push({
        mode: "expire",
        label: t("accounts.creditsExpireFull", "下次积分到期：{{time}}", {
          time: formatDate(expireAt as number),
        }),
        enabled: true,
      });
    }
    return list;
  }, [hasRefresh, switched, hasExpire, refreshAt, lastSwitchedAt, expireAt, formatDate, t]);

  useEffect(() => {
    if (!open) return;
    const onDocDown = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDocDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("mousedown", onDocDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  if (options.length === 0) {
    return (
      <span className="card-date account-last-used">
        <Clock size={12} />
        <span>{t("common.none", "暂无")}</span>
      </span>
    );
  }

  const requested = cardTimeMode ?? timeDisplayMode;
  const current = options.find((option) => option.mode === requested) ?? options[0];

  return (
    <span className="card-date account-last-used" ref={rootRef}>
      <Clock size={12} />
      <button
        type="button"
        className={`alu-trigger${open ? " open" : ""}`}
        onClick={() => setOpen((value) => !value)}
        title={t("accounts.toggleTimeMode", "切换显示 上次切换时间 / 刷新时间 / 下次积分到期")}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        <span>{current.label}</span>
        <ChevronDown size={12} aria-hidden="true" />
      </button>
      {open && (
        <span className="alu-menu" role="listbox">
          {options.map((option) => (
            <button
              key={option.mode}
              type="button"
              role="option"
              aria-selected={option.mode === current.mode}
              className={`alu-menu-item${option.mode === current.mode ? " active" : ""}`}
              onClick={() => {
                setCardTimeMode(accountId, option.mode);
                setOpen(false);
              }}
            >
              {option.label}
            </button>
          ))}
        </span>
      )}
    </span>
  );
}
