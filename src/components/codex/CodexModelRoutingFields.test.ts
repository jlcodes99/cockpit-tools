import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

// Execute the component's real handlers and rendered props without a browser.
// Only hooks, translated labels and external services are replaced by test doubles.
type Element = { type: any; props: Record<string, any> };
type Props = Record<string, any>;
const source = readFileSync(new URL('./CodexModelRoutingFields.tsx', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: {
  module: ts.ModuleKind.CommonJS,
  target: ts.ScriptTarget.ES2022,
  jsx: ts.JsxEmit.ReactJSX,
} }).outputText;

const account = {
  id: 'provider-1', email: 'provider', auth_mode: 'apikey',
  openai_api_key: 'test-key', api_base_url: 'https://example.invalid',
  api_model_catalog: ['model-1'],
};
const route = {
  id: 'route-1', namespace: 'api', providerAccountId: account.id,
  enabled: true, extraModels: ['manual-model'],
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function harness(overrides: Props = {}) {
  let active: { slots: any[]; index: number; effects: (() => void)[] };
  const calls = { confirmed: 0, fetched: 0, saved: 0, refreshed: 0, routes: [] as any[], enabled: [] as boolean[], closed: 0 };
  const exports: Props = {};
  const hooks = {
    useState(initial: any) {
      const owner = active;
      const index = owner.index++;
      if (!(index in owner.slots)) owner.slots[index] = typeof initial === 'function' ? initial() : initial;
      return [owner.slots[index], (next: any) => {
        owner.slots[index] = typeof next === 'function' ? next(owner.slots[index]) : next;
      }];
    },
    useRef(initial: any) {
      const index = active.index++;
      if (!(index in active.slots)) active.slots[index] = { current: initial };
      return active.slots[index];
    },
    useMemo(factory: () => any) { return factory(); },
    useCallback(callback: any) { return callback; },
    useEffect(effect: () => void | (() => void), dependencies?: any[]) {
      const owner = active;
      const index = owner.index++;
      const previous = owner.slots[index];
      if (previous && dependencies && dependencies.length === previous.dependencies?.length
        && dependencies.every((value, offset) => Object.is(value, previous.dependencies[offset]))) return;
      owner.effects.push(() => {
        previous?.cleanup?.();
        owner.slots[index] = { dependencies, cleanup: effect() };
      });
    },
  };
  const jsx = (type: any, props: Props): Element => ({ type, props });
  vm.runInNewContext(compiled, {
    exports,
    document: { body: {} },
    window: { confirm: () => { throw new Error('Unexpected native fallback'); } },
    require(name: string) {
      if (name === 'react') return hooks;
      if (name === 'react/jsx-runtime') return { jsx, jsxs: jsx, Fragment: 'fragment' };
      if (name === 'react-dom') return { createPortal: (element: Element) => element };
      if (name === 'lucide-react') return new Proxy({}, { get: (_, key) => key });
      if (name === 'react-i18next') return { useTranslation: () => ({ t: (key: string) => key }) };
      if (name === '@tauri-apps/plugin-dialog') return {
        confirm: () => { calls.confirmed += 1; return overrides.confirm?.() ?? Promise.resolve(true); },
      };
      if (name === '../../hooks/useEscClose') return { useEscClose() {} };
      if (name === '../../services/modelProviderUsageService') return {
        listModelProviderModels: () => {
          calls.fetched += 1;
          return overrides.fetch?.() ?? Promise.resolve({ models: [{ id: 'new-model' }] });
        },
      };
      if (name === '../../services/codexService') return {
        updateCodexApiKeyCredentials: async () => { calls.saved += 1; },
      };
      if (name === '../../types/instance') return { CODEX_PROVIDER_GATEWAY_INTERNAL_NAMESPACE: 'internal' };
      if (name === '../../utils/codexModelRoutingValue') return {};
      if (name === '../SingleSelectDropdown') return { SingleSelectDropdown: 'dropdown' };
      if (name === '../ModalErrorMessage') return { ModalErrorMessage: 'modal-error' };
      if (name.endsWith('.css')) return {};
      throw new Error(`Unexpected import: ${name}`);
    },
  });
  return {
    calls,
    component(name: string, initial: Props = {}) {
      const state = { slots: [], index: 0, effects: [] as (() => void)[] };
      let props: Props = {
        enabled: true, open: true, routes: [route], accounts: [account],
        onRoutesChange: (next: any[]) => { calls.routes.push(next); },
        onEnabledChange: (next: boolean) => { calls.enabled.push(next); },
        onAccountsRefresh: () => { calls.refreshed += 1; },
        onClose: () => { calls.closed += 1; },
        ...initial,
      };
      return {
        render(next: Props = {}) {
          props = { ...props, ...next };
          active = state;
          state.index = 0;
          state.effects = [];
          const tree: Element = exports[name](props);
          for (const effect of state.effects) effect();
          return tree;
        },
      };
    },
  };
}

function elements(tree: Element): Element[] {
  const result: Element[] = [];
  const visit = (value: any) => {
    if (Array.isArray(value)) { value.forEach(visit); return; }
    if (!value || typeof value !== 'object' || !value.props) return;
    result.push(value);
    visit(value.props.children);
  };
  visit(tree);
  return result;
}

function byClass(tree: Element, className: string): Element {
  const found = elements(tree).find((element) => element.props.className?.split(' ').includes(className));
  assert.ok(found, className);
  return found;
}

function byComponent(tree: Element, name: string): Element {
  const found = elements(tree).find((element) => element.type?.name === name);
  assert.ok(found, name);
  return found;
}

function button(tree: Element, key: string): Element {
  const found = elements(tree).find((element) => element.type === 'button'
    && [element.props.children].flat().includes(key));
  assert.ok(found, key);
  return found;
}

const flush = () => new Promise<void>((resolve) => setImmediate(resolve));
const clickEvent = { preventDefault() {}, stopPropagation() {} };

test('disabled propagates through row, card-summary and card-inline paths', () => {
  const app = harness();
  for (const [variant, mode, child] of [
    ['row', 'summary', 'CodexModelRoutingModal'],
    ['card', 'summary', 'CodexModelRoutingSummary'],
    ['card', 'inline', 'CodexModelRoutingEditor'],
  ]) {
    const tree = app.component('CodexModelRoutingFields', { variant, mode, disabled: true }).render();
    assert.equal(byComponent(tree, child).props.disabled, true);
    const toggle = elements(tree).find((element) => element.type === 'input');
    assert.equal(toggle?.props.disabled, true);
  }
  const summary = app.component('CodexModelRoutingSummary', { disabled: true });
  const tree = summary.render();
  assert.equal(byComponent(tree, 'CodexModelRoutingModal').props.disabled, true);
  assert.equal(byClass(tree, 'codex-model-routing-summary__manage-btn').props.disabled, true);
  byClass(tree, 'codex-model-routing-summary__row').props.onClick();
  assert.equal(byComponent(summary.render(), 'CodexModelRoutingModal').props.open, false);
});

test('toggle confirmations cannot mutate after fields or a nested modal become disabled', async () => {
  for (const name of ['CodexModelRoutingFields', 'CodexModelRoutingModal']) {
    const confirmation = deferred<boolean>();
    const app = harness({ confirm: () => confirmation.promise });
    const component = app.component(name, { disabled: false, enabled: false });
    byClass(component.render(), 'codex-model-routing__switch').props.onClick(clickEvent);
    assert.equal(app.calls.confirmed, 1);
    component.render({ disabled: true });
    confirmation.resolve(true);
    await flush();
    assert.deepEqual(app.calls.enabled, []);
    assert.deepEqual(app.calls.routes, []);
  }
});

test('accepted confirmation uses the current routes instead of replacing them from an old closure', async () => {
  const confirmation = deferred<boolean>();
  const app = harness({ confirm: () => confirmation.promise });
  const component = app.component('CodexModelRoutingFields', { enabled: false, routes: [] });
  byClass(component.render(), 'codex-model-routing__switch').props.onClick(clickEvent);
  component.render({ routes: [route] });
  confirmation.resolve(true);
  await flush();
  assert.deepEqual(app.calls.enabled, [true]);
  assert.deepEqual(app.calls.routes, []);
});

test('disabled modal blocks apply and toggle while close and cancel remain available', () => {
  const app = harness();
  const component = app.component('CodexModelRoutingModal', { disabled: true, errorMessage: 'config read timed out' });
  component.render();
  const tree = component.render();
  assert.ok(elements(tree).some((element) => element.type === 'modal-error' && element.props.message === 'config read timed out'));
  assert.equal(byComponent(tree, 'CodexModelRoutingEditor').props.disabled, true);
  const save = button(tree, 'common.save');
  assert.equal(save.props.disabled, true);
  save.props.onClick();
  byClass(tree, 'codex-model-routing__switch').props.onClick(clickEvent);
  assert.equal(app.calls.confirmed, 0);
  assert.deepEqual(app.calls.routes, []);
  for (const control of [byClass(tree, 'modal-close'), button(tree, 'common.cancel')]) {
    assert.notEqual(control.props.disabled, true);
    control.props.onClick();
  }
  assert.equal(app.calls.closed, 2);
});

test('modal preserves edits on equivalent parent refresh and reloads genuinely changed routes', () => {
  const app = harness();
  const component = app.component('CodexModelRoutingModal');
  component.render();
  const edited = [{ ...route, namespace: 'draft' }];
  byComponent(component.render(), 'CodexModelRoutingEditor').props.onRoutesChange(edited);
  const refresh = component.render({ disabled: true, routes: [{ ...route }] });
  assert.equal(byComponent(refresh, 'CodexModelRoutingEditor').props.routes[0].namespace, 'draft');
  component.render({ disabled: false, routes: [{ ...route, namespace: 'reloaded' }] });
  assert.equal(byComponent(component.render(), 'CodexModelRoutingEditor').props.routes[0].namespace, 'reloaded');
});

test('disabled editor blocks real mutation handlers, model fetches and native controls', async () => {
  const app = harness();
  const tree = app.component('CodexModelRoutingEditor', { disabled: true }).render();
  for (const element of elements(tree)) {
    if (element.type === 'button' || element.type === 'input' || element.type === 'dropdown') {
      assert.equal(element.props.disabled, true, element.props.className || element.type);
    }
  }
  byClass(tree, 'codex-model-routing-card__pill').props.onClick();
  byClass(tree, 'codex-model-routing-card__delete').props.onClick();
  byClass(tree, 'codex-model-routing-card__namespace-input').props.onChange({ target: { value: 'changed' } });
  button(tree, 'instances.form.modelRouting.fetchModels').props.onClick();
  await flush();
  assert.deepEqual(app.calls.routes, []);
  assert.equal(app.calls.fetched, 0);
});

test('catalog fetch started before blocking cannot persist or refresh after disable and re-enable', async () => {
  const request = deferred<any>();
  const app = harness({ fetch: () => request.promise });
  const component = app.component('CodexModelRoutingEditor');
  button(component.render(), 'instances.form.modelRouting.fetchModels').props.onClick();
  assert.equal(app.calls.fetched, 1);
  component.render({ disabled: true });
  component.render({ disabled: false });
  request.resolve({ models: [{ id: 'obsolete-model' }] });
  await flush();
  assert.equal(app.calls.saved, 0);
  assert.equal(app.calls.refreshed, 0);
  assert.equal(button(component.render(), 'instances.form.modelRouting.fetchModels').props.disabled, false);
});

test('omitting disabled retains model editing and catalog refresh behavior', async () => {
  const app = harness();
  const tree = app.component('CodexModelRoutingEditor').render();
  byClass(tree, 'codex-model-routing-card__pill').props.onClick();
  assert.equal(app.calls.routes.length, 1);
  button(tree, 'instances.form.modelRouting.fetchModels').props.onClick();
  await flush();
  assert.equal(app.calls.fetched, 1);
  assert.equal(app.calls.saved, 1);
  assert.equal(app.calls.refreshed, 1);
});
