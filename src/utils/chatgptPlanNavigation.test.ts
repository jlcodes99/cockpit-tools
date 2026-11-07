import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

class DetailEvent extends Event {
  detail: unknown;
  constructor(type: string, options?: { detail?: unknown }) { super(type); this.detail = options?.detail; }
}

test('a shortcut navigates to the shared API Service page and preserves context until the account panel mounts', () => {
  const exports: Record<string, any> = {};
  const window = new EventTarget();
  const events: unknown[] = [];
  const source = readFileSync(new URL('./chatgptPlanNavigation.ts', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS } }).outputText;
  vm.runInNewContext(compiled, { exports, window, CustomEvent: DetailEvent });
  window.addEventListener('app-request-navigate', (event) => {
    events.push((event as DetailEvent).detail);
    assert.equal(exports.hasPendingChatgptPlanAuthorization(), true);
  });
  window.addEventListener('chatgpt-plan-open-request', () => events.push('open'));
  assert.equal(exports.hasPendingChatgptPlanAuthorization(), false);
  exports.requestChatgptPlanAuthorization(' user@example.com ');
  assert.deepEqual(events, ['codex-api-service', 'open']);
  assert.equal(exports.takePendingChatgptPlanAuthorization().email, 'user@example.com');
  assert.equal(exports.takePendingChatgptPlanAuthorization(), null);
  assert.equal(exports.hasPendingChatgptPlanAuthorization(), false);
});

test('service pages retire the standalone grant tab and use the shared account guide', () => {
  const page = readFileSync(new URL('../pages/CodexApiServicePage.tsx', import.meta.url), 'utf8');
  const view = readFileSync(new URL('../pages/CodexApiServiceView.tsx', import.meta.url), 'utf8');
  const app = readFileSync(new URL('../App.tsx', import.meta.url), 'utf8');
  assert.doesNotMatch(page, /CHATGPT_PLAN_OPEN_REQUEST|hasPendingChatgptPlanAuthorization/);
  assert.doesNotMatch(view, /<ChatGptPlanAccounts/);
  assert.doesNotMatch(page, /requestCodexOfficialAuthorizationCheck/);
  assert.match(app, /<CodexAccountServiceHost activePage=/);
});
