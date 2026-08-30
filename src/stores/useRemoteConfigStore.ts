import { create } from 'zustand';
import type { PlatformId } from '../types/platform';
import type { RemoteConfigState } from '../types/remoteConfig';
import {
  forceRefreshRemoteConfigState,
  getRemoteConfigState,
} from '../services/remoteConfigService';

const DEFAULT_REFRESH_INTERVAL_MS = 60 * 60 * 1000;

const EMPTY_STATE: RemoteConfigState = {
  version: '',
  codexOAuthAppVersion: '26.820.60940',
  updatedAt: 0,
  currentOs: '',
  hiddenPlatformIds: [],
  appliedRules: [],
  refreshIntervalMs: DEFAULT_REFRESH_INTERVAL_MS,
  updatePromptMode: 'normal',
};

interface RemoteConfigStoreState {
  state: RemoteConfigState;
  hiddenPlatformIds: PlatformId[];
  loading: boolean;
  initialized: boolean;
  lastError: string | null;
  fetchState: (force?: boolean) => Promise<RemoteConfigState>;
}

export const useRemoteConfigStore = create<RemoteConfigStoreState>((set) => ({
  state: EMPTY_STATE,
  hiddenPlatformIds: [],
  loading: false,
  initialized: false,
  lastError: null,

  fetchState: async (force = false) => {
    set({ loading: true });
    try {
      const nextState = force
        ? await forceRefreshRemoteConfigState()
        : await getRemoteConfigState();
      set({
        state: nextState,
        hiddenPlatformIds: nextState.hiddenPlatformIds,
        loading: false,
        initialized: true,
        lastError: null,
      });
      return nextState;
    } catch (error) {
      console.error('加载远端配置失败:', error);
      set((current) => ({
        loading: false,
        initialized: true,
        lastError: String(error),
        hiddenPlatformIds: current.state.hiddenPlatformIds,
      }));
      return EMPTY_STATE;
    }
  },
}));
