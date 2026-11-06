import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule } from '../../tests/helpers/reactHookHarness';
import * as mfaVault from '../utils/mfaVault';
import type { MfaRecord } from '../utils/mfaVault';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}

function text(tree: any): string {
  if (typeof tree === 'string' || typeof tree === 'number') return String(tree);
  if (Array.isArray(tree)) return tree.map(text).join('');
  return tree && typeof tree === 'object' ? text(tree.props?.children) : '';
}

const saved: MfaRecord[] = [
  { id: 'charlie', accountName: 'Charlie', secret: 'SECRET_C', time: 3000 },
  { id: 'alpha', accountName: 'Alpha', secret: 'SECRET_A', time: 1000 },
  { id: 'bravo', accountName: 'Bravo', secret: 'SECRET_B', time: 2000 },
];
const history: MfaRecord[] = [
  { id: 'history-alpha', accountName: 'Alpha', secret: 'SECRET_A', time: 4000 },
];

function setup(options: {
  confirm?: () => Promise<boolean>;
  fallback?: boolean;
  records?: MfaRecord[];
  savePath?: string | null;
  importData?: string;
} = {}) {
  const storage = new Map<string, string>();
  const confirmations: Array<{ message: string; options: unknown }> = [];
  const fallbackMessages: string[] = [];
  const writes: Array<{ path: string; contents: string }> = [];
  const qrExports: MfaRecord[][] = [];
  let nextId = 0;
  const h = loadHookModule(new URL('./MfaVaultManager.tsx', import.meta.url), {
    '@tauri-apps/api/core': { isTauri: () => true },
    'react-i18next': { useTranslation: () => ({
      t: (key: string, defaultOrOptions?: unknown, translateOptions?: { count?: number }) => {
        const params = typeof defaultOrOptions === 'object'
          ? defaultOrOptions as { count?: number }
          : translateOptions;
        return params?.count === undefined ? key : `${key}:${params.count}`;
      },
    }) },
    '@tauri-apps/plugin-dialog': {
      confirm: async (message: string, dialogOptions: unknown) => {
        confirmations.push({ message, options: dialogOptions });
        return options.confirm ? options.confirm() : true;
      },
      open: async () => options.importData === undefined ? null : 'backup.json',
      save: async () => options.savePath ?? null,
    },
    '@tauri-apps/plugin-fs': {
      readTextFile: async () => options.importData ?? '',
      writeTextFile: async (path: string, contents: string) => { writes.push({ path, contents }); },
    },
    '../utils/mfaMigrationQr': {
      buildMfaMigrationQrPages: (records: readonly MfaRecord[]) => {
        qrExports.push(records.map(record => ({ ...record })));
        return [{ uri: 'demo', count: records.length, svg: '<svg/>', dataUrl: 'data:image/svg+xml,%3Csvg%2F%3E' }];
      },
    },
    '../utils/mfaVault': {
      ...mfaVault,
      loadSavedMfaRecords: () => (options.records ?? saved).map(record => ({ ...record })),
      loadMfaHistoryRecords: () => history.map(record => ({ ...record })),
      getMfaOtpToken: () => '123456',
      getMfaTimeRemaining: () => 30,
      createMfaRecordId: () => `added-${++nextId}`,
      parseGoogleAuthenticatorMigrationBatch: () => null,
      parseMfaCredentialInputs: (input: string) => input === 'new-account'
        ? [{ accountName: 'Added', secret: 'SECRET_NEW' }]
        : [],
      dedupeMfaRecordsBySecret: (records: MfaRecord[]) => records,
      toMfaSecretIdentity: (secret: string) => secret,
    },
  }, {
    localStorage: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
    },
    window: {
      setInterval: () => 1,
      clearInterval() {},
      confirm: (message: string) => {
        fallbackMessages.push(message);
        return options.fallback ?? false;
      },
    },
  });
  h.render(() => h.exports.MfaVaultManager());
  const allNodes = () => nodes(h.flush());
  const find = (predicate: (node: any) => boolean) => allNodes().find(predicate);
  const button = (key: string) => find(node => node.type === 'button' && text(node).includes(key));
  const rows = () => nodes(find(node => node.type === 'tbody')).filter(node => node.type === 'tr');
  const row = (id: string) => rows().find(node => node.key === id);
  const rowCheckbox = (id: string) => nodes(row(id)).find(node => node.type === 'input' && node.props.type === 'checkbox');
  const allCheckbox = () => find(node => node.type === 'input' && node.props['aria-label'] === 'common.selectAll');
  const toggle = (id: string, checked = true) => {
    const checkbox = rowCheckbox(id);
    assert.ok(checkbox, `Missing saved record checkbox: ${id}`);
    checkbox.props.onChange({ target: { checked } });
    h.flush();
  };
  const stored = (key = mfaVault.MFA_STORAGE_KEY_SAVED): MfaRecord[] => JSON.parse(storage.get(key) || '[]');
  const searchInput = () => find(node => node.type === 'input' && node.props.type === 'search');
  const search = (value: string) => {
    assert.ok(searchInput(), 'Missing account search');
    searchInput().props.onChange({ target: { value } });
    h.flush();
  };
  return { h, allNodes, find, button, rows, row, rowCheckbox, allCheckbox, toggle, stored, confirmations,
    fallbackMessages, writes, qrExports, searchInput, search };
}

