import { create } from 'zustand';
import { DEFAULT_APP_BACKGROUND, normalizeAppBackground, type AppBackgroundSettings } from '../utils/appBackground';
import { loadAppBackground, saveAppBackground, normalizeBackgroundRecord, type AppBackgroundRecord } from '../services/appBackgroundService';

interface BackgroundState {
  record: AppBackgroundRecord;
  imageUrl: string | null;
  loaded: boolean;
  loading: boolean;
  saving: boolean;
  error: boolean;
  saveError: boolean;
  load: () => Promise<void>;
  update: (patch: Partial<AppBackgroundSettings>, options?: { image?: Blob | null; debounce?: boolean }) => void;
  addSkin: (name: string, image: Blob) => void;
  selectSkin: (id: string) => void;
  replaceSkin: (id: string, image: Blob) => void;
  renameSkin: (id: string, name: string) => void;
  deleteSkin: (id: string) => void;
  flush: () => Promise<void>;
  retrySave: () => void;
}

export function createAppBackgroundStore(storage = { load: loadAppBackground, save: saveAppBackground },
  urls = { create: (blob: Blob) => URL.createObjectURL(blob), revoke: (url: string) => URL.revokeObjectURL(url) }) {
  let loading: Promise<void> | null = null;
  let writing: Promise<void> | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let revision = 0;
  let savedRevision = 0;
  return create<BackgroundState>((set, get) => {
    const apply = (record: AppBackgroundRecord) => {
      if (record.image === get().record.image) { set({ record }); return; }
      const imageUrl = record.image ? urls.create(record.image) : null;
      const previous = get().imageUrl;
      set({ record, imageUrl });
      if (previous) urls.revoke(previous);
    };
    const flush = (): Promise<void> => {
      clearTimeout(timer);
      if (writing) return writing;
      if (revision === savedRevision || get().saveError) return Promise.resolve();
      writing = (async () => {
        while (savedRevision < revision) {
          const version = revision;
          const record = get().record;
          try { await storage.save(record); }
          catch {
            // Keep the current preview; the last successful disk record remains intact.
            // A newer choice can still be saved if the earlier write failed.
            if (version !== revision) continue;
            set({ saveError: true });
            return;
          }
          savedRevision = version;
        }
      })().finally(() => {
        writing = null;
        const pending = savedRevision < revision && !get().saveError;
        set({ saving: pending });
        if (pending) timer = setTimeout(() => { void flush(); }, 200);
      });
      return writing;
    };
    const commit = (record: AppBackgroundRecord, debounce = false) => {
      revision++;
      apply(record);
      set({ saving: true, saveError: false });
      clearTimeout(timer);
      if (debounce) timer = setTimeout(() => { void flush(); }, 200);
      else void flush();
    };
    return {
      record: { settings: { ...DEFAULT_APP_BACKGROUND }, image: null, skins: [], activeSkinId: null },
      imageUrl: null, loaded: false, loading: false, saving: false, error: false, saveError: false,
      load: () => {
        if (get().loaded) return Promise.resolve();
        if (loading) return loading;
        set({ loading: true, error: false });
        loading = storage.load().then(record => { apply(normalizeBackgroundRecord(record)); set({ loaded: true }); })
          .catch(() => { set({ error: true }); })
          .finally(() => { loading = null; set({ loading: false }); });
        return loading;
      },
      update: (patch, options = {}) => {
        if (!get().loaded) return;
        const record = get().record;
        const settings = normalizeAppBackground({ ...record.settings, ...patch });
        const next = { ...record, settings, image: options.image === undefined ? record.image : options.image };
        if (next.settings.preset === 'custom' && !next.image) throw new Error('imageRead');
        if (settings.preset === 'custom') {
          const id = record.activeSkinId ?? crypto.randomUUID();
          const skins = [...record.skins ?? []];
          const index = skins.findIndex(skin => skin.id === id);
          const skin = { id, name: index < 0 ? '' : skins[index].name, image: next.image!, settings };
          if (index < 0) skins.push(skin); else skins[index] = skin;
          commit({ ...next, skins, activeSkinId: id }, options.debounce);
        } else commit({ ...next, image: null, activeSkinId: null }, options.debounce);
      },
      addSkin: (name, image) => {
        if (!get().loaded) return;
        const record = get().record;
        const settings = { ...DEFAULT_APP_BACKGROUND, preset: 'custom' as const };
        const skin = { id: crypto.randomUUID(), name: name.trim(), image, settings };
        commit({ settings, image, skins: [...record.skins ?? [], skin], activeSkinId: skin.id });
      },
      selectSkin: id => {
        if (!get().loaded) return;
        const record = get().record;
        const skin = record.skins?.find(skin => skin.id === id);
        if (skin) commit({ ...record, settings: skin.settings, image: skin.image, activeSkinId: id });
      },
      replaceSkin: (id, image) => {
        if (!get().loaded) return;
        const record = get().record;
        if (!record.skins?.some(skin => skin.id === id)) return;
        commit({ ...record, image: record.activeSkinId === id ? image : record.image,
          skins: record.skins.map(skin => skin.id === id ? { ...skin, image } : skin) });
      },
      renameSkin: (id, name) => {
        if (!get().loaded || !name.trim()) return;
        const record = get().record;
        commit({ ...record, skins: record.skins?.map(skin => skin.id === id ? { ...skin, name: name.trim() } : skin) });
      },
      deleteSkin: id => {
        if (!get().loaded) return;
        const record = get().record;
        const reset = record.activeSkinId === id;
        commit({ ...record, skins: record.skins?.filter(skin => skin.id !== id),
          ...(reset ? { settings: { ...DEFAULT_APP_BACKGROUND }, image: null, activeSkinId: null } : {}) });
      },
      flush,
      retrySave: () => {
        if (savedRevision === revision) return;
        set({ saving: true, saveError: false }); void flush();
      },
    };
  });
}

export const useAppBackgroundStore = createAppBackgroundStore();
