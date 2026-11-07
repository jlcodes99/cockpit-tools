import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function picker() {
  const file = ts.createSourceFile('manager.tsx', readFileSync(new URL('./CodexModelProviderManager.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const owner = file.statements.find((node): node is ts.FunctionDeclaration => ts.isFunctionDeclaration(node) && node.name?.text === 'useCodexModelProviderManagerController');
  assert.ok(owner?.body);
  const names: string[] = [];
  const required = new Set(['notice', 'providerOauthError', 'oauthAccounts', 'providerOauthPickerId', 'providerOauthSaving', 'providerOauthSelectedAccountId',
    'providerOauthSearchQuery', 'providerOauthFilterTypes', 'providerOauthTagFilter', 'providerOauthSortBy', 'providerOauthSortDirection',
    'providerOauthTarget', 'providerOauthAccounts', 'providerOauthEligibleAccounts', 'selectedProviderOauthAccount',
    'isOAuthBindingEligibleAccount', 'handleProviderOauthBindingChange']);
  function bindings(node: ts.BindingName): string[] {
    return ts.isIdentifier(node) ? [node.text] : node.elements.flatMap((element) => ts.isBindingElement(element) ? bindings(element.name) : []);
  }
  const statements = owner.body.statements.filter((node) => {
    if (ts.isVariableStatement(node)) {
      const bound = node.declarationList.declarations.flatMap((decl) => bindings(decl.name));
      if (bound.some((name) => required.has(name))) { names.push(...bound); return true; }
    }
    return ts.isExpressionStatement(node) && ts.isCallExpression(node.expression)
      && ['useEffect', 'useDialogSession'].includes(node.expression.expression.getText(file))
      && /setProviderOauthSelectedAccountId|providerOauthPagination.setCurrentPage/.test(node.expression.getText(file));
  });
  const h = loadHookModule(new URL('../../hooks/useDialogSession.ts', import.meta.url), {});
  let fail = false, page = 1;
  const calls: any[] = [];
  const context = {
    ...h.hooks, ...h.exports,
    providers: [{ id: 'provider', boundOauthAccountId: 'original' }, { id: 'other', boundOauthAccountId: 'other-account' }],
    providerOauthPagination: { setCurrentPage(value: number) { page = value; } },
    isCodexApiKeyAccount: () => false,
    async updateCodexModelProvider(id: string, payload: any) { calls.push({ id, payload }); if (fail) throw new Error('save failed'); },
    async reloadProviders() {}, parseServiceError: String, t: (key: string) => key,
  };
  const source = `(() => { ${statements.map((node) => node.getText(file)).join('\n')}\nreturn { ${names.join(',')} }; })`;
  const render = vm.runInNewContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  h.render(render);
  const accounts = [{ id: 'original', tokens: { refresh_token: 'token' } }, { id: 'chosen', tokens: { refresh_token: 'token' } }];
  return { h, context, calls, accounts, read: () => h.flush(), fail(value: boolean) { fail = value; },
    page(value?: number) { if (value !== undefined) page = value; return page; },
  };
}

test('provider refresh preserves OAuth selection, search, filters, ordering and pagination; failure stays retryable', async () => {
  const p = picker(); p.read().setOauthAccounts(p.accounts); p.read().setProviderOauthPickerId('provider');
  assert.equal(p.read().providerOauthSelectedAccountId, 'original');
  p.read().setProviderOauthSelectedAccountId('chosen'); p.read().setProviderOauthSearchQuery('search');
  p.read().setProviderOauthFilterTypes(['TEAM']); p.read().setProviderOauthTagFilter(['tag']);
  p.read().setProviderOauthSortBy('created_at'); p.read().setProviderOauthSortDirection('asc'); p.h.flush(); p.page(3);
  p.context.providers = structuredClone(p.context.providers); p.read().setOauthAccounts(structuredClone(p.accounts)); p.h.flush();
  assert.equal(p.read().providerOauthSelectedAccountId, 'chosen'); assert.equal(p.read().providerOauthSearchQuery, 'search');
  assert.deepEqual([...p.read().providerOauthFilterTypes], ['TEAM']); assert.deepEqual([...p.read().providerOauthTagFilter], ['tag']);
  assert.equal(p.read().providerOauthSortBy, 'created_at'); assert.equal(p.read().providerOauthSortDirection, 'asc'); assert.equal(p.page(), 3);
  p.fail(true); await p.read().handleProviderOauthBindingChange(p.read().providerOauthTarget, p.read().selectedProviderOauthAccount.id);
  p.context.providers = structuredClone(p.context.providers); p.h.flush();
  assert.equal(p.read().providerOauthPickerId, 'provider'); assert.equal(p.read().providerOauthSelectedAccountId, 'chosen');
  assert.equal(p.read().providerOauthSaving, false); assert.ok(p.read().providerOauthError); assert.equal(p.read().notice, null);
  p.fail(false); await p.read().handleProviderOauthBindingChange(p.read().providerOauthTarget, p.read().selectedProviderOauthAccount.id);
  assert.equal(p.calls[p.calls.length - 1].payload.boundOauthAccountId, 'chosen');
  assert.equal(p.read().providerOauthPickerId, null); p.h.unmount();
});

test('late OAuth metadata does not reset the picker; reopening and switching providers initialize latest bindings', () => {
  const p = picker(); p.read().setProviderOauthPickerId('provider');
  assert.equal(p.read().providerOauthSelectedAccountId, 'original'); assert.equal(p.read().selectedProviderOauthAccount, null);
  p.read().setProviderOauthSearchQuery('typed before load'); p.read().setOauthAccounts(p.accounts); p.h.flush();
  assert.equal(p.read().selectedProviderOauthAccount.id, 'original'); assert.equal(p.read().providerOauthSearchQuery, 'typed before load');
  p.read().setProviderOauthSelectedAccountId('chosen'); p.read().setProviderOauthPickerId(null); p.h.flush();
  p.context.providers[0].boundOauthAccountId = 'chosen'; p.read().setProviderOauthPickerId('provider');
  assert.equal(p.read().providerOauthSelectedAccountId, 'chosen'); assert.equal(p.read().providerOauthSearchQuery, '');
  p.read().setProviderOauthPickerId('other'); assert.equal(p.read().providerOauthSelectedAccountId, 'other-account');
  p.h.unmount();
});
