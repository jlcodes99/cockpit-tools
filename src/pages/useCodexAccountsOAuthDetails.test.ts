import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { parseMfaCredentialInput } from '../utils/mfaVault';
import { deferred } from '../../tests/helpers/reactHookHarness';

const file = 'useCodexAccountsOAuthController.ts';
const source = ts.createSourceFile(file, readFileSync(new URL(file, import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true);
const handlers = new Map<string, string>();
ts.forEachChild(source, function visit(node) {
  if (ts.isVariableDeclaration(node) && node.initializer) {
    const initializer = ts.isCallExpression(node.initializer) && node.initializer.expression.getText(source) === 'useCallback'
      ? node.initializer.arguments[0] : node.initializer;
    handlers.set(node.name.getText(source), initializer.getText(source));
  }
  ts.forEachChild(node, visit);
});

function harness() {
  const saves: { id: string; update: Record<string, string> }[] = [];
  const exchanges: string[] = [];
  const timers: (() => void)[] = [];
  const snapshots: any[] = [];
  const emptyForm = { note: '', twoFactorSecret: '', accountPassword: '', phoneNumber: '', mailUrl: '' };
  const account = { id: 'authorized-account', email: 'authorized@example.com', account_password: 'existing-password', note: 'existing-note' };
  const state: Record<string, any> = {
    pendingOAuthNoteFormRef: { current: { ...emptyForm } },
    oauthCompletedAccountRef: { current: null }, oauthLoginIdRef: { current: 'login-1' },
    oauthCompletingRef: { current: false }, oauthActiveRef: { current: true },
    showAddModalRef: { current: true }, addTabRef: { current: "oauth" },
    reauthTargetAccountId: '', reauthRetryOAuthBinding: null, reauthRetrySwitchAccountId: null,
    reauthRetryInstanceId: null, reauthRetryLaunchAfterSwitch: false,
    parseMfaCredentialInput, t: (key: string, args?: any) => `${key}${args?.error ? ': ' + args.error : ''}`,
    openPendingOAuthNoteModal() { state.noteModalOpen = true; },
    setPendingOAuthFieldErrors(value: any) { state.fieldErrors = typeof value === 'function' ? value(state.fieldErrors ?? {}) : value; },
    setAccountNoteError(value: any) { state.noteError = value; },
    setAccountNoteFieldErrors(value: any) { state.noteFieldErrors = value; },
    setAddStatus(value: string) { state.status = value; },
    setAddMessage(value: string) { state.message = value; },
    applyAccountSnapshot(value: any) { snapshots.push(value); },
    fetchAccounts: async () => {}, fetchCurrentAccount: async () => {}, reloadLocalAccessState: async () => {},
    assignCodexAccountsToTargetGroup: async () => {}, emitAccountsChanged: async () => {},
    syncImportedAccountsToApiService: async () => {}, oauthLog() {},
    resetAddModalState() { state.reset = true; }, setShowAddModal(value: boolean) { state.modalOpen = value; },
    setTimeout(callback: () => void) { timers.push(callback); },
    modalOpen: true,
    codexService: {
      async completeCodexOAuthLogin(loginId: string) { exchanges.push(loginId); return { ...account }; },
      async updateCodexAccountNote(id: string, update: any) {
        saves.push({ id, update });
        if (state.failSave) throw new Error('disk full');
        return { ...account, account_password: update.accountPassword ?? account.account_password,
          two_factor_secret: update.twoFactorSecret, note: update.note ?? account.note };
      },
    },
  };
  for (const name of ['OauthUrl', 'OauthUrlCopied', 'OauthPrepareError', 'OauthPortInUse', 'OauthTimeoutInfo',
    'OauthCallbackInput', 'OauthCallbackSubmitting', 'OauthCallbackError', 'OauthTokenExchangeRetryVisible',
    'DeviceAuthInfo', 'DeviceAuthError', 'OauthMethod', 'DeviceCodeCopied']) {
    state[`set${name}`] = (value: any) => { state[name] = value; };
  }
  const context = vm.createContext(state);
  for (const name of ['buildPendingOAuthNoteUpdate', 'resolveCompletedOauthAccount', 'completeOauthSuccess',
    'completeOauthError', 'handleRetryOauthTokenExchange']) {
    assert.ok(handlers.has(name), `missing handler ${name}`);
    vm.runInContext(ts.transpileModule(`globalThis.${name} = ${handlers.get(name)}`, {
      compilerOptions: { target: ts.ScriptTarget.ES2022 },
    }).outputText, context);
  }
  return { state, saves, exchanges, timers, snapshots, emptyForm };
}

test('direct OAuth completion saves the latest entered account details on the authorized account', async () => {
  const h = harness();
  const exchange = deferred<any>();
  h.state.codexService.completeCodexOAuthLogin = () => exchange.promise;
  const completing = h.state.resolveCompletedOauthAccount('login-1');
  h.state.pendingOAuthNoteFormRef.current = { ...h.emptyForm, note: 'memo', accountPassword: 'entered-password',
    twoFactorSecret: 'JBSWY3DPEHPK3PXP', phoneNumber: '+1234567', mailUrl: 'https://mail.example.com' };
  exchange.resolve({ id: 'actual-oauth-account', email: 'actual@example.com' });
  await h.state.completeOauthSuccess(await completing);
  assert.equal(h.saves[0].id, 'actual-oauth-account');
  assert.equal(h.saves[0].update.accountPassword, 'entered-password');
  assert.equal(h.saves[0].update.twoFactorSecret, 'JBSWY3DPEHPK3PXP');
  assert.equal(h.saves[0].update.phoneNumber, '+1234567');
  assert.equal(h.saves[0].update.mailUrl, 'https://mail.example.com');
  assert.equal(h.state.status, 'success');
  assert.equal(h.timers.length, 1);
});

test('local save failure keeps the dialog open and retries without exchanging OAuth again', async () => {
  const h = harness();
  h.state.pendingOAuthNoteFormRef.current.accountPassword = 'entered-password';
  h.state.failSave = true;
  await h.state.completeOauthSuccess(await h.state.resolveCompletedOauthAccount('login-1'));
  assert.equal(h.state.status, 'error');
  assert.match(h.state.message, /accountNote.saveFailed: disk full/);
  assert.equal(h.state.modalOpen, true);
  assert.equal(h.state.OauthTokenExchangeRetryVisible, true);
  assert.equal(h.state.oauthLoginIdRef.current, 'login-1');
  assert.equal(h.timers.length, 0);
  h.state.failSave = false;
  h.state.pendingOAuthNoteFormRef.current.accountPassword = 'corrected-password';
  await h.state.handleRetryOauthTokenExchange();
  assert.deepEqual(h.exchanges, ['login-1']);
  assert.equal(h.saves.length, 2);
  assert.equal(h.saves[1].update.accountPassword, 'corrected-password');
  assert.equal(h.state.status, 'success');
  assert.equal(h.state.OauthTokenExchangeRetryVisible, false);
  assert.equal(h.state.OauthCallbackError, null);
  assert.deepEqual(Object.keys(h.state.fieldErrors), []);
  assert.equal(h.state.oauthCompletingRef.current, false);
});

test('empty or partial OAuth drafts preserve existing account details', async () => {
  const h = harness();
  h.state.reauthTargetAccountId = 'authorized-account';
  await h.state.completeOauthSuccess(await h.state.resolveCompletedOauthAccount('login-1'));
  assert.equal(h.saves.length, 0);
  assert.equal(h.snapshots[0].account_password, 'existing-password');
  const partial = harness();
  partial.state.pendingOAuthNoteFormRef.current.note = 'new note';
  await partial.state.completeOauthSuccess(await partial.state.resolveCompletedOauthAccount('login-1'));
  assert.deepEqual(Object.keys(partial.saves[0].update), ['note']);
  assert.equal(partial.snapshots[0].account_password, 'existing-password');
});

test('invalid MFA blocks the local save and can be corrected without another OAuth exchange', async () => {
  const h = harness();
  h.state.pendingOAuthNoteFormRef.current.twoFactorSecret = 'invalid!';
  await h.state.completeOauthSuccess(await h.state.resolveCompletedOauthAccount('login-1'));
  assert.equal(h.saves.length, 0);
  assert.equal(h.state.status, 'error');
  assert.equal(h.state.noteModalOpen, true);
  assert.ok(h.state.fieldErrors.twoFactorSecret);
  assert.equal(h.timers.length, 0);
  h.state.pendingOAuthNoteFormRef.current.twoFactorSecret = 'JBSWY3DPEHPK3PXP';
  await h.state.handleRetryOauthTokenExchange();
  assert.deepEqual(h.exchanges, ['login-1']);
  assert.equal(h.state.status, 'success');
});

test('a completed result is never reused for a different OAuth session', async () => {
  const h = harness();
  await h.state.resolveCompletedOauthAccount('login-1');
  await h.state.resolveCompletedOauthAccount('login-1');
  h.state.oauthLoginIdRef.current = 'login-2';
  await h.state.resolveCompletedOauthAccount('login-2');
  assert.deepEqual(h.exchanges, ['login-1', 'login-2']);
});


test('a late exchange from a closed dialog cannot save a newer login draft', async () => {
  const h = harness();
  const exchange = deferred<any>();
  h.state.codexService.completeCodexOAuthLogin = () => exchange.promise;
  const result = h.state.resolveCompletedOauthAccount('login-1');
  h.state.oauthLoginIdRef.current = 'login-2';
  h.state.pendingOAuthNoteFormRef.current.accountPassword = 'new-session-password';
  exchange.resolve({ id: 'old-session-account' });
  await h.state.completeOauthSuccess(await result);
  assert.equal(h.state.oauthCompletedAccountRef.current, null);
  assert.equal(h.saves.length, 0);
  assert.equal(h.state.status, undefined);
  assert.equal(h.timers.length, 0);
});

test('a local save finishing after dialog replacement does not update or close the new dialog', async () => {
  const h = harness();
  h.state.pendingOAuthNoteFormRef.current.note = 'old-session-note';
  const save = deferred<any>();
  h.state.codexService.updateCodexAccountNote = () => save.promise;
  const saving = h.state.completeOauthSuccess(await h.state.resolveCompletedOauthAccount('login-1'));
  h.state.oauthLoginIdRef.current = 'login-2';
  h.state.oauthCompletedAccountRef.current = null;
  save.resolve({ id: 'authorized-account', note: 'old-session-note' });
  await saving;
  assert.equal(h.state.status, undefined);
  assert.equal(h.state.modalOpen, true);
  assert.equal(h.timers.length, 0);
  assert.equal(h.snapshots.length, 0);
});