test('saved records support partial selection, select all and clearing without selecting history', () => {
  const s = setup();
  try {
    assert.ok(s.allCheckbox());
    assert.equal(s.allCheckbox().props.checked, false);
    assert.equal(s.button('mfaVault.batchDelete').props.disabled, true);
    const headerInput = { indeterminate: false };
    s.allCheckbox().props.ref.current = headerInput;
    s.toggle('alpha');
    assert.equal(s.rowCheckbox('alpha').props.checked, true);
    assert.equal(s.rowCheckbox('bravo').props.checked, false);
    assert.equal(s.allCheckbox().props.checked, false);
    assert.equal(headerInput.indeterminate, true);
    assert.ok(s.button('mfaVault.batchDelete'));
    s.allCheckbox().props.onChange({ target: { checked: true } });
    assert.equal(s.allCheckbox().props.checked, true);
    assert.equal(headerInput.indeterminate, false);
    assert.ok(['alpha', 'bravo', 'charlie'].every(id => s.rowCheckbox(id).props.checked));
    s.button('messages.clearSelection').props.onClick();
    assert.ok(['alpha', 'bravo', 'charlie'].every(id => !s.rowCheckbox(id).props.checked));
    s.toggle('bravo');
    s.button('twoFactorAuth.tabHistory').props.onClick();
    assert.equal(s.allNodes().filter(node => node.type === 'input' && node.props.type === 'checkbox').length, 0);
    assert.equal(s.button('mfaVault.batchDelete'), undefined);
    s.button('twoFactorAuth.tabSaved').props.onClick();
    assert.equal(s.rowCheckbox('bravo').props.checked, true);
  } finally { s.h.unmount(); }
});

test('sorting keeps selection on record IDs and confirmed batch deletion preserves other records and history', async () => {
  const s = setup({ records: [
    { ...saved[0], accountName: 'Same' },
    { ...saved[1], accountName: 'Same' },
    saved[2],
  ] });
  try {
    s.toggle('alpha');
    s.button('mfaVault.time').props.onClick();
    assert.equal(s.rowCheckbox('alpha').props.checked, true);
    assert.equal(s.rowCheckbox('charlie').props.checked, false);
    await s.button('mfaVault.batchDelete').props.onClick();
    s.h.flush();
    assert.deepEqual(s.stored().map(record => record.id).sort(), ['bravo', 'charlie']);
    assert.deepEqual(s.stored(mfaVault.MFA_STORAGE_KEY_HISTORY), history);
    assert.equal(s.button('mfaVault.batchDelete').props.disabled, true);
    assert.match(s.confirmations[0].message, /1/);
  } finally { s.h.unmount(); }
});

test('canceling confirmation preserves both records and selection', async () => {
  const s = setup({ confirm: async () => false });
  try {
    s.toggle('alpha');
    s.toggle('bravo');
    await s.button('mfaVault.batchDelete').props.onClick();
    s.h.flush();
    assert.deepEqual(s.stored(), saved);
    assert.equal(s.rowCheckbox('alpha').props.checked, true);
    assert.equal(s.rowCheckbox('bravo').props.checked, true);
    assert.equal(s.button('mfaVault.batchDelete').props.disabled, false);
    assert.equal(s.fallbackMessages.length, 0);
  } finally { s.h.unmount(); }
});

