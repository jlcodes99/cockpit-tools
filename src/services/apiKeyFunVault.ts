import { invoke, isTauri } from '@tauri-apps/api/core';

export type ManagedApiKey = {
  id: string;
  key: string;
  name: string;
  createdAt: number;
  lastUsedAt: number;
  lastStatus?: 'ok' | 'bad' | 'unknown';
  lastRemaining?: string;
  addedToCodexAt?: number;
  codexProviderName?: string;
  addedToClaudeAt?: number;
  claudeAccountName?: string;
};

interface VaultSnapshot {
  version: number;
  revision: number;
  keys: ManagedApiKey[];
}

const LEGACY_KEY = 'apikey_fun_managed_keys';

let current: VaultSnapshot | undefined;
let visibleKeys: ManagedApiKey[] = [];
let initialization: Promise<VaultSnapshot> | undefined;
let restoration: Promise<void> | undefined;
let writes: Promise<unknown> = Promise.resolve();
const listeners = new Set<(keys: ManagedApiKey[]) => void>();

function publish(keys: ManagedApiKey[]) {
  visibleKeys = keys;
  listeners.forEach(listener => listener(keys));
}

export function subscribeApiKeyFunVault(listener: (keys: ManagedApiKey[]) => void) {
  listeners.add(listener);
  listener(visibleKeys);
  return () => { listeners.delete(listener); };
}

async function readLegacyKeys(): Promise<ManagedApiKey[]> {
  const raw = localStorage.getItem(LEGACY_KEY);
  const parsed: unknown = raw !== null ? JSON.parse(raw)
    : globalThis.location?.origin === 'http://localhost:5173'
      ? await invoke<unknown[] | null>('read_legacy_webkit_vault', { vault: 'apikeyfun' }) ?? []
      : [];
  if (!Array.isArray(parsed)) throw new Error('apikey_fun_vault_legacy_invalid');
  const keys = parsed.filter((item): item is ManagedApiKey => (
    typeof item?.id === 'string' &&
    typeof item?.key === 'string' &&
    typeof item?.name === 'string' &&
    Number.isSafeInteger(item?.createdAt) && item.createdAt >= 0 &&
    Number.isSafeInteger(item?.lastUsedAt) && item.lastUsedAt >= 0
  ));
  if (parsed.length && !keys.length) throw new Error('apikey_fun_vault_legacy_invalid');
  return keys;
}

export function requiresApiKeyFunRecovery(error: unknown): boolean {
  return String(error).includes('apikey_fun_vault_checkpoint_required');
}

function loadVault() {
  return invoke<VaultSnapshot | null>('load_apikey_fun_vault').catch(error => {
    if (requiresApiKeyFunRecovery(error)) current = undefined;
    throw error;
  });
}

function writeVault(expectedRevision: number | null, keys: ManagedApiKey[]) {
  return invoke<VaultSnapshot>('write_apikey_fun_vault', { expectedRevision, keys });
}

async function initialize() {
  if (current) return current;
  if (initialization) return initialization;
  initialization = (async () => {
    let snapshot = await loadVault();
    if (!snapshot) {
      const legacy = await readLegacyKeys();
      publish(legacy);
      // The Rust command initializes only if the vault is still absent.
      snapshot = await writeVault(null, legacy);
    } else {
      void invoke('schedule_apikey_fun_recovery_checkpoint')
        .catch(error => console.warn('[APIKEY.FUN] Could not schedule a protected backup', error));
    }
    current = snapshot;
    publish(snapshot.keys);
    return snapshot;
  })();
  try {
    return await initialization;
  } finally {
    initialization = undefined;
  }
}

export async function initializeApiKeyFunVault(): Promise<void> {
  if (isTauri()) await initialize();
}

export function restoreApiKeyFunVault(): Promise<void> {
  if (restoration) return restoration;
  restoration = writes.then(async () => {
    const snapshot = await invoke<VaultSnapshot>('restore_apikey_fun_vault');
    current = snapshot;
    publish(snapshot.keys);
  }).finally(() => { restoration = undefined; });
  writes = restoration.catch(() => undefined);
  return restoration;
}

export function exportLegacyApiKeyFunVault(path: string): Promise<void> {
  return invoke('export_legacy_apikey_fun_vault', { path });
}

export function updateApiKeyFunVault(
  update: (keys: ManagedApiKey[]) => ManagedApiKey[],
): Promise<void> {
  const pending = writes.then(async () => {
    await initialize();
    const before = await loadVault();
    if (!before) throw new Error('apikey_fun_vault_missing');
    const next = update(before.keys);
    if (next.length === before.keys.length && next.every((item, i) => item === before.keys[i])) return;
    const saved = await writeVault(before.revision, next);
    current = saved;
    publish(saved.keys);
  });
  writes = pending.catch(() => undefined);
  return pending;
}
