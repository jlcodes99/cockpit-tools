import { create } from "zustand";

const UI_CONFIG_STORAGE_KEY = "agtools.ui_config.v1";

export type TimeDisplayMode = "switch" | "refresh";

export interface UiConfigState {
  /** 账号卡片时间区域显示内容：'switch' 切换时间（默认），'refresh' 刷新时间 */
  timeDisplayMode: TimeDisplayMode;
  setTimeDisplayMode: (value: TimeDisplayMode) => void;
  /**
   * 单卡片临时覆盖：仅记录与全局模式不同的账号，规模 = 账号数。
   * 不持久化（瞬态 UI 状态），切回全局模式时自动清除该账号条目。
   */
  cardTimeModes: Map<string, TimeDisplayMode>;
  setCardTimeMode: (id: string, mode: TimeDisplayMode) => void;
}

interface PersistedUiConfig {
  timeDisplayMode?: TimeDisplayMode;
  // ponytail: 旧版曾用 showSwitchTime/showRefreshTime 两个布尔，已合并为单一切换模式
  showSwitchTime?: boolean;
  showRefreshTime?: boolean;
}

function loadUiConfig(): { timeDisplayMode: TimeDisplayMode } {
  try {
    const raw = localStorage.getItem(UI_CONFIG_STORAGE_KEY);
    if (!raw) return { timeDisplayMode: "switch" };
    const parsed = JSON.parse(raw) as PersistedUiConfig;
    if (parsed.timeDisplayMode === "refresh") {
      return { timeDisplayMode: "refresh" };
    }
    if (parsed.timeDisplayMode === "switch") {
      return { timeDisplayMode: "switch" };
    }
    // ponytail: 默认"原来的时间"=切换时间（last_used）
    return { timeDisplayMode: "switch" };
  } catch {
    return { timeDisplayMode: "switch" };
  }
}

function persistUiConfig(state: { timeDisplayMode: TimeDisplayMode }) {
  try {
    localStorage.setItem(
      UI_CONFIG_STORAGE_KEY,
      JSON.stringify({ timeDisplayMode: state.timeDisplayMode }),
    );
  } catch {
    /* localStorage 不可用时忽略 */
  }
}

const initial = loadUiConfig();

export const useUiConfigStore = create<UiConfigState>((set, get) => ({
  ...initial,
  setTimeDisplayMode: (value) => {
    set({ timeDisplayMode: value });
    persistUiConfig(get());
  },
  cardTimeModes: new Map<string, TimeDisplayMode>(),
  setCardTimeMode: (id, mode) => {
    const globalMode = get().timeDisplayMode;
    set((s) => {
      const next = new Map(s.cardTimeModes);
      if (mode === globalMode) {
        next.delete(id); // 与全局一致时清除覆盖
      } else {
        next.set(id, mode);
      }
      return { cardTimeModes: next };
    });
  },
}));
