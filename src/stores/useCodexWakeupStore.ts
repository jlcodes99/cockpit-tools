import { create } from 'zustand';
import * as codexWakeupService from '../services/codexWakeupService';
import {
  CodexCliStatus,
  CodexWakeupBatchResult,
  CodexWakeupModelPreset,
  CodexWakeupReasoningEffort,
  CodexWakeupState,
  CodexWakeupTask,
} from '../types/codexWakeup';

interface CodexWakeupStoreState {
  runtime: CodexCliStatus | null;
  state: CodexWakeupState;
  loading: boolean;
  saving: boolean;
  runningTaskId: string | null;
  testing: boolean;
  error: string | null;
  loadAll: (refreshAfterPending?: boolean) => Promise<void>;
  refreshRuntime: (runtimeConfig?: { codexCliPath?: string; nodePath?: string }) => Promise<void>;
  saveState: (
    enabled: boolean,
    tasks: CodexWakeupTask[],
    modelPresets: CodexWakeupModelPreset[],
  ) => Promise<CodexWakeupState>;
  runTask: (taskId: string, runId: string) => Promise<CodexWakeupBatchResult>;
  runTest: (
    accountIds: string[],
    runId: string,
    prompt?: string,
    model?: string,
    modelDisplayName?: string,
    modelReasoningEffort?: CodexWakeupReasoningEffort,
    cancelScopeId?: string,
  ) => Promise<CodexWakeupBatchResult>;
  cancelTestScope: (cancelScopeId: string) => Promise<void>;
  releaseTestScope: (cancelScopeId: string) => Promise<void>;
  clearHistory: () => Promise<void>;
}

const EMPTY_STATE: CodexWakeupState = {
  enabled: false,
  tasks: [],
  model_presets: [],
  model_preset_migrations: [],
};

let loadAllInFlight: Promise<void> | null = null;

export const useCodexWakeupStore = create<CodexWakeupStoreState>((set, get) => ({
  runtime: null,
  state: EMPTY_STATE,
  loading: false,
  saving: false,
  runningTaskId: null,
  testing: false,
  error: null,
  loadAll: async (refreshAfterPending = false): Promise<void> => {
    if (loadAllInFlight) {
      if (refreshAfterPending) {
        await loadAllInFlight;
        return get().loadAll();
      }
      return loadAllInFlight;
    }
    set((current) => ({
      loading: current.runtime === null && current.state.tasks.length === 0,
      error: null,
    }));
    loadAllInFlight = (async () => {
      try {
        const overview = await codexWakeupService.getCodexWakeupOverview();
        set({
          runtime: overview.runtime,
          state: overview.state,
          loading: false,
        });
      } catch (error) {
        set({ loading: false, error: String(error) });
      } finally {
        loadAllInFlight = null;
      }
    })();
    return loadAllInFlight;
  },
  refreshRuntime: async (runtimeConfig) => {
    try {
      const runtime = runtimeConfig
        ? await codexWakeupService.updateCodexWakeupRuntimeConfig(
            runtimeConfig.codexCliPath,
            runtimeConfig.nodePath,
          )
        : await codexWakeupService.getCodexWakeupCliStatus();
      set({ runtime });
    } catch (error) {
      set({ error: String(error) });
      throw error;
    }
  },
  saveState: async (enabled, tasks, modelPresets) => {
    set({ saving: true, error: null });
    try {
      const currentMigrations = useCodexWakeupStore.getState().state.model_preset_migrations;
      const state = await codexWakeupService.saveCodexWakeupState(
        enabled,
        tasks,
        modelPresets,
        currentMigrations,
      );
      set({ state, saving: false });
      return state;
    } catch (error) {
      set({ saving: false, error: String(error) });
      throw error;
    }
  },
  runTask: async (taskId, runId) => {
    set({ runningTaskId: taskId, error: null });
    try {
      const result = await codexWakeupService.runCodexWakeupTask(taskId, runId);
      const [state, runtime] = await Promise.all([
        codexWakeupService.getCodexWakeupState(),
        codexWakeupService.getCodexWakeupCliStatus(),
      ]);
      set({ state, runtime, runningTaskId: null });
      return result;
    } catch (error) {
      set({ runningTaskId: null, error: String(error) });
      throw error;
    }
  },
  runTest: async (
    accountIds,
    runId,
    prompt,
    model,
    modelDisplayName,
    modelReasoningEffort,
    cancelScopeId,
  ) => {
    set({ testing: true, error: null });
    try {
      const result = await codexWakeupService.testCodexWakeup(
        accountIds,
        runId,
        prompt,
        model,
        modelDisplayName,
        modelReasoningEffort,
        cancelScopeId,
      );
      const runtime = await codexWakeupService.getCodexWakeupCliStatus();
      set({ runtime, testing: false });
      return result;
    } catch (error) {
      set({ testing: false, error: String(error) });
      throw error;
    }
  },
  cancelTestScope: async (cancelScopeId) => {
    await codexWakeupService.cancelCodexWakeupScope(cancelScopeId);
  },
  releaseTestScope: async (cancelScopeId) => {
    await codexWakeupService.releaseCodexWakeupScope(cancelScopeId);
  },
  clearHistory: async () => {
    set({ error: null });
    await codexWakeupService.clearCodexWakeupHistory();
  },
}));
