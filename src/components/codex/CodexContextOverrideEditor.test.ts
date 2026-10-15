import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import vm from 'node:vm';
import ts from 'typescript';

// Exercise the real component's handlers and rendered branches without a browser
// or an additional renderer dependency. Only hooks and imported UI are stubbed.
type Values = { enabled: boolean; contextWindow: string; compactLimit: string };
type Element = { type: unknown; props: Record<string, any> };
function editor(initial: Values) {
  let values = { ...initial };
  let customEditing = false;
  let effect: (() => void) | undefined;
  let previousEnabled: boolean | undefined;
  const exports: Record<string, any> = {};
  const require = createRequire(import.meta.url);
  const source = readFileSync(new URL('./CodexContextOverrideEditor.tsx', import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, { compilerOptions: {
    module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX,
  } }).outputText;
  vm.runInNewContext(compiled, { exports, require: (name: string) => {
    if (name === 'react') return {
      useMemo: (factory: () => unknown) => factory(),
      useState: () => [customEditing, (value: boolean) => { customEditing = value; }],
      useEffect: (callback: () => void, deps: boolean[]) => {
        if (previousEnabled !== deps[0]) { effect = callback; previousEnabled = deps[0]; }
      },
    };
    if (name === 'react-i18next') return { useTranslation: () => ({ t: (key: string) => key }) };
    if (name === '../SingleSelectDropdown') return { SingleSelectDropdown: 'dropdown' };
    return require(name);
  } });
  function render() {
    const root: Element = exports.CodexContextOverrideEditor({ ...values,
      onChange: (next: Values) => { values = { ...next }; },
    });
    const pendingEffect = effect;
    effect = undefined;
    pendingEffect?.();
    const [dropdown, fields] = root.props.children;
    return { dropdown: dropdown as Element, fields: fields as Element | false };
  }
  return {
    render,
    select(value: string) { render().dropdown.props.onChange(value); return render(); },
    values: () => values,
    update(next: Partial<Values>) { values = { ...values, ...next }; return render(); },
  };
}

for (const [contextWindow, compactLimit] of [['516000', '464400'], ['1000000', '900000']]) {
  test(`custom editing opens and preserves preset ${contextWindow}/${compactLimit}`, () => {
    const control = editor({ enabled: true, contextWindow, compactLimit });
    assert.equal(control.render().fields, false);
    const selected = control.select('custom');
    assert.equal(selected.dropdown.props.value, 'custom');
    assert.ok(selected.fields);
    assert.deepEqual(control.values(), { enabled: true, contextWindow, compactLimit });
    assert.ok(control.render().fields);
    assert.equal(control.select('official').fields, false);
    assert.ok(control.select('custom').fields);
    assert.equal(control.select('preset_1m').dropdown.props.value, 'preset_1m');
    assert.equal(control.render().fields, false);
  });
}

test('editing values into an exact preset does not collapse custom inputs', () => {
  const control = editor({ enabled: true, contextWindow: '1000000', compactLimit: '850000' });
  const fields = control.render().fields;
  assert.ok(fields);
  const compactInput: Element = fields.props.children[1].props.children[1];
  compactInput.props.onChange({ target: { value: '900000' } });
  assert.equal(control.values().compactLimit, '900000');
  assert.equal(control.render().dropdown.props.value, 'custom');
  assert.ok(control.render().fields);
  control.update({ enabled: false });
  assert.equal(control.update({ enabled: true }).dropdown.props.value, 'preset_1m');
});

test('empty official values enter custom editing without inventing defaults', () => {
  const control = editor({ enabled: false, contextWindow: '', compactLimit: '' });
  assert.ok(control.select('custom').fields);
  assert.deepEqual(control.values(), { enabled: true, contextWindow: '', compactLimit: '' });
});

test('typing only a context window derives the 90% compact limit', () => {
  const control = editor({ enabled: false, contextWindow: '', compactLimit: '' });
  const fields = control.select('custom').fields;
  assert.ok(fields);
  const contextInput: Element = fields.props.children[0].props.children[1];
  contextInput.props.onChange({ target: { value: '272000' } });
  assert.deepEqual(control.values(), { enabled: true, contextWindow: '272000', compactLimit: '244800' });
  contextInput.props.onChange({ target: { value: '516000' } });
  assert.deepEqual(control.values(), { enabled: true, contextWindow: '516000', compactLimit: '464400' });
});

test('explicit compact limit stays untouched while context changes', () => {
  const control = editor({ enabled: true, contextWindow: '516000', compactLimit: '450000' });
  const fields = control.render().fields;
  assert.ok(fields);
  const contextInput: Element = fields.props.children[0].props.children[1];
  contextInput.props.onChange({ target: { value: '1000000' } });
  assert.deepEqual(control.values(), { enabled: true, contextWindow: '1000000', compactLimit: '450000' });
});

test('clearing the compact limit falls back to the derived 90% value', () => {
  const control = editor({ enabled: true, contextWindow: '516000', compactLimit: '450000' });
  const fields = control.render().fields;
  assert.ok(fields);
  const compactInput: Element = fields.props.children[1].props.children[1];
  compactInput.props.onChange({ target: { value: '' } });
  assert.deepEqual(control.values(), { enabled: true, contextWindow: '516000', compactLimit: '464400' });
});
