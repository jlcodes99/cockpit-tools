import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';

export type SideNavLayoutMode = 'original' | 'classic';

export const ORIGINAL_SIDEBAR_ENTRY_LIMIT = 3;
export const CLASSIC_SIDEBAR_ENTRY_LIMIT = 6;

interface SideNavLayoutState {
  mode: SideNavLayoutMode;
  classicCollapsed: boolean;
  hideClassicSwitchPrompt: boolean;
  classicFirstSyncDone: boolean;
  setMode: (mode: SideNavLayoutMode) => void;
  setClassicCollapsed: (collapsed: boolean) => void;
  toggleClassicCollapsed: () => void;
  setHideClassicSwitchPrompt: (hidden: boolean) => void;
  markClassicFirstSyncDone: () => void;
}

// 兼容迁移旧版本的零散 localStorage 键值
function getOldStorage<T>(key: string, parse: (val: string) => T, fallback: T): T {
  if (typeof window === 'undefined') return fallback;
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return fallback;
    return parse(raw);
  } catch {
    return fallback;
  }
}

export const useSideNavLayoutStore = create<SideNavLayoutState>()(
  persist(
    (set) => ({
      mode: getOldStorage<SideNavLayoutMode>('agtools.side_nav.layout.v1', (v) => (v === 'classic' ? 'classic' : 'original'), 'original'),
      classicCollapsed: getOldStorage('agtools.side_nav.classic_collapsed.v1', (v) => v === '1', false),
      hideClassicSwitchPrompt: getOldStorage('agtools.side_nav.hide_classic_switch_prompt.v1', (v) => v === '1', false),
      classicFirstSyncDone: getOldStorage('agtools.side_nav.classic_first_sync_done.v1', (v) => v === '1', false),

      setMode: (mode) => set({ mode }),
      setClassicCollapsed: (classicCollapsed) => set({ classicCollapsed }),
      toggleClassicCollapsed: () => set((state) => ({ classicCollapsed: !state.classicCollapsed })),
      setHideClassicSwitchPrompt: (hideClassicSwitchPrompt) => set({ hideClassicSwitchPrompt }),
      markClassicFirstSyncDone: () => set({ classicFirstSyncDone: true }),
    }),
    {
      name: 'agtools.side_nav.store.v2',
      storage: createJSONStorage(() => localStorage),
      onRehydrateStorage: () => (state) => {
        // 在成功合并旧数据并写入新 JSON 结构后，清理残留的旧键
        if (state && typeof window !== 'undefined') {
          setTimeout(() => {
            localStorage.removeItem('agtools.side_nav.layout.v1');
            localStorage.removeItem('agtools.side_nav.classic_collapsed.v1');
            localStorage.removeItem('agtools.side_nav.hide_classic_switch_prompt.v1');
            localStorage.removeItem('agtools.side_nav.classic_first_sync_done.v1');
          }, 0);
        }
      },
    }
  )
);
