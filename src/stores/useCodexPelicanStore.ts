import { create } from 'zustand';
import type { CodexPelicanBatch, CodexPelicanRequest } from '../types/codexPelican';
import * as service from '../services/codexPelicanService';
import { mergePelicanSnapshot } from '../components/codex/pelican/pelicanState';

type View = 'setup' | 'results' | 'history';
interface PelicanState {
  visible: boolean;
  view: View;
  active: CodexPelicanBatch | null;
  batch: CodexPelicanBatch | null;
  selectedAccountIds: string[];
  dismissedIds: Set<string>;
  starting: boolean;
  open: (accountIds?: string[]) => void;
  show: (view: View) => void;
  minimize: () => void;
  receive: (batch: CodexPelicanBatch, activate?: boolean) => void;
  start: (request: CodexPelicanRequest) => Promise<void>;
  showBatch: (batch: CodexPelicanBatch) => void;
  dismiss: (batchId: string) => Promise<void>;
}

export const useCodexPelicanStore = create<PelicanState>((set, get) => ({
  visible: false,
  view: 'setup',
  active: null,
  batch: null,
  selectedAccountIds: [],
  dismissedIds: new Set(),
  starting: false,
  open: (accountIds) => {
    const active = get().active;
    set({ visible: true, view: active ? 'results' : 'setup', batch: active,
      ...(accountIds ? { selectedAccountIds: accountIds } : {}) });
  },
  show: (view) => set({ visible: true, view, ...(view === 'results' ? { batch: get().active } : {}) }),
  minimize: () => set({ visible: false }),
  receive: (batch, activate = true) => set((state) => {
    return {
      ...(activate ? { active: mergePelicanSnapshot(state.active, batch, state.dismissedIds) } : {}),
      ...(state.batch?.id === batch.id && batch.revision >= state.batch.revision ? { batch } : {}),
    };
  }),
  start: async (request) => {
    if (get().starting) return;
    set({ starting: true });
    try {
      const batch = await service.startPelican(request);
      get().receive(batch);
      set({ visible: true, view: 'results', batch: get().active ?? batch });
    } finally { set({ starting: false }); }
  },
  showBatch: (batch) => {
    const active = get().active;
    set({ view: 'results', visible: true, batch: active?.id === batch.id && active.revision > batch.revision ? active : batch });
  },
  dismiss: async (batchId) => {
    await service.dismissPelican(batchId);
    set((state) => ({
      dismissedIds: new Set([...state.dismissedIds, batchId]),
      active: state.active?.id === batchId ? null : state.active,
      batch: state.batch?.id === batchId ? null : state.batch,
      visible: false,
    }));
  },
}));
