import { create } from 'zustand';
import type { TopRightAdState } from '../types/topRightAd';
import { getTopRightAdState } from '../services/topRightAdService';

const EMPTY_STATE: TopRightAdState = {
  ad: null,
  ads: [],
};

const TOP_RIGHT_AD_STATE_CACHE_KEY = 'agtools.top_right_ad_state.cache.v1';

interface TopRightAdStoreState {
  state: TopRightAdState;
  loading: boolean;
  initialized: boolean;
  fetchState: () => Promise<TopRightAdState>;
}

function isTopRightAdState(value: unknown): value is TopRightAdState {
  if (!value || typeof value !== 'object') {
    return false;
  }
  const record = value as Partial<TopRightAdState>;
  return Array.isArray(record.ads);
}

function normalizeTopRightAdState(state: TopRightAdState): TopRightAdState {
  return {
    ad: state.ad ?? state.ads?.[0] ?? null,
    ads: Array.isArray(state.ads) ? state.ads : [],
  };
}

function loadCachedTopRightAdState(): TopRightAdState {
  if (typeof localStorage === 'undefined') {
    return EMPTY_STATE;
  }

  try {
    const raw = localStorage.getItem(TOP_RIGHT_AD_STATE_CACHE_KEY);
    if (!raw) return EMPTY_STATE;
    const parsed = JSON.parse(raw) as { state?: unknown };
    return isTopRightAdState(parsed.state)
      ? normalizeTopRightAdState(parsed.state)
      : EMPTY_STATE;
  } catch {
    return EMPTY_STATE;
  }
}

function persistTopRightAdState(state: TopRightAdState): void {
  if (typeof localStorage === 'undefined') {
    return;
  }

  try {
    localStorage.setItem(
      TOP_RIGHT_AD_STATE_CACHE_KEY,
      JSON.stringify({ savedAt: Date.now(), state: normalizeTopRightAdState(state) }),
    );
  } catch {
    // 缓存写入失败不影响主流程。
  }
}

const initialTopRightAdState = loadCachedTopRightAdState();

export const useTopRightAdStore = create<TopRightAdStoreState>((set, get) => ({
  state: initialTopRightAdState,
  loading: false,
  initialized: initialTopRightAdState.ads.length > 0,

  fetchState: async () => {
    set({ loading: true });
    try {
      const nextState = normalizeTopRightAdState(await getTopRightAdState());
      set({ state: nextState, loading: false, initialized: true });
      persistTopRightAdState(nextState);
      return nextState;
    } catch (error) {
      console.error('加载右上角广告位失败:', error);
      const currentState = get().state;
      set({ state: currentState, loading: false, initialized: true });
      return currentState;
    }
  },
}));
