import { create } from 'zustand';
import * as kimiWakeupService from '../services/kimiWakeupService';
import type {
  KimiCliStatus,
  KimiWakeupHistoryItem,
  KimiWakeupState,
  KimiWakeupTask,
} from '../types/kimiWakeup';

interface KimiWakeupStore {
  state: KimiWakeupState;
  history: KimiWakeupHistoryItem[];
  runtime: KimiCliStatus | null;
  loading: boolean;
  error: string | null;
  fetchOverview: () => Promise<void>;
  saveState: (state: KimiWakeupState) => Promise<void>;
  setEnabled: (enabled: boolean) => Promise<void>;
  upsertTask: (task: KimiWakeupTask) => Promise<void>;
  deleteTask: (taskId: string) => Promise<void>;
  toggleTask: (taskId: string, enabled: boolean) => Promise<void>;
  clearHistory: () => Promise<void>;
}

const emptyState: KimiWakeupState = { enabled: false, tasks: [] };

export const useKimiWakeupStore = create<KimiWakeupStore>((set, get) => ({
  state: emptyState,
  history: [],
  runtime: null,
  loading: false,
  error: null,

  fetchOverview: async () => {
    set({ loading: true, error: null });
    try {
      const overview = await kimiWakeupService.getKimiWakeupOverview();
      set({
        state: overview.state,
        history: overview.history,
        runtime: overview.runtime,
        loading: false,
      });
    } catch (e) {
      set({ loading: false, error: String(e) });
    }
  },

  saveState: async (state) => {
    const saved = await kimiWakeupService.saveKimiWakeupState(state);
    set({ state: saved });
  },

  setEnabled: async (enabled) => {
    const next = { ...get().state, enabled };
    await get().saveState(next);
  },

  upsertTask: async (task) => {
    const tasks = [...get().state.tasks];
    const idx = tasks.findIndex((t) => t.id === task.id);
    if (idx >= 0) tasks[idx] = task;
    else tasks.push(task);
    await get().saveState({ ...get().state, tasks });
  },

  deleteTask: async (taskId) => {
    const tasks = get().state.tasks.filter((t) => t.id !== taskId);
    await get().saveState({ ...get().state, tasks });
  },

  toggleTask: async (taskId, enabled) => {
    const tasks = get().state.tasks.map((t) =>
      t.id === taskId ? { ...t, enabled, updated_at: Math.floor(Date.now() / 1000) } : t,
    );
    await get().saveState({ ...get().state, tasks });
  },

  clearHistory: async () => {
    await kimiWakeupService.clearKimiWakeupHistory();
    set({ history: [] });
  },
}));
