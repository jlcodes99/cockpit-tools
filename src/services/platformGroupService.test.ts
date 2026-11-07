import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

type StoredGroup = {
  id: string;
  name: string;
  accountIds: string[];
  createdAt: number;
  sortOrder?: number;
  quotaAutoRefreshMinutes?: number | null;
  quotaRefreshEnabled?: boolean;
};

function group(id: string, overrides: Partial<StoredGroup> = {}): StoredGroup {
  return { id, name: id, accountIds: [`${id}-account`], createdAt: 100, ...overrides };
}

function harness(initial: Record<string, StoredGroup[]> = {}) {
  const disk = new Map(Object.entries(initial).map(([platform, groups]) => [platform, JSON.stringify(groups)]));
  const readErrors = new Map<string, Error>();
  const writeErrors = new Map<string, Error>();
  const writeGates = new Map<string, ReturnType<typeof deferred<void>>>();
  const writes: { platform: string; data: string }[] = [];
  const imports = {
    '@tauri-apps/api/core': {
      async invoke(command: string, args?: { platform?: string; data?: string }) {
        const platform = command.includes('codex_account_groups') ? 'codex'
          : command.includes('platform_account_groups') ? args?.platform : 'antigravity';
        assert.ok(platform, `Missing platform for ${command}`);
        if (command.startsWith('load_')) {
          const error = readErrors.get(platform);
          if (error) throw error;
          return disk.get(platform) ?? '[]';
        }
        assert.match(command, /^save_(?:platform_|codex_)?account_groups$/);
        assert.equal(typeof args?.data, 'string');
        const data = args!.data!;
        writes.push({ platform, data });
        const gate = writeGates.get(platform);
        if (gate) {
          writeGates.delete(platform);
          await gate.promise;
        }
        const error = writeErrors.get(platform);
        if (error) throw error;
        disk.set(platform, data);
      },
    },
  };
  const globals = { localStorage: { getItem: () => null, removeItem() {} }, console: { error() {}, log() {} } };
  const antigravity = loadHookModule(new URL('./accountGroupService.ts', import.meta.url), imports, globals)
    .exports as typeof import('./accountGroupService');
  const codex = loadHookModule(new URL('./codexAccountGroupService.ts', import.meta.url), imports, globals)
    .exports as typeof import('./codexAccountGroupService');
  const platform = loadHookModule(new URL('./platformGroupService.ts', import.meta.url), {
    ...imports,
    './accountGroupService': antigravity,
    './codexAccountGroupService': codex,
  }, globals).exports as typeof import('./platformGroupService');
  return {
    platform, antigravity, codex, disk, writes, readErrors, writeErrors, writeGates,
    stored(key: string): StoredGroup[] { return JSON.parse(disk.get(key) ?? '[]'); },
  };
}

test('common Codex edits retain quota policies and persist sort order for both entry points', async () => {
  const h = harness({ codex: [
    group('disabled', { sortOrder: 20, quotaAutoRefreshMinutes: -1 }),
    group('custom', { sortOrder: 10, quotaAutoRefreshMinutes: 37 }),
    group('legacy', { sortOrder: 30, quotaRefreshEnabled: false }),
  ] });
  assert.deepEqual(Array.from(await h.platform.getPlatformGroups('codex'), (g) => g.id), ['custom', 'disabled', 'legacy']);
  await h.platform.renamePlatformGroup('codex', 'disabled', 'Renamed');
  let stored = h.stored('codex');
  assert.equal(stored.find((g) => g.id === 'disabled')?.quotaAutoRefreshMinutes, -1);
  assert.equal(stored.find((g) => g.id === 'custom')?.quotaAutoRefreshMinutes, 37);
  assert.equal(stored.find((g) => g.id === 'legacy')?.quotaAutoRefreshMinutes, -1);
  assert.equal(stored.find((g) => g.id === 'disabled')?.sortOrder, 20);
  await h.platform.reorderPlatformGroups('codex', ['legacy', 'disabled', 'custom']);
  h.platform.invalidatePlatformGroupCache('codex');
  h.codex.invalidateCodexGroupCache();
  assert.deepEqual(Array.from(await h.codex.getCodexAccountGroups(), (g) => g.id), ['legacy', 'disabled', 'custom']);
  assert.deepEqual(Array.from(await h.platform.getPlatformGroups('codex'), (g) => g.id), ['legacy', 'disabled', 'custom']);
  stored = h.stored('codex');
  assert.ok(stored[0].sortOrder! < stored[1].sortOrder! && stored[1].sortOrder! < stored[2].sortOrder!);
  assert.deepEqual(stored.map((g) => g.quotaAutoRefreshMinutes), [-1, -1, 37]);
  assert.deepEqual(Array.from(await h.codex.getCodexQuotaRefreshDisabledAccountIds()).sort(), ['disabled-account', 'legacy-account']);
  assert.deepEqual(Array.from((await h.codex.getCodexCustomQuotaRefreshAccountIdsByMinutes()).get(37)!), ['custom-account']);
});

