import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule } from '../../tests/helpers/reactHookHarness';
import type { ManagedApiKey } from './apiKeyFunVault';

const legacyKey = 'apikey_fun_managed_keys';
const fixture: ManagedApiKey = {
  id: 'fixture', key: 'example-only', name: 'Original', createdAt: 1, lastUsedAt: 2,
};

function setup(checkpoint?: ManagedApiKey[]) {
  const old = new Map([[legacyKey, JSON.stringify([fixture])]]);
  let disk: { version: number; revision: number; keys: ManagedApiKey[] } | null = null;
  let writes = 0;
  let restores = 0;
  let legacyReads = 0;
  let onWrite: (() => Promise<void>) | undefined;
  const session = () => loadHookModule(new URL('./apiKeyFunVault.ts', import.meta.url), {
    '@tauri-apps/api/core': {
      isTauri: () => true,
      async invoke(command: string, args?: { expectedRevision: number | null; keys: ManagedApiKey[] }) {
        if (command === 'schedule_apikey_fun_recovery_checkpoint') return null;
        if (command === 'load_apikey_fun_vault') {
          if (!disk && checkpoint) throw new Error('apikey_fun_vault_checkpoint_required');
          return structuredClone(disk);
        }
        if (command === 'restore_apikey_fun_vault') {
          assert.ok(!disk && checkpoint);
          restores++;
          disk = { version: 1, revision: 5, keys: structuredClone(checkpoint) };
          return structuredClone(disk);
        }
        assert.equal(command, 'write_apikey_fun_vault');
        assert.ok(args);
        writes++;
        await onWrite?.();
        if (args.expectedRevision === null && disk) return structuredClone(disk);
        assert.equal(args.expectedRevision, disk?.revision ?? null);
        disk = { version: 1, revision: disk ? disk.revision + 1 : 0, keys: structuredClone(args.keys) };
        return structuredClone(disk);
      },
    },
  }, { localStorage: { getItem: (key: string) => { legacyReads++; return old.get(key) ?? null; } } }).exports;
  return {
    old, session, disk: () => disk, writes: () => writes, restores: () => restores,
    legacyReads: () => legacyReads, interceptWrite: (cb?: () => Promise<void>) => { onWrite = cb; },
  };
}

test('missing vault recovery requires an explicit restore and keeps a cleared backup empty', async () => {
  const s = setup([]);
  const service = s.session();
  let visible: ManagedApiKey[] = [fixture];
  service.subscribeApiKeyFunVault((keys: ManagedApiKey[]) => { visible = keys; });
  await assert.rejects(service.initializeApiKeyFunVault(), /checkpoint_required/);
  assert.equal(s.legacyReads(), 0);
  assert.equal(s.writes(), 0);
  const restore = service.restoreApiKeyFunVault();
  assert.equal(service.restoreApiKeyFunVault(), restore);
  await restore;
  assert.equal(s.restores(), 1);
  assert.deepEqual(visible, []);
  await s.session().initializeApiKeyFunVault();
  assert.equal(s.legacyReads(), 0);
  assert.equal(s.old.get(legacyKey), JSON.stringify([fixture]));
});

test('old keys remain visible during one migration and an empty new vault stays authoritative', async () => {
  const s = setup();
  const service = s.session();
  const started = deferred<void>();
  const finish = deferred<void>();
  s.interceptWrite(async () => { started.resolve(); await finish.promise; });
  let visible: ManagedApiKey[] = [];
  service.subscribeApiKeyFunVault((keys: ManagedApiKey[]) => { visible = keys; });
  const first = service.initializeApiKeyFunVault();
  const second = service.initializeApiKeyFunVault();
  await started.promise;
  assert.equal(visible[0]?.id, fixture.id);
  assert.equal(s.writes(), 1);
  finish.resolve();
  await Promise.all([first, second]);
  assert.equal(s.disk()?.keys[0]?.key, fixture.key);

  await service.updateApiKeyFunVault(() => []);
  await s.session().initializeApiKeyFunVault();
  assert.equal(s.writes(), 2);
  assert.equal(s.disk()?.keys.length, 0);
  assert.equal(s.old.get(legacyKey), JSON.stringify([fixture]));
});

test('failed initialization preserves the old source and concurrent writes use the latest saved revision', async () => {
  const s = setup();
  const service = s.session();
  s.interceptWrite(async () => { throw new Error('disk full'); });
  await assert.rejects(service.initializeApiKeyFunVault(), /disk full/);
  assert.equal(s.disk(), null);
  assert.equal(s.old.get(legacyKey), JSON.stringify([fixture]));
  s.interceptWrite();

  await Promise.all([
    service.updateApiKeyFunVault((keys: ManagedApiKey[]) => keys.map(key => ({ ...key, name: 'Renamed' }))),
    service.updateApiKeyFunVault((keys: ManagedApiKey[]) => keys.map(key => ({ ...key, lastUsedAt: 10 }))),
  ]);
  assert.equal(s.disk()?.keys[0].name, 'Renamed');
  assert.equal(s.disk()?.keys[0].lastUsedAt, 10);
});