test('pending confirmation blocks duplicate requests and deletes only the snapshot after a new record is saved', async () => {
  const confirmation = deferred<boolean>();
  const s = setup({ confirm: () => confirmation.promise });
  try {
    s.toggle('alpha');
    const staleButton = s.button('mfaVault.batchDelete');
    const deleting = staleButton.props.onClick();
    await staleButton.props.onClick();
    assert.equal(s.confirmations.length, 1);
    assert.equal(s.button('mfaVault.batchDelete').props.disabled, true);
    assert.equal(s.rowCheckbox('bravo').props.disabled, true);
    assert.equal(s.searchInput().props.disabled, true);
    s.search('Bravo');
    assert.equal(s.searchInput().props.value, '');
    assert.ok(s.row('alpha'));
    const input = s.find(node => node.type === 'input' && node.props.onPaste);
    input.props.onChange({ target: { value: 'new-account' } });
    s.button('twoFactorAuth.btnSaveToFavorites').props.onClick();
    s.h.flush();
    assert.ok(s.stored().some(record => record.id === 'added-1'));
    confirmation.resolve(true);
    await deleting;
    s.h.flush();
    assert.deepEqual(s.stored().map(record => record.id).sort(), ['added-1', 'bravo', 'charlie']);
    assert.deepEqual(s.stored(mfaVault.MFA_STORAGE_KEY_HISTORY), history);
  } finally { s.h.unmount(); }
});

test('deleting all selected records preserves the query panel and renders the empty state inside the list', async () => {
  const s = setup();
  try {
    s.toggle('alpha');
    const deleteAlpha = nodes(s.row('alpha')).find(node => node.type === 'button' && node.props.title === 'mfaVault.delete');
    await deleteAlpha.props.onClick();
    assert.equal(s.button('mfaVault.batchDelete').props.disabled, true);
    s.allCheckbox().props.onChange({ target: { checked: true } });
    await s.button('mfaVault.batchDelete').props.onClick();
    s.h.flush();
    assert.deepEqual(s.stored(), []);
    assert.equal(s.allCheckbox().props.checked, false);
    assert.equal(s.allCheckbox().props.disabled, true);
    assert.equal(s.button('mfaVault.batchDelete').props.disabled, true);
    assert.ok(s.find(node => node.type === 'table'));
    assert.ok(s.find(node => node.type === 'thead'));
    assert.match(text(s.find(node => node.type === 'tbody')), /mfaVault.noData/);
    assert.equal(s.find(node => node.props.className === 'mfa-empty-heading'), undefined);
    assert.equal(s.button('twoFactorAuth.btnQuery').props.disabled, true);
    assert.equal(s.button('twoFactorAuth.btnSaveToFavorites').props.disabled, true);
    assert.equal(s.button('mfaVault.exportBackup').props.disabled, true);
    assert.equal(s.button('mfaVault.exportMigrationQr').props.disabled, true);
    assert.match(text(s.find(node => node.props.className === 'query-result-box')), /twoFactorAuth.emptyQueryData/);
  } finally { s.h.unmount(); }
});

test('search matches issuer and account but never secrets and clears selection when the query changes', () => {
  const s = setup({ records: [
    { ...saved[1], accountName: 'OpenAI:Alice@example.com' },
    { ...saved[2], accountName: 'GitHub:bob@example.com' },
    { ...saved[0], accountName: 'OpenAI:carol@example.com' },
  ] });
  try {
    s.toggle('bravo');
    s.search('  oPeNaI  ');
    assert.deepEqual(s.rows().map(row => row.key), ['alpha', 'charlie']);
    assert.equal(s.button('mfaVault.batchDelete').props.disabled, true);
    s.allCheckbox().props.onChange({ target: { checked: true } });
    assert.equal(s.allCheckbox().props.checked, true);
    assert.ok(['alpha', 'charlie'].every(id => s.rowCheckbox(id).props.checked));
    s.search('ALICE@EXAMPLE.COM');
    assert.deepEqual(s.rows().map(row => row.key), ['alpha']);
    assert.equal(s.rowCheckbox('alpha').props.checked, false);
    s.search('SECRET_A');
    assert.match(text(s.find(node => node.type === 'tbody')), /mfaVault.noSearchResults/);
    assert.equal(s.allCheckbox().props.disabled, true);
    s.search('');
    assert.ok(['alpha', 'bravo', 'charlie'].every(id => !s.rowCheckbox(id).props.checked));
  } finally { s.h.unmount(); }
});

