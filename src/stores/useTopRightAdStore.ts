import { create } from 'zustand';
import type { TopRightAdState } from '../types/topRightAd';
import { getTopRightAdState } from '../services/topRightAdService';

const EMPTY_STATE: TopRightAdState = {
  ad: null,
  ads: [],
};

interface TopRightAdStoreState {
  state: TopRightAdState;
  loading: boolean;
  initialized: boolean;
  fetchState: () => Promise<TopRightAdState>;
}

export const useTopRightAdStore = create<TopRightAdStoreState>((set) => ({
  state: EMPTY_STATE,
  loading: false,
  initialized: false,

  fetchState: async () => {
    set({ loading: true });
    try {
      const nextState = await getTopRightAdState();
      set({ state: nextState, loading: false, initialized: true });
      return nextState;
    } catch (error) {
      console.error('加载右上角广告位失败:', error);
      set({ state: EMPTY_STATE, loading: false, initialized: true });
      return EMPTY_STATE;
    }
  },
}));
