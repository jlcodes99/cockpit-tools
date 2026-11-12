import { invoke, isTauri } from '@tauri-apps/api/core';
import {
  createMfaRecordId,
  dedupeMfaRecordsBySecret,
  getMfaOtpToken,
  hasLegacyMfaHistoryStorage,
  parseMfaCredentialInput,
  normalizeMfaRecords,
  readLegacyMfaHistoryRecords,
  readSavedMfaSnapshot,
  toMfaSecretIdentity,
  type MfaRecord,
} from '../utils/mfaVault';

export type MfaRecoveryStatus = 'idle' | 'running' | 'complete' | 'failed' | 'checkpoint-required' | 'restoring';
type VaultList = 'saved' | 'history';

interface Progress {
  revision: number;
  offset: number;
  complete: boolean;
  sourceId: string | null;
}

interface NativeSnapshot {
  progress: Progress;
  records: MfaRecord[];
  removed: string[];
}

const IMPORT_BATCH_ROWS = 32;
const IMPORT_BATCH_BYTES = 2 * 1024 * 1024;
const lists = ['saved', 'history'] as const;
const snapshots: Partial<Record<VaultList, NativeSnapshot>> = {};
const identities: Partial<Record<VaultList, Set<string>>> = {};
const legacy: Record<VaultList, MfaRecord[]> = { saved: [], history: [] };
const visible: Record<VaultList, MfaRecord[]> = { saved: [], history: [] };
const queues: Record<VaultList, Promise<unknown>> = {
  saved: Promise.resolve(),
  history: Promise.resolve(),
};

let status: MfaRecoveryStatus = 'idle';
let pending: Promise<void> | undefined;
let restoring: Promise<void> | undefined;
const loading: Partial<Record<VaultList, Promise<void>>> = {};
const recoveryListeners = new Set<(status: MfaRecoveryStatus) => void>();
const recordListeners = new Set<() => void>();

function publish(next: MfaRecoveryStatus) {
  status = next;
  recoveryListeners.forEach(listener => listener(next));
}

export function subscribeMfaRecovery(listener: (status: MfaRecoveryStatus) => void) {
  recoveryListeners.add(listener);
  listener(status);
  return () => { recoveryListeners.delete(listener); };
}

export function subscribeMfaVault(listener: () => void) {
  recordListeners.add(listener);
  listener();
  return () => { recordListeners.delete(listener); };
}

export function loadSavedMfaRecords(): MfaRecord[] {
  return visible.saved;
}

export function loadMfaHistoryRecords(): MfaRecord[] {
  return visible.history;
}

async function readVaultRecordsForBackup(list: VaultList): Promise<MfaRecord[] | undefined> {
  return queue(list, async () => {
    if (!snapshots[list]) {
      await loadVault(list);
    } else {
      installSnapshot(list, await readNative(list));
    }
    // Configuration import treats a present array as a complete replacement.
    // Committed rows can be shown while migration runs, but are not a backup yet.
    return snapshots[list]?.progress.complete ? visible[list] : undefined;
  });
}

export const readSavedMfaRecordsForBackup = () => readVaultRecordsForBackup('saved');
export const readMfaHistoryRecordsForBackup = () => readVaultRecordsForBackup('history');

function publishRecords(list: VaultList) {
  const snapshot = snapshots[list];
  if (!snapshot) return;
  if (snapshot.progress.complete) {
    visible[list] = dedupeMfaRecordsBySecret(snapshot.records);
  } else {
    const nativeIds = new Set(
      [...snapshot.records.map(record => record.secret), ...snapshot.removed].map(toMfaSecretIdentity),
    );
    const pendingRecords = legacy[list].filter(record => !nativeIds.has(toMfaSecretIdentity(record.secret)));
    visible[list] = dedupeMfaRecordsBySecret([...snapshot.records, ...pendingRecords]);
  }
  recordListeners.forEach(listener => listener());
}

function queue<T>(list: VaultList, work: () => Promise<T>) {
  const result = queues[list].then(work);
  queues[list] = result.catch(() => undefined);
  return result;
}