test('select all and batch deletion act only on current search results', async () => {
  const s = setup({ records: [
    { ...saved[1], accountName: 'OpenAI:alice@example.com' },
    { ...saved[2], accountName: 'GitHub:bob@example.com' },
    { ...saved[0], accountName: 'OpenAI:carol@example.com' },
  ] });
  try {
    s.search('OpenAI');
    s.allCheckbox().props.onChange({ target: { checked: true } });
    await s.button('mfaVault.batchDelete').props.onClick();
    s.h.flush();
    assert.deepEqual(s.stored().map(record => record.id), ['bravo']);
    assert.deepEqual(s.stored(mfaVault.MFA_STORAGE_KEY_HISTORY), history);
    assert.equal(s.allCheckbox().props.disabled, true);
    assert.match(text(s.find(node => node.type === 'tbody')), /mfaVault.noSearchResults/);
    assert.match(s.confirmations[0].message, /2/);
  } finally { s.h.unmount(); }
});

test('editing a selected account out of the search results removes it from batch actions', async () => {
  const s = setup({ records: [
    { ...saved[1], accountName: 'OpenAI:alice@example.com' },
    { ...saved[2], accountName: 'OpenAI:bob@example.com' },
  ] });
  try {
    s.search('OpenAI');
    s.toggle('alpha');
    const edit = nodes(s.row('alpha')).find(node => node.type === 'button' && node.props.title === 'mfaVault.editAccountName');
    edit.props.onClick();
    const editInput = nodes(s.row('alpha')).find(node => node.type === 'input' && node.props.type === 'text');
    editInput.props.onChange({ target: { value: 'GitHub:alice@example.com' } });
    const saveName = nodes(s.row('alpha')).find(node => node.type === 'button' && node.props.title === 'mfaVault.saveAccountName');
    saveName.props.onClick();
    s.h.flush();
    assert.equal(s.row('alpha'), undefined);
    assert.equal(s.button('mfaVault.batchDelete').props.disabled, true);
    await s.button('mfaVault.batchDelete').props.onClick();
    assert.equal(s.confirmations.length, 0);
    assert.equal(s.stored().length, 2);
    await s.button('mfaVault.exportMigrationQr').props.onClick();
    s.h.flush();
    assert.match(text(s.find(node => node.props.className === 'mfa-migration-summary')), /mfaVault.migrationAllSummary:2/);
    assert.deepEqual(s.qrExports[0].map(record => record.id).sort(), ['alpha', 'bravo']);
  } finally { s.h.unmount(); }
});

test('issuer and account use separate lines while time remains its own table cell', () => {
  const s = setup({ records: [
    { ...saved[1], accountName: 'OpenAI:alice:work@example.com' },
    { ...saved[2], accountName: 'bob@example.com' },
  ] });
  try {
    const name = (id: string) => nodes(s.row(id)).find(node => node.props.className === 'mfa-vault-account-text');
    const account = (id: string) => nodes(s.row(id)).find(node => node.props.className === 'mfa-account-secondary');
    const time = (id: string) => nodes(s.row(id)).find(node => node.type === 'td' && node.props.className === 'tfa-time-cell');
    assert.equal(text(name('alpha')), 'OpenAI');
    assert.equal(text(account('alpha')), 'alice:work@example.com');
    assert.ok(text(time('alpha')).length > 0);
    assert.equal(text(name('bravo')), 'bob@example.com');
    assert.equal(account('bravo'), undefined);
    assert.ok(s.find(node => node.type === 'th' && node.props.className === 'tfa-time-column'));
    assert.equal(nodes(s.row('alpha')).filter(node => node.props.className === 'mfa-account-time').length, 0);
    s.button('mfaVault.time').props.onClick();
    assert.deepEqual(s.rows().map(row => row.key), ['bravo', 'alpha']);
  } finally { s.h.unmount(); }
});

test('migration export snapshots selected records or all saved records independent of search', async () => {
  const s = setup();
  try {
    s.search('Alpha');
    await s.button('mfaVault.exportMigrationQr').props.onClick();
    s.h.flush();
    assert.deepEqual(s.qrExports[0].map(record => record.id).sort(), ['alpha', 'bravo', 'charlie']);
    assert.ok(s.find(node => node.type === 'img' && node.props.src.startsWith('data:image/svg+xml,')));
    s.toggle('alpha');
    await s.button('mfaVault.exportMigrationQr').props.onClick();
    s.h.flush();
    assert.deepEqual(s.qrExports[1].map(record => record.id), ['alpha']);
    s.search('Bravo');
    assert.deepEqual(s.qrExports[1].map(record => record.id), ['alpha']);
    assert.equal(s.stored().length, 3);
  } finally { s.h.unmount(); }
});