for (const key of ['antigravity', 'codex'] as const) {
  test(`${key}: common and existing entry points share updates in both directions`, async () => {
    const h = harness({ [key]: [group('one'), group('two')] });
    const legacy = key === 'codex' ? {
      read: h.codex.getCodexAccountGroups,
      assign: h.codex.assignAccountsToCodexGroup,
      rename: h.codex.renameCodexGroup,
    } : {
      read: h.antigravity.getAccountGroups,
      assign: h.antigravity.assignAccountsToGroup,
      rename: h.antigravity.renameGroup,
    };
    await h.platform.getPlatformGroups(key);
    await legacy.assign('one', ['added-by-existing-entry']);
    await h.platform.renamePlatformGroup(key, 'one', 'Common rename');
    assert.deepEqual(h.stored(key)[0].accountIds, ['one-account', 'added-by-existing-entry']);
    await h.platform.assignAccountsToPlatformGroup(key, 'two', ['added-by-common-entry']);
    await legacy.rename('two', 'Existing rename');
    assert.deepEqual(h.stored(key)[1].accountIds, ['two-account', 'added-by-common-entry']);
    assert.equal((await h.platform.getPlatformGroups(key))[1].name, 'Existing rename');
    assert.equal((await legacy.read())[0].name, 'Common rename');
  });

  for (const firstEntry of ['common', 'existing'] as const) {
    test(`${key}: overlapping ${firstEntry}-first writes preserve both memberships`, async () => {
      const h = harness({ [key]: [group('one')] });
      const legacyAssign = key === 'codex' ? h.codex.assignAccountsToCodexGroup : h.antigravity.assignAccountsToGroup;
      await h.platform.getPlatformGroups(key);
      const gate = deferred<void>();
      h.writeGates.set(key, gate);
      const common = () => h.platform.assignAccountsToPlatformGroup(key, 'one', ['from-common']);
      const existing = () => legacyAssign('one', ['from-existing']);
      const first = firstEntry === 'common' ? common() : existing();
      await settlePromises();
      assert.equal(h.writes.length, 1);
      const second = firstEntry === 'common' ? existing() : common();
      await settlePromises();
      gate.resolve();
      await Promise.all([first, second]);
      assert.deepEqual(h.stored(key)[0].accountIds.slice().sort(), ['from-common', 'from-existing', 'one-account']);
    });
  }
}