function readNative(list: VaultList) {
  return invoke<NativeSnapshot>('read_mfa_vault', { list }).catch(error => {
    if (status !== 'restoring' && String(error).includes('mfa_recovery_checkpoint_required')) {
      publish('checkpoint-required');
    }
    throw error;
  });
}

function installSnapshot(list: VaultList, snapshot: NativeSnapshot) {
  snapshots[list] = snapshot;
  identities[list] = new Set(
    [...snapshot.records.map(record => record.secret), ...snapshot.removed].map(toMfaSecretIdentity),
  );
  publishRecords(list);
}

async function loadVault(list: VaultList) {
  if (snapshots[list]) return;
  if (loading[list]) return loading[list];
  loading[list] = (async () => {
    const snapshot = await readNative(list);
    if (!snapshot.progress.complete) {
      try {
        legacy[list] = list === 'saved' ? readSavedMfaSnapshot().records : readLegacyMfaHistoryRecords();
      } catch {
        // The migration pass reports an unreadable source separately from the native vault.
      }
    }
    installSnapshot(list, snapshot);
  })();
  try {
    await loading[list];
  } finally {
    delete loading[list];
  }
}

async function readOldDevOrigin(list: VaultList) {
  if (globalThis.location?.origin !== 'http://localhost:5173') return null;
  return invoke<unknown[] | null>('read_legacy_webkit_vault', { vault: list });
}

async function readSource(list: VaultList): Promise<{ origin: string; records: MfaRecord[] }> {
  if (list === 'history') {
    const records = readLegacyMfaHistoryRecords();
    if (records.length || hasLegacyMfaHistoryStorage()) return { origin: 'history', records };
    return { origin: 'history-dev-origin', records: normalizeMfaRecords(await readOldDevOrigin(list) ?? []) };
  }
  const local = readSavedMfaSnapshot();
  if ((local.migrated && !local.unreadable) || local.records.length) {
    return { origin: 'saved', records: local.records };
  }
  const oldDev = await readOldDevOrigin(list);
  const oldRecords = normalizeMfaRecords(oldDev ?? []);
  if (oldRecords.length) return { origin: 'saved-dev-origin', records: oldRecords };
  const backup = await invoke<unknown[] | null>('find_mfa_recovery_records');
  const records = normalizeMfaRecords(backup ?? []);
  if (records.length === 0 && (local.unreadable || backup !== null)) {
    throw new Error('mfa_recovery_source_unreadable');
  }
  return { origin: 'backup', records };
}

function sourceId(origin: string, records: MfaRecord[]) {
  // A stable cursor is valid only for the same ordered source snapshot. The
  // fingerprint uses record timestamps and labels, never OTP secrets.
  let hash = 0x811c9dc5;
  for (const record of records) {
    const metadata = JSON.stringify([record.accountName, record.time, record.remark]);
    for (let index = 0; index < metadata.length; index++) {
      hash = Math.imul(hash ^ metadata.charCodeAt(index), 0x01000193);
    }
  }
  return `${origin}:${records.length}:${hash >>> 0}`;
}

function nextBatch(records: MfaRecord[], offset: number) {
  const encoder = new TextEncoder();
  const batch: MfaRecord[] = [];
  let bytes = 0;
  for (const record of records.slice(offset, offset + IMPORT_BATCH_ROWS)) {
    const size = encoder.encode(JSON.stringify(record)).length;
    if (bytes + size > IMPORT_BATCH_BYTES) break;
    batch.push(record);
    bytes += size;
  }
  if (!batch.length && offset < records.length) throw new Error('mfa_recovery_record_too_large');
  return batch;
}

function addImported(list: VaultList, snapshot: NativeSnapshot, batch: MfaRecord[]) {
  const existing = identities[list]!;
  for (const record of batch) {
    const identity = toMfaSecretIdentity(record.secret);
    if (!existing.has(identity)) {
      snapshot.records.push(record);
      existing.add(identity);
    }
  }
}

