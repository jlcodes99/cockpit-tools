import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness.ts';
import * as order from '../../utils/codexExperimentalModelOrder.ts';
import * as context from '../../utils/codexModelContext.ts';
import * as modelConfig from '../../utils/codexModelConfig.ts';

function elements(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(elements);
  return [value, ...elements(value.props?.children)];
}

function harness(initial: any = {}) {
  const listeners = new Map<string, Set<(event: any) => void>>();
  const document: any = {
    body: {}, activeElement: null,
    addEventListener(type: string, listener: (event: any) => void) {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type)!.add(listener);
    },
    removeEventListener(type: string, listener: (event: any) => void) { listeners.get(type)?.delete(listener); },
  };
  class Element {
    parentElement: any = null;
    children: Element[] = [];
    props: any = {};
    focused = 0;
    focus() { this.focused++; document.activeElement = this; }
    contains(node: any) { return node === this || this.children.includes(node); }
    getBoundingClientRect() { return { left: 500, top: 510, bottom: 556, width: 200 }; }
    querySelectorAll() { return this.children; }
    querySelector(selector: string) {
      return selector.includes('aria-selected')
        ? this.children.find(child => child.props['aria-selected']) : this.children[0];
    }
  }
  const dom = new Map<string, Element>();
  const node = (key: string, props: any) => {
    if (!dom.has(key)) dom.set(key, new Element());
    const element = dom.get(key)!;
    element.props = props;
    return element;
  };
  let escape: { open: boolean; close: () => void; ref: any };
  let models: any[] = [{ model_id: 'gpt-6-luna', display_name: 'Model', ...initial }];
  let disabled = false;
  const h = loadHookModule(new URL('./CodexExperimentalModelEditor.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../utils/codexExperimentalModelOrder': order,
    '../../utils/codexModelContext': context,
    '../../utils/codexModelConfig': modelConfig,
    '../../services/codexService': { getCodexModelReasoningEfforts: async () => ({}) },
    '../../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: () => void, ref: any) => { escape = { open, close, ref }; } },
    '../SingleSelectDropdown': { SingleSelectDropdown: () => null },
    './CodexModelConfigTransferModal': { CodexModelConfigTransferModal: () => null },
    'react-dom': { createPortal: (child: any, target: any) => ({ type: 'portal', target, props: { children: child } }) },
  }, { document, Node: Element, window: {
    innerWidth: 900, innerHeight: 600,
    getComputedStyle: (element: Element) => ({ zIndex: element.props.style?.zIndex ?? '12040' }),
    addEventListener() {}, removeEventListener() {},
  } });
  let tree = h.render(() => {
    const output = h.exports.CodexExperimentalModelEditor({ models, mode: 'inline', disabled, onChange: (next: any[]) => { models = next; } });
    // Simulate React's ref commit before effects, including the separate portal subtree.
    for (const element of elements(output)) {
      const props = element.props;
      if (!props?.ref) continue;
      const current = node(props.className, props);
      if (props.role === 'listbox') {
        current.children = elements(props.children).filter(child => child.props?.role === 'option')
          .map(child => node(`${props.className}:${child.key ?? 'official'}`, child.props));
      }
      props.ref.current = current;
    }
    return output;
  });
  const flush = () => { tree = h.flush(); };
  const trigger = (kind: string) => elements(tree).find(element => element.props?.className === `codex-experimental-model-editor__${kind}-trigger`);
  const menu = () => elements(tree).find(element => element.props?.role === 'listbox');
  const key = (element: any, value: string, shiftKey = false) => {
    let prevented = false;
    element.props.onKeyDown({ key: value, shiftKey, preventDefault() { prevented = true; } });
    flush();
    return prevented;
  };
  return {
    h, document, flush, trigger, menu, key,
    open(kind: string, arrow = 'ArrowDown') { return key(trigger(kind), arrow); },
    get options(): Element[] { return menu()?.props.ref.current.children ?? []; },
    get models() { return models; },
    get escape() { return escape; },
    get portal() { return elements(tree).find(element => element.type === 'portal'); },
    clickOption(index: number) { this.options[index].props.onClick(); flush(); },
    pointerDown(target: unknown) { for (const listener of listeners.get('pointerdown') ?? []) listener({ target }); flush(); },
    disable() { disabled = true; flush(); },
  };
}

