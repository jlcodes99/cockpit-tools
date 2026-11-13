import { create } from 'zustand';
import { previewDurableUiSetting, readDurableUiSetting } from '../utils/durableUiSettings';
import {
  markUserMemoryDismissed,
  USER_MEMORY_FLAGS,
} from '../utils/userMemory';

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

const MODE = 'agtools.side_nav.layout.v1';
const COLLAPSED = 'agtools.side_nav.classic_collapsed.v1';
const HIDE_PROMPT = 'agtools.side_nav.hide_classic_switch_prompt.v1';
const FIRST_SYNC = 'agtools.side_nav.classic_first_sync_done.v1';

function savedLayout() {
  const mode: SideNavLayoutMode = readDurableUiSetting(MODE) === 'original' ? 'original' : 'classic';
  return {
    mode,
    classicCollapsed: readDurableUiSetting(COLLAPSED) === '1',
    hideClassicSwitchPrompt: readDurableUiSetting(HIDE_PROMPT) === '1',
    classicFirstSyncDone: readDurableUiSetting(FIRST_SYNC) === '1',
  };
}

export const useSideNavLayoutStore = create<SideNavLayoutState>((set, get) => ({
  ...savedLayout(),
  setMode(mode) {
    set({ mode });
    previewDurableUiSetting(MODE, mode);
  },
  setClassicCollapsed(classicCollapsed) {
    set({ classicCollapsed });
    previewDurableUiSetting(COLLAPSED, classicCollapsed ? '1' : '0');
  },
  toggleClassicCollapsed() {
    get().setClassicCollapsed(!get().classicCollapsed);
  },
  setHideClassicSwitchPrompt(hideClassicSwitchPrompt) {
    if (hideClassicSwitchPrompt) {
      void markUserMemoryDismissed(USER_MEMORY_FLAGS.classicSwitchPrompt);
    }
    set({ hideClassicSwitchPrompt });
    previewDurableUiSetting(HIDE_PROMPT, hideClassicSwitchPrompt ? '1' : '0');
  },
  markClassicFirstSyncDone() {
    set({ classicFirstSyncDone: true });
    previewDurableUiSetting(FIRST_SYNC, '1');
  },
}));

export function reloadSideNavPreferences(): void {
  useSideNavLayoutStore.setState(savedLayout());
}