for (const key of ['antigravity', 'codex', 'cursor']) {
  test(`${key}: a rejected load cannot overwrite groups and can be retried`, async () => {
    const original = [group('existing')];
    const h = harness({ [key]: original });
    h.readErrors.set(key, new Error('Temporary disk read failure'));
    await assert.rejects(h.platform.getPlatformGroups(key), /Temporary disk read failure/);
    await assert.rejects(h.platform.createPlatformGroup(key, 'Unsafe create'), /Temporary disk read failure/);
    assert.equal(h.writes.length, 0);
    assert.deepEqual(h.stored(key), original);
    h.readErrors.delete(key);
    await h.platform.createPlatformGroup(key, 'Recovered create');
    assert.deepEqual(h.stored(key)[0].accountIds, ['existing-account']);
    assert.equal(h.stored(key).length, 2);
  });

  for (const invalid of ['{truncated', '{"groups": []}', '[null]', '[{"id":"one","name":"One","accountIds":[42]}]']) {
    test(`${key}: invalid data ${invalid} is preserved until repaired`, async () => {
      const h = harness();
      h.disk.set(key, invalid);
      await assert.rejects(h.platform.getPlatformGroups(key));
      await assert.rejects(h.platform.createPlatformGroup(key, 'Unsafe create'));
      assert.equal(h.writes.length, 0);
      assert.equal(h.disk.get(key), invalid);
      h.disk.set(key, JSON.stringify([group('restored')]));
      await h.platform.renamePlatformGroup(key, 'restored', 'Recovered rename');
      assert.equal(h.stored(key)[0].name, 'Recovered rename');
      assert.deepEqual(h.stored(key)[0].accountIds, ['restored-account']);
    });
  }

  test(`${key}: failed saves leave the cache intact and subsequent writes recover`, async () => {
    const h = harness({ [key]: [group('existing')] });
    await h.platform.getPlatformGroups(key);
    h.writeErrors.set(key, new Error('Disk full'));
    await assert.rejects(h.platform.renamePlatformGroup(key, 'existing', 'Unsaved rename'), /Disk full/);
    await assert.rejects(h.platform.assignAccountsToPlatformGroup(key, 'existing', ['unsaved-account']), /Disk full/);
    const groups = await h.platform.getPlatformGroups(key);
    assert.equal(groups[0].name, 'existing');
    assert.deepEqual(Array.from(groups[0].accountIds), ['existing-account']);
    assert.deepEqual(h.stored(key)[0].accountIds, ['existing-account']);
    h.writeErrors.delete(key);
    await h.platform.assignAccountsToPlatformGroup(key, 'existing', ['saved-account']);
    assert.equal(h.stored(key)[0].name, 'existing');
    assert.deepEqual(h.stored(key)[0].accountIds, ['existing-account', 'saved-account']);
  });

  test(`${key}: reads and renames preserve existing account membership without deduplication`, async () => {
    const original = [group('one', { accountIds: ['shared', 'shared'] }), group('two', { accountIds: ['shared'] })];
    const h = harness({ [key]: original });
    await h.platform.getPlatformGroups(key);
    assert.equal(h.writes.length, 0);
    await h.platform.renamePlatformGroup(key, 'one', 'Renamed');
    assert.deepEqual(h.stored(key).map((g) => g.accountIds), original.map((g) => g.accountIds));
  });
}

test('platform aliases share their own store while distinct platforms stay isolated', async () => {
  const h = harness({
    antigravity: [group('same', { name: 'Antigravity' })],
    codex: [group('same', { name: 'Codex', quotaAutoRefreshMinutes: -1 })],
    cursor: [group('same', { name: 'Cursor' })],
    windsurf: [group('same', { name: 'Windsurf' })],
  });
  await Promise.all(['antigravity', 'codex', 'cursor', 'windsurf'].map((key) => h.platform.getPlatformGroups(key)));
  await h.platform.renamePlatformGroup(' gemini ', 'same', 'Antigravity changed');
  await h.platform.renamePlatformGroup('codex_api_service', 'same', 'Codex changed');
  await h.platform.assignAccountsToPlatformGroup('cursor', 'same', ['cursor-only']);
  assert.equal((await h.antigravity.getAccountGroups())[0].name, 'Antigravity changed');
  assert.equal((await h.codex.getCodexAccountGroups())[0].name, 'Codex changed');
  assert.equal(h.stored('codex')[0].quotaAutoRefreshMinutes, -1);
  assert.deepEqual(h.stored('cursor')[0].accountIds, ['same-account', 'cursor-only']);
  assert.deepEqual(h.stored('windsurf'), [group('same', { name: 'Windsurf' })]);
  assert.deepEqual(h.stored('antigravity')[0].accountIds, ['same-account']);
  assert.deepEqual(h.stored('codex')[0].accountIds, ['same-account']);
});

for (const key of ['antigravity', 'codex', 'cursor']) {
  test(`${key}: overlapping membership and replacement preserve other groups and policies`, async () => {
    const h = harness({ [key]: [group('one', { quotaAutoRefreshMinutes: 37 }), group('two')] });
    await h.platform.assignAccountsToPlatformGroup(key, 'two', ['one-account']);
    assert.deepEqual(h.stored(key)[0].accountIds, ['one-account']);
    assert.deepEqual(h.stored(key)[1].accountIds, ['two-account', 'one-account']);
    await h.platform.setPlatformGroupAccounts(key, 'one', ['replacement', 'replacement']);
    assert.deepEqual(h.stored(key)[0].accountIds, ['replacement']);
    assert.deepEqual(h.stored(key)[1].accountIds, ['two-account', 'one-account']);
    if (key === 'codex') assert.equal(h.stored(key)[0].quotaAutoRefreshMinutes, 37);
  });
}