test('JSON backup exports all saved metadata and imports it without adding a type field', async () => {
  const records: MfaRecord[] = [
    { id: 'one', accountName: 'OpenAI:alice@example.com', secret: 'JBSWY3DPEHPK3PXP', remark: 'work', time: 1700000000000 },
    { id: 'two', accountName: 'bob@example.com', secret: 'GEZDGNBVGY3TQOJQ', remark: '', time: 1700000005000 },
  ];
  const exporter = setup({ records, savePath: 'backup.json' });
  let backup = '';
  try {
    exporter.search('OpenAI');
    exporter.toggle('one');
    await exporter.button('mfaVault.exportBackup').props.onClick();
    assert.equal(exporter.writes.length, 1);
    backup = exporter.writes[0].contents;
    assert.deepEqual(JSON.parse(backup), records.map(({ id: _id, ...record }) => record));
  } finally { exporter.h.unmount(); }
  const importer = setup({ records: [], importData: backup });
  try {
    await importer.button('mfaVault.importBackup').props.onClick();
    importer.h.flush();
    const imported = importer.stored().map(({ id: _id, ...record }) => record).sort((a, b) => a.time - b.time);
    assert.deepEqual(imported, records.map(({ id: _id, ...record }) => record));
    assert.ok(importer.stored().every(record => !('issuer' in record) && !('type' in record)));
  } finally { importer.h.unmount(); }
});

test('native confirmation failure uses the existing browser confirmation before deleting', async () => {
  for (const fallback of [false, true]) {
    const s = setup({ confirm: async () => { throw new Error('Native dialog unavailable'); }, fallback });
    try {
      s.toggle('alpha');
      await s.button('mfaVault.batchDelete').props.onClick();
      s.h.flush();
      assert.equal(s.fallbackMessages.length, 1);
      assert.equal(s.fallbackMessages[0], s.confirmations[0].message);
      assert.equal(s.stored().some(record => record.id === 'alpha'), !fallback);
      assert.deepEqual(s.stored(mfaVault.MFA_STORAGE_KEY_HISTORY), history);
    } finally { s.h.unmount(); }
  }
});

test('empty saved view keeps the query and list panels available across history and first save', () => {
  const s = setup({ records: [] });
  try {
    assert.equal(s.find(node => node.props.className === 'mfa-empty-heading'), undefined);
    assert.ok(s.find(node => node.type === 'table'));
    assert.ok(s.find(node => node.type === 'thead'));
    assert.match(text(s.find(node => node.type === 'tbody')), /mfaVault.noData/);
    assert.equal(s.allCheckbox().props.disabled, true);
    assert.equal(s.button('twoFactorAuth.btnQuery').props.disabled, true);
    assert.equal(s.button('twoFactorAuth.btnSaveToFavorites').props.disabled, true);
    assert.ok(nodes(s.find(node => node.props.className === 'list-actions'))
      .some(node => node.type === 'button' && text(node).includes('mfaVault.importBackup')));
    assert.ok(s.button('mfaVault.uploadQrImage'));
    assert.match(text(s.find(node => node.props.className === 'query-result-box')), /twoFactorAuth.emptyQueryData/);
    const input = () => s.find(node => node.type === 'input' && node.props.onPaste);
    input().props.onChange({ target: { value: 'new-account' } });
    assert.equal(s.button('twoFactorAuth.btnQuery').props.disabled, false);
    assert.equal(s.button('twoFactorAuth.btnSaveToFavorites').props.disabled, false);
    s.button('twoFactorAuth.tabHistory').props.onClick();
    assert.ok(s.find(node => node.type === 'table'));
    assert.equal(input().props.value, 'new-account');
    s.button('twoFactorAuth.tabSaved').props.onClick();
    assert.equal(input().props.value, 'new-account');
    s.button('twoFactorAuth.btnSaveToFavorites').props.onClick();
    assert.equal(s.find(node => node.props.className === 'mfa-empty-heading'), undefined);
    assert.ok(s.find(node => node.type === 'table'));
    assert.ok(s.find(node => node.type === 'thead'));
    assert.ok(s.find(node => node.props.className === 'query-result-box'));
    assert.equal(input().props.value, 'new-account');
    assert.equal(s.stored().length, 1);
    assert.equal(s.allCheckbox().props.disabled, false);
    assert.ok(s.row('added-1'));
    assert.doesNotMatch(text(s.find(node => node.type === 'tbody')), /mfaVault.noData/);
  } finally { s.h.unmount(); }
});
