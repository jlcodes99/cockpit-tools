import { create } from "zustand";

const UI_CONFIG_STORAGE_KEY = "agtools.ui_config.v1";

export interface UiConfigState {
  /** 是否显示账号「切换时间」（最后切换时间） */
  showSwitchTime: boolean;
  /** 是否显示配额「刷新时间」（下次刷新 / 上次成功） */
  showRefreshTime: boolean;
  setShowSwitchTime: (value: boolean) => void;
  setShowRefreshTime: (value: boolean) => void;
}

interface PersistedUiConfig {
  showSwitchTime?: boolean;
  showRefreshTime?: boolean;
}

function loadUiConfig(): { showSwitchTime: boolean; showRefreshTime: boolean } {
  try {
    const raw = localStorage.getItem(UI_CONFIG_STORAGE_KEY);
    if (!raw) return { showSwitchTime: true, showRefreshTime: true };
    const parsed = JSON.parse(raw) as PersistedUiConfig;
    return {
      showSwitchTime: parsed.showSwitchTime !== false,
      showRefreshTime: parsed.showRefreshTime !== false,
    };
  } catch {
    return { showSwitchTime: true, showRefreshTime: true };
  }
}

function persistUiConfig(state: {
  showSwitchTime: boolean;
  showRefreshTime: boolean;
}) {
  try {
    localStorage.setItem(
      UI_CONFIG_STORAGE_KEY,
      JSON.stringify({
        showSwitchTime: state.showSwitchTime,
        showRefreshTime: state.showRefreshTime,
      }),
    );
  } catch {
    /* localStorage 不可用时忽略 */
  }
}

const initial = loadUiConfig();

export const useUiConfigStore = create<UiConfigState>((set, get) => ({
  ...initial,
  setShowSwitchTime: (value) => {
    set({ showSwitchTime: value });
    persistUiConfig(get());
  },
  setShowRefreshTime: (value) => {
    set({ showRefreshTime: value });
    persistUiConfig(get());
  },
}));