async function migrate(list: VaultList) {
  if (snapshots[list]?.progress.complete) return;
  const { origin, records } = await readSource(list);
  if (snapshots[list]?.progress.complete) return;
  const id = sourceId(origin, records);
  legacy[list] = records;
  publishRecords(list);

  let busySince: number | undefined;
  while (!snapshots[list]?.progress.complete) {
    const imported = await queue(list, async () => {
      const snapshot = snapshots[list]!;
      if (snapshot.progress.complete) return true;
      const offset = snapshot.progress.sourceId === id ? snapshot.progress.offset : 0;
      const batch = nextBatch(records, offset);
      const progress = await invoke<Progress | null>('import_mfa_vault_batch', {
        list, sourceId: id, offset, records: batch, finalBatch: offset + batch.length >= records.length,
      });
      if (!progress) return false;
      // A different window may have committed a batch or replaced the vault.
      if (progress.revision !== snapshot.progress.revision + 1 || progress.offset !== offset + batch.length) {
        installSnapshot(list, await readNative(list));
        return true;
      }
      addImported(list, snapshot, batch);
      snapshot.progress = progress;
      if (progress.complete) {
        snapshot.removed = [];
        legacy[list] = [];
        installSnapshot(list, snapshot);
      }
      return true;
    });
    busySince = imported ? undefined : busySince ?? Date.now();
    if (busySince !== undefined && Date.now() - busySince > 5000) {
      throw new Error('mfa_recovery_busy');
    }
    if (!snapshots[list]?.progress.complete) {
      await new Promise<void>(resolve => setTimeout(resolve, 150));
    }
  }
}

export function startMfaRecovery(): Promise<void> {
  if (restoring) return restoring;
  if (pending) return pending;
  if (!isTauri() || status === 'complete') return Promise.resolve();
  pending = Promise.resolve().then(async () => {
    publish('running');
    try {
      const reads = await Promise.allSettled(lists.map(loadVault));
      if (status === 'checkpoint-required') return;
      let failed = false;
      for (const [index, list] of lists.entries()) {
        if (reads[index].status === 'rejected') {
          failed = true;
          continue;
        }
        try {
          await migrate(list);
        } catch {
          failed = true;
        }
      }
      if (failed) throw new Error('mfa_recovery_incomplete');
      publish('complete');
      void invoke('create_mfa_recovery_checkpoint')
        .catch(error => console.warn('[2FA] Could not create a protected recovery checkpoint', error));
    } catch {
      console.warn('[2FA] Could not migrate saved credentials or query history');
      if (status !== 'checkpoint-required') publish('failed');
    }
  }).finally(() => { pending = undefined; });
  return pending;
}

export function restoreMfaRecoveryCheckpoint(): Promise<void> {
  if (restoring) return restoring;
  restoring = (async () => {
    await pending;
    publish('restoring');
    let installed = false;
    try {
      await queue('saved', () => queue('history', async () => {
        for (const list of lists) {
          delete snapshots[list];
          legacy[list] = [];
        }
        await invoke('restore_mfa_recovery_checkpoint');
        installed = true;
        const [saved, history] = await Promise.all(lists.map(readNative));
        installSnapshot('saved', saved);
        installSnapshot('history', history);
      }));
      publish('complete');
    } catch (error) {
      publish(installed || String(error).includes('mfa_recovery_live_vault_exists')
        ? 'failed' : 'checkpoint-required');
      throw error;
    }
  })().finally(() => { restoring = undefined; });
  return restoring;
}

