import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule } from '../../tests/helpers/reactHookHarness';
import { MFA_STORAGE_KEY_HISTORY, type MfaRecord } from '../utils/mfaVault';

const oldKey = 'agtools.mfa.vault.v2';
const olderKey = 'agtools.two_factor_auth.saved.v2';
const example = {
  id: 'backup-id', accountName: 'Example', secret: 'JBSWY3DPEHPK3PXP', remark: 'Work', time: 123,
};
const newer = { ...example, id: 'new-id', accountName: 'New', secret: 'GEZDGNBVGY3TQOJQ' };

function empty() {
  return {
    records: [] as MfaRecord[], removed: [] as string[],
    progress: { revision: 0, offset: 0, sourceId: null as string | null, complete: false },
  };
}

function setup(
  readBackup: () => Promise<unknown> = async () => [example],
  readCheckpoint?: () => Promise<{ saved: MfaRecord[]; history: MfaRecord[] }>,
) {
  const legacy = new Map<string, string>();
  const inaccessible = new Set<string>();
  const native = { saved: empty(), history: empty() };
  const localStorage = { getItem: (key: string) => {
    if (inaccessible.has(key)) throw new Error('WebView storage denied');
    return legacy.get(key) ?? null;
  } };
  let backupReads = 0;
  let checkpointRequests = 0;
  let missingVault = readCheckpoint !== undefined;
  let restoredCheckpoints = 0;
  let batches = 0;
  let failBatch: number | undefined;
  const vault = loadHookModule(new URL('../utils/mfaVault.ts', import.meta.url), {}, { localStorage }).exports;
  const freshSession = () => loadHookModule(new URL('./mfaRecoveryService.ts', import.meta.url), {
    '@tauri-apps/api/core': {
      isTauri: () => true,
      invoke: async (command: string, args: {
        list: 'saved' | 'history';
        records: MfaRecord[];
        sourceId: string;
        offset: number;
        finalBatch: boolean;
        changes: { revision: number; upsert: MfaRecord[]; remove: string[]; replace: boolean };
      }) => {
        if (command === 'create_mfa_recovery_checkpoint') {
          checkpointRequests++;
          return null;
        }
        if (command === 'find_mfa_recovery_records') {
          backupReads++;
          return readBackup();
        }
        if (command === 'restore_mfa_recovery_checkpoint') {
          assert.ok(readCheckpoint && missingVault);
          const records = await readCheckpoint();
          for (const list of ['saved', 'history'] as const) {
            native[list].records = records[list];
            native[list].progress = { revision: 3, offset: records[list].length, sourceId: 'fixture', complete: true };
          }
          missingVault = false;
          restoredCheckpoints++;
          return null;
        }
        if (missingVault) throw new Error('mfa_recovery_checkpoint_required');
        const snapshot = native[args.list];
        if (command === 'read_mfa_vault') return structuredClone(snapshot);
        if (command === 'import_mfa_vault_batch') {
          if (++batches === failBatch) throw new Error('simulated interruption');
          if (snapshot.progress.complete) return structuredClone(snapshot.progress);
          const existing = new Set([...snapshot.records.map(item => item.secret), ...snapshot.removed]);
          snapshot.records.push(...args.records.filter(record => !existing.has(record.secret)));
          snapshot.progress = {
            revision: snapshot.progress.revision + 1,
            sourceId: args.sourceId, offset: args.offset + args.records.length, complete: args.finalBatch,
          };
          if (args.finalBatch) snapshot.removed = [];
          return structuredClone(snapshot.progress);
        }
        assert.equal(command, 'change_mfa_vault');
        const { revision, upsert, remove, replace } = args.changes;
        assert.equal(snapshot.progress.revision, revision);
        const changed = new Set([...upsert.map(record => record.secret), ...remove]);
        snapshot.records = replace ? [...upsert] : [
          ...snapshot.records.filter(record => !changed.has(record.secret)), ...upsert,
        ];
        snapshot.progress.revision++;
        snapshot.progress.complete ||= replace;
        snapshot.removed = snapshot.progress.complete ? [] : [
          ...snapshot.removed.filter(secret => !changed.has(secret)), ...remove,
        ];
        return structuredClone(snapshot.progress);
      },
    },
    '../utils/mfaVault': vault,
  }, { TextEncoder, console: { warn() {} } }).exports;
  return {
    legacy, inaccessible, native, service: freshSession(), freshSession,
    reads: () => backupReads,
    checkpoints: () => checkpointRequests,
    restored: () => restoredCheckpoints,
    interruptBatch: (number: number) => { failBatch = number; },
  };
}

