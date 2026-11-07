import { create } from 'zustand';
import { CODEX_PELICAN_DEFAULT_MODEL, CODEX_PELICAN_DEFAULT_PROMPT, isPelicanRunning, type CodexPelicanBatch, type CodexPelicanRequest } from '../types/codexPelican';
import * as service from '../services/codexPelicanService';
import { mergePelicanSnapshot } from '../components/codex/pelican/pelicanState';
import type { CodexPelicanProviderTarget } from '../types/codexPelican';

type View = 'setup' | 'results' | 'history';
interface PelicanState {
  visible: boolean;
  view: View;
  active: CodexPelicanBatch | null;
  activatedBatchId: string | null;
  batch: CodexPelicanBatch | null;
  selectedAccountIds: string[];
  sourceMode: 'accounts' | 'providers';
  providerTargets: CodexPelicanProviderTarget[];
  draft: Omit<CodexPelicanRequest, 'accountIds' | 'providerTargets'>;
  dismissedIds: Set<string>;
  starting: boolean;
  startSnapshots: Map<string, CodexPelicanBatch>;
  open: (accountIds?: string[]) => void;
  openProviders: (targets: CodexPelicanProviderTarget[]) => void;
  setProviderTargets: (targets: CodexPelicanProviderTarget[]) => void;
  updateDraft: (draft: Partial<PelicanState['draft']>) => void;
  editBatch: (batch: CodexPelicanBatch) => void;
  show: (view: View) => void;
  minimize: () => void;
  receive: (batch: CodexPelicanBatch, activate?: boolean) => void;
  resumeBatch: (batch: CodexPelicanBatch) => void;
  start: (request: CodexPelicanRequest) => Promise<void>;
  showBatch: (batch: CodexPelicanBatch) => void;
  dismiss: (batchId: string) => Promise<void>;
}

export const useCodexPelicanStore = create<PelicanState>((set, get) => ({
  visible: false,
  view: 'setup',
  active: null,
  activatedBatchId: null,
  batch: null,
  selectedAccountIds: [],
  sourceMode: 'accounts',
  providerTargets: [],
  draft: { prompt: CODEX_PELICAN_DEFAULT_PROMPT, model: CODEX_PELICAN_DEFAULT_MODEL, effort: 'medium', concurrency: 3 },
  dismissedIds: new Set(),
  starting: false,
  startSnapshots: new Map(),
  open: (accountIds) => {
    const active = get().active;
    const showResults = !!active && (accountIds === undefined || isPelicanRunning(active) || get().starting);
    set({ visible: true, view: showResults ? 'results' : 'setup', batch: showResults ? active : null,
      ...(!showResults && accountIds ? { selectedAccountIds: accountIds, sourceMode: 'accounts' as const } : {}) });
  },
  openProviders: (targets) => {
    if (get().starting || isPelicanRunning(get().active)) {
      set({ visible: true, view: get().active ? 'results' : get().view, batch: get().active });
      return;
    }
    set({ visible: true, view: 'setup', batch: null, sourceMode: 'providers', providerTargets: targets.map((target) => ({ ...target })) });
  },
  setProviderTargets: (providerTargets) => set({ providerTargets }),
  updateDraft: (draft) => set((state) => ({ draft: { ...state.draft, ...draft } })),
  editBatch: (batch) => {
    if (get().starting || isPelicanRunning(get().active) || isPelicanRunning(batch)) return;
    set({
      visible: true, view: 'setup', batch: null,
      selectedAccountIds: [...new Set(batch.items.map((item) => item.accountId))],
      sourceMode: batch.items.some((item) => item.provider) ? 'providers' : 'accounts',
      providerTargets: batch.items.flatMap((item) => item.provider ? [{ providerId: item.provider.providerId, apiKeyId: item.provider.apiKeyId, model: item.provider.model }] : []),
      draft: { prompt: batch.prompt, model: batch.model, effort: batch.effort, concurrency: batch.concurrency },
    });
  },
  show: (view) => set({ visible: true, view, ...(view === 'results' ? { batch: get().active } : {}) }),
  minimize: () => set({ visible: false }),
  receive: (batch, activate = true) => set((state) => {
    // A new batch can stream before its start invoke returns. Buffer those
    // revisions without letting them steal activation from the current batch.
    const pending = state.startSnapshots.get(batch.id);
    const startSnapshots = state.starting && (!pending || batch.revision >= pending.revision)
      ? new Map(state.startSnapshots).set(batch.id, batch) : state.startSnapshots;
    return {
      startSnapshots,
      ...(activate ? { active: mergePelicanSnapshot(state.active, batch, state.dismissedIds,
        state.active?.id === state.activatedBatchId ? state.activatedBatchId : null) } : {}),
      ...(state.batch?.id === batch.id && batch.revision >= state.batch.revision ? { batch } : {}),
    };
  }),
  resumeBatch: (batch) => set((state) => {
    const latest = [state.active, state.batch, state.startSnapshots.get(batch.id)].reduce<CodexPelicanBatch>((result, candidate) =>
      candidate?.id === batch.id && candidate.revision > result.revision ? candidate : result, batch);
    const dismissedIds = new Set(state.dismissedIds);
    dismissedIds.delete(batch.id);
    return { active: latest, activatedBatchId: batch.id, batch: latest, dismissedIds, visible: true, view: 'results' };
  }),
  start: async (request) => {
    if (get().starting) return;
    set({ starting: true, startSnapshots: new Map() });
    try {
      const batch = await service.startPelican(request);
      get().resumeBatch(batch);
    } finally { set({ starting: false, startSnapshots: new Map() }); }
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
      activatedBatchId: state.activatedBatchId === batchId ? null : state.activatedBatchId,
      batch: state.batch?.id === batchId ? null : state.batch,
      visible: false,
    }));
  },
}));
