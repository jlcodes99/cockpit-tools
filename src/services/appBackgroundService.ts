import { DEFAULT_APP_BACKGROUND, normalizeAppBackground, type AppBackgroundSettings } from '../utils/appBackground';

export interface AppBackgroundRecord {
  settings: AppBackgroundSettings;
  image: Blob | null;
  /** Optional only for records saved before the skin library was introduced. */
  skins?: BackgroundSkin[];
  activeSkinId?: string | null;
}

export interface BackgroundSkin {
  id: string;
  name: string;
  image: Blob;
  settings: AppBackgroundSettings;
}

/** Upgrade the old single-image record in memory; the next edit saves it atomically. */
export function normalizeBackgroundRecord(record: AppBackgroundRecord): AppBackgroundRecord {
  const settings = normalizeAppBackground(record.settings);
  const skins = (record.skins ?? []).map(skin => ({ ...skin, settings: normalizeAppBackground({ ...skin.settings, preset: 'custom' }) }));
  let activeSkinId = record.activeSkinId ?? null;
  if (!record.skins && record.image instanceof Blob) {
    skins.push({ id: 'legacy-custom', name: '', image: record.image, settings: { ...settings, preset: 'custom' } });
    if (settings.preset === 'custom') activeSkinId = 'legacy-custom';
  }
  const active = settings.preset === 'custom' ? skins.find(skin => skin.id === activeSkinId) : undefined;
  if (settings.preset === 'custom' && !active) settings.preset = 'default';
  return { settings: active ? active.settings : settings, image: active?.image ?? null, skins, activeSkinId: active?.id ?? null };
}

/** Backgrounds belong to this WebView's local storage, never account exports or network sync. */
async function database(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open('cockpit-appearance', 1);
    let expired = false;
    const fail = () => { expired = true; clearTimeout(timer); reject(new Error('storage')); };
    const timer = setTimeout(fail, 5000);
    request.onupgradeneeded = () => request.result.createObjectStore('background');
    request.onblocked = fail;
    request.onerror = fail;
    request.onsuccess = () => {
      clearTimeout(timer);
      if (expired) request.result.close();
      else resolve(request.result);
    };
  });
}

async function transaction(write?: AppBackgroundRecord): Promise<AppBackgroundRecord | undefined> {
  const db = await database();
  return new Promise((resolve, reject) => {
    const tx = db.transaction('background', write ? 'readwrite' : 'readonly');
    const store = tx.objectStore('background');
    const request = write ? store.put(write, 'current') : store.get('current');
    const timer = setTimeout(() => tx.abort(), 5000);
    const finish = () => { clearTimeout(timer); db.close(); };
    tx.oncomplete = () => { finish(); resolve(write ?? request.result); };
    tx.onabort = () => { finish(); reject(new Error('storage')); };
    tx.onerror = () => { /* onabort handles failed transactions without duplicate rejection. */ };
  });
}

export async function loadAppBackground(): Promise<AppBackgroundRecord> {
  const record = await transaction();
  return normalizeBackgroundRecord(record ?? { settings: { ...DEFAULT_APP_BACKGROUND }, image: null });
}

export async function saveAppBackground(record: AppBackgroundRecord): Promise<void> {
  await transaction(record);
}

/** Decode once on user selection, downsample locally and persist a static image without metadata. */
export async function prepareBackgroundImage(file: File): Promise<Blob> {
  if (!['image/jpeg', 'image/png', 'image/webp'].includes(file.type)) throw new Error('imageType');
  if (file.size > 10 * 1024 * 1024) throw new Error('imageSize');
  const url = URL.createObjectURL(file);
  try {
    const image = await new Promise<HTMLImageElement>((resolve, reject) => {
      const img = new Image();
      const timer = setTimeout(() => { img.src = ''; reject(new Error('imageRead')); }, 10000);
      img.onload = () => { clearTimeout(timer); resolve(img); };
      img.onerror = () => { clearTimeout(timer); reject(new Error('imageRead')); };
      img.src = url;
    });
    if (!image.naturalWidth || !image.naturalHeight) throw new Error('imageRead');
    const scale = Math.min(1, 2560 / Math.max(image.naturalWidth, image.naturalHeight));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(image.naturalWidth * scale));
    canvas.height = Math.max(1, Math.round(image.naturalHeight * scale));
    const context = canvas.getContext('2d');
    if (!context) throw new Error('imageRead');
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    return await new Promise<Blob>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('imageRead')), 10000);
      canvas.toBlob(blob => {
        clearTimeout(timer);
        if (blob) resolve(blob); else reject(new Error('imageRead'));
      }, 'image/webp', 0.88);
    });
  } finally {
    URL.revokeObjectURL(url);
  }
}