test('a missing native vault waits for explicit checkpoint recovery and reloads both lists', async () => {
  const checkpoint = deferred<{ saved: MfaRecord[]; history: MfaRecord[] }>();
  const s = setup(async () => [example], () => checkpoint.promise);
  s.legacy.set(oldKey, JSON.stringify([example]));
  const statuses: string[] = [];
  s.service.subscribeMfaRecovery((status: string) => statuses.push(status));
  await s.service.startMfaRecovery();
  assert.equal(statuses[statuses.length - 1], 'checkpoint-required');
  assert.equal(s.reads(), 0);
  assert.equal(s.checkpoints(), 0);
  assert.equal(s.restored(), 0);
  const restoring = s.service.restoreMfaRecoveryCheckpoint();
  assert.equal(s.service.restoreMfaRecoveryCheckpoint(), restoring);
  checkpoint.resolve({ saved: [newer], history: [example] });
  await restoring;
  assert.equal(statuses[statuses.length - 1], 'complete');
  assert.equal(s.restored(), 1);
  assert.equal(s.service.loadSavedMfaRecords()[0]?.id, newer.id);
  assert.equal(s.service.loadMfaHistoryRecords()[0]?.id, example.id);
  assert.equal(s.legacy.get(oldKey), JSON.stringify([example]));
});

test('upgrade migrates existing saved keys without changing the old source or consulting backup', async () => {
  const s = setup();
  s.legacy.set(oldKey, JSON.stringify([newer]));
  s.legacy.set(olderKey, JSON.stringify([example]));
  await s.service.startMfaRecovery();
  assert.equal(s.reads(), 0);
  assert.equal(s.checkpoints(), 1);
  assert.deepEqual(s.native.saved.records.map(record => record.id).sort(), ['backup-id', 'new-id']);
  assert.equal(s.legacy.get(oldKey), JSON.stringify([newer]));
});

test('empty old vault restores once and a later deletion stays deleted on restart', async () => {
  const s = setup();
  s.legacy.set(oldKey, '[]');
  const pending = s.service.startMfaRecovery();
  assert.equal(s.service.startMfaRecovery(), pending);
  await pending;
  assert.equal(s.native.saved.records[0]?.secret, example.secret);
  await s.service.updateSavedMfaRecords(() => []);
  await s.freshSession().startMfaRecovery();
  assert.equal(s.reads(), 1);
  assert.equal(s.native.saved.records.length, 0);
});

test('a WebView read error still allows automatic recovery from backup', async () => {
  const s = setup();
  s.legacy.set(oldKey, JSON.stringify([newer]));
  s.inaccessible.add(oldKey);
  await s.service.startMfaRecovery();
  assert.equal(s.reads(), 1);
  assert.equal(s.native.saved.progress.complete, true);
  assert.equal(s.native.saved.records[0]?.secret, example.secret);
  assert.equal(s.legacy.get(oldKey), JSON.stringify([newer]));
});

test('a save during slow backup recovery can finish before the backup and both records are kept', async () => {
  const backup = deferred<unknown>();
  const reading = deferred<void>();
  const s = setup(() => { reading.resolve(); return backup.promise; });
  const pending = s.service.startMfaRecovery();
  await reading.promise;
  await s.service.upsertSavedMfaRecord(newer);
  assert.equal(s.native.saved.records[0].secret, newer.secret);
  assert.equal(await s.service.readSavedMfaRecordsForBackup(), undefined);
  backup.resolve([example]);
  await pending;
  assert.equal(s.native.saved.records.length, 2);
  assert.deepEqual(Array.from(await s.service.readSavedMfaRecordsForBackup(), (record: MfaRecord) => record.secret).sort(),
    [example.secret, newer.secret].sort());
});

test('unreadable legacy data remains intact after failed recovery and the same session can retry', async () => {
  let attempts = 0;
  const s = setup(async () => {
    attempts++;
    if (attempts === 1) throw new Error('backup temporarily unavailable');
    return attempts === 2 ? null : [example];
  });
  s.legacy.set(oldKey, 'corrupt storage');
  const statuses: string[] = [];
  s.service.subscribeMfaRecovery((status: string) => statuses.push(status));
  await s.service.startMfaRecovery();
  await s.service.startMfaRecovery();
  assert.equal(s.native.saved.progress.complete, false);
  assert.equal(s.legacy.get(oldKey), 'corrupt storage');
  assert.deepEqual(statuses, ['idle', 'running', 'failed', 'running', 'failed']);
  await s.service.startMfaRecovery();
  assert.equal(s.native.saved.records[0]?.secret, example.secret);
  assert.equal(statuses[statuses.length - 1], 'complete');
});

test('history migration resumes at its last batch and does not evict query history', async () => {
  const s = setup(async () => null);
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';
  const history = Array.from({ length: 55 }, (_, i) => ({
    ...example,
    id: `history-${i}`,
    secret: `JBSWY3DPEHPK3P${alphabet[Math.floor(i / 32)]}${alphabet[i % 32]}`,
    time: i + 1,
  }));
  s.legacy.set(MFA_STORAGE_KEY_HISTORY, JSON.stringify(history));
  s.interruptBatch(3);
  await s.service.startMfaRecovery();
  assert.equal(s.native.history.progress.offset, 32);
  assert.equal(s.native.history.progress.complete, false);
  assert.equal(s.service.loadMfaHistoryRecords().length, 55);
  const fresh = s.freshSession();
  await fresh.startMfaRecovery();
  assert.equal(s.native.history.progress.complete, true);
  assert.equal(s.native.history.records.length, 55);
  await fresh.rememberMfaQuery(example);
  assert.equal(s.native.history.records.length, 56);
  await fresh.rememberMfaQuery(example);
  assert.equal(s.native.history.records.length, 56);
});