function update(
  list: VaultList,
  change: (records: MfaRecord[]) => MfaRecord[],
  replace = false,
): Promise<MfaRecord[]> {
  return queue(list, async () => {
    await loadVault(list);
    const snapshot = snapshots[list]!;
    const current = new Map(visible[list].map(record => [toMfaSecretIdentity(record.secret), record]));
    const next = dedupeMfaRecordsBySecret(change([...visible[list]]));
    const nextIds = new Set(next.map(record => toMfaSecretIdentity(record.secret)));
    const remove = replace ? [] : [...current.entries()]
      .filter(([id]) => !nextIds.has(id)).map(([, record]) => record.secret);
    const upsert = replace ? next : next.filter(record => current.get(toMfaSecretIdentity(record.secret)) !== record);
    if (!replace && !remove.length && !upsert.length) return visible[list];

    let progress: Progress;
    try {
      progress = await invoke<Progress>('change_mfa_vault', {
        list, changes: { revision: snapshot.progress.revision, remove, upsert, replace },
      });
    } catch (error) {
      installSnapshot(list, await readNative(list));
      throw error;
    }
    const changedIds = new Set([...upsert.map(record => record.secret), ...remove].map(toMfaSecretIdentity));
    snapshot.records = replace ? upsert : [
      ...snapshot.records.filter(record => !changedIds.has(toMfaSecretIdentity(record.secret))),
      ...upsert,
    ];
    snapshot.removed = progress.complete ? [] : [
      ...snapshot.removed.filter(secret => !changedIds.has(toMfaSecretIdentity(secret))),
      ...remove,
    ];
    snapshot.progress = progress;
    if (progress.complete) legacy[list] = [];
    installSnapshot(list, snapshot);
    return visible[list];
  });
}

export function updateSavedMfaRecords(change: (records: MfaRecord[]) => MfaRecord[]) {
  return update('saved', change);
}

export function updateMfaHistoryRecords(change: (records: MfaRecord[]) => MfaRecord[]) {
  return update('history', change);
}

export function replaceSavedMfaRecords(records: MfaRecord[]) {
  return update('saved', () => records, true);
}

export function replaceMfaHistoryRecords(records: MfaRecord[]) {
  return update('history', () => records, true);
}

export function clearMfaHistoryRecords() {
  return replaceMfaHistoryRecords([]);
}


function recordCredential(
  list: VaultList,
  input: { secret: string; accountName?: string | null; remark?: string | null },
): Promise<MfaRecord[]> {
  const parsed = parseMfaCredentialInput(input.secret);
  if (!parsed || (list === 'history' && !getMfaOtpToken(input.secret))) {
    return Promise.resolve(visible[list]);
  }
  const identity = toMfaSecretIdentity(parsed.secret);
  return update(list, records => {
    const existing = records.find(record => toMfaSecretIdentity(record.secret) === identity);
    return [{
      id: existing?.id ?? createMfaRecordId(),
      secret: parsed.secret,
      accountName: input.accountName?.trim() || parsed.accountName || existing?.accountName || '',
      remark: input.remark?.trim() || existing?.remark || '',
      time: Date.now(),
    }, ...records.filter(record => record !== existing)];
  });
}

export function rememberMfaQuery(input: Parameters<typeof recordCredential>[1]) {
  return recordCredential('history', input);
}

export function upsertSavedMfaRecord(input: Parameters<typeof recordCredential>[1]) {
  return upsertSavedMfaRecords([input]);
}

export async function upsertSavedMfaRecords(inputs: Parameters<typeof recordCredential>[1][]) {
  for (let offset = 0; offset < inputs.length; offset += IMPORT_BATCH_ROWS) {
    const batch = inputs.slice(offset, offset + IMPORT_BATCH_ROWS);
    try {
      await update('saved', records => {
        const byIdentity = new Map(records.map(record => [toMfaSecretIdentity(record.secret), record]));
        for (const input of batch) {
          const parsed = parseMfaCredentialInput(input.secret);
          if (!parsed) continue;
          const identity = toMfaSecretIdentity(parsed.secret);
          const existing = byIdentity.get(identity);
          byIdentity.set(identity, {
            id: existing?.id ?? createMfaRecordId(),
            secret: parsed.secret,
            accountName: input.accountName?.trim() || parsed.accountName || existing?.accountName || '',
            remark: input.remark?.trim() || existing?.remark || '',
            time: Date.now(),
          });
        }
        return Array.from(byIdentity.values());
      });
    } catch {
      console.warn('[2FA] Could not copy imported credentials into the vault');
      break;
    }
  }
  return visible.saved;
}
