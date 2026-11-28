import { create } from 'zustand';
import { antigravityProxyApi, type AntigravityProxyView } from '../services/antigravityProxyService';

interface ProxyState {
  views: Record<string, AntigravityProxyView>;
  loading: boolean;
  failed: boolean;
  reload(): Promise<void>;
  accept(view: AntigravityProxyView): void;
}

/** One list read per page refresh, not per card; writes outrank older list reads. */
export function createAntigravityProxyStore(api: Pick<typeof antigravityProxyApi, 'list'> = antigravityProxyApi) {
  let revision = 0;
  const acceptedAt = new Map<string, number>();
  let pending: Promise<void> | null = null;
  return create<ProxyState>((set) => ({
    views: {}, loading: false, failed: false,
    accept(view) {
      acceptedAt.set(view.accountId, ++revision);
      set((state) => ({ views: { ...state.views, [view.accountId]: view } }));
    },
    reload() {
      if (pending) return pending;
      const generation = revision;
      set({ loading: true });
      pending = api.list().then((views) => {
        set((state) => {
          const next = Object.fromEntries(views.map((view) => [view.accountId, view]));
          for (const [id, at] of acceptedAt) {
            if (at > generation && state.views[id]) next[id] = state.views[id];
          }
          return { views: next, failed: false };
        });
      }).catch(() => {
        if (generation === revision) set({ failed: true });
      }).finally(() => { pending = null; set({ loading: false }); });
      return pending;
    },
  }));
}
export const useAntigravityProxyStore = createAntigravityProxyStore();