test('reasoning portal focuses the selected option and keeps multi-selection keyboard focus stable', () => {
  const h = harness({ reasoning_efforts: ['medium', 'high'] });
  assert.equal(h.open('reasoning'), true);
  assert.equal(h.portal.target, h.document.body);
  const menu = h.menu();
  assert.equal(menu.props.style.zIndex, 12041);
  assert.equal(menu.props.style.bottom, 100);
  assert.equal(menu.props.style.maxHeight, 280);
  assert.equal(h.document.activeElement, h.options[2]);
  assert.equal(menu.props['aria-multiselectable'], 'true');
  assert.equal(h.key(menu, 'End'), true);
  assert.equal(h.document.activeElement, h.options[6]);
  h.key(h.menu(), 'ArrowDown');
  assert.equal(h.document.activeElement, h.options[0]);
  h.key(h.menu(), 'ArrowUp');
  assert.equal(h.document.activeElement, h.options[6]);
  h.key(h.menu(), 'Home');
  h.key(h.menu(), 'ArrowDown');
  const low = h.options[1];
  h.clickOption(1);
  assert.deepEqual(h.models[0].reasoning_efforts, ['low', 'medium', 'high']);
  assert.equal(h.document.activeElement, low);
  assert.equal(h.options[1].props['aria-selected'], true);
  assert.ok(h.menu());
  h.key(h.menu(), 'Home');
  const trigger = h.trigger('reasoning').props.ref.current;
  h.clickOption(0);
  assert.equal(h.menu(), undefined);
  assert.equal(h.document.activeElement, trigger);
  assert.equal(h.models[0].reasoning_efforts, undefined);
  h.h.unmount();
});

test('context selection returns focus, reopening stays visible, and Tab in either direction closes safely', () => {
  const h = harness({ context_window: 1_000_000, auto_compact_token_limit: 900_000 });
  h.open('context', 'ArrowUp');
  const trigger = h.trigger('context').props.ref.current;
  assert.equal(h.document.activeElement, h.options[2]);
  trigger.focus();
  h.open('context');
  assert.equal(h.menu().props.style.visibility, 'visible');
  assert.equal(h.document.activeElement, h.options[2]);
  h.key(h.menu(), 'ArrowUp');
  h.clickOption(1);
  assert.equal(h.models[0].context_window, 516_000);
  assert.equal(h.document.activeElement, trigger);
  assert.equal(h.menu(), undefined);
  for (const shiftKey of [false, true]) {
    h.open('context');
    assert.equal(h.key(h.menu(), 'Tab', shiftKey), true);
    assert.equal(h.menu(), undefined);
    assert.equal(h.document.activeElement, trigger);
    assert.equal(h.escape.open, false);
  }
  h.h.unmount();
});

test('Escape registers the portaled layer and returns focus without closing the editor', () => {
  const h = harness();
  for (const kind of ['reasoning', 'context']) {
    h.trigger(kind).props.onClick(); h.flush();
    const trigger = h.trigger(kind).props.ref.current;
    assert.equal(h.escape.open, true);
    assert.equal(h.escape.ref, h.menu().props.ref);
    h.escape.close(); h.flush();
    assert.equal(h.menu(), undefined);
    assert.ok(h.trigger(kind));
    assert.equal(h.document.activeElement, trigger);
  }
  h.h.unmount();
});

test('outside clicks and disabling close the portal without stealing another control focus', () => {
  const h = harness();
  h.open('reasoning');
  h.pointerDown(h.options[1]);
  assert.ok(h.menu());
  const outside = {};
  h.document.activeElement = outside;
  h.pointerDown(outside);
  assert.equal(h.menu(), undefined);
  assert.equal(h.document.activeElement, outside);
  h.open('context');
  h.document.activeElement = outside;
  h.disable();
  assert.equal(h.menu(), undefined);
  assert.equal(h.escape.open, false);
  assert.equal(h.document.activeElement, outside);
  h.h.unmount();
});
