import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}

test('speed picker offers three explicit choices without changing the default', () => {
  for (const compact of [false, true]) {
    const calls: string[] = [];
    const h = loadHookModule(new URL('./CodexSpeedSelect.tsx', import.meta.url), {
      'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
      'react-dom': { createPortal: (element: any) => element },
      'lucide-react': { Check: 'Check', ChevronDown: 'ChevronDown', Zap: 'Zap' },
    }, {
      setTimeout, clearTimeout,
      window: { innerHeight: 600, innerWidth: 400, setTimeout, clearTimeout, addEventListener() {}, removeEventListener() {} },
      document: { body: {}, addEventListener() {}, removeEventListener() {} },
    });
    const props: any = { compact, onChange: (value: string) => calls.push(value) };
    let tree = h.render(() => h.exports.CodexSpeedSelect(props));
    const trigger = nodes(tree).find((node) => node.props?.className?.startsWith('codex-speed-trigger'));
    assert.match(trigger.props.className, /standard/);
    assert.deepEqual(calls, []);
    trigger.props.ref.current = { getBoundingClientRect: () => ({ top: 560, bottom: 582, right: 390, width: 80 }) };
    trigger.props.onClick(); tree = h.flush();
    const options = nodes(tree).filter((node) => node.type === 'button' && node.props.className?.startsWith('codex-speed-option '));
    assert.deepEqual(options.map((option) => option.key), ['standard', 'fast', 'ultrafast']);
    const menu = nodes(tree).find((node) => node.props?.className?.startsWith('codex-speed-menu placement-'));
    assert.ok(Number.parseFloat(menu.props.style.top) >= 8);
    assert.ok(Number.parseFloat(menu.props.style.left) + Number.parseFloat(menu.props.style.width) <= 392);
    options[2].props.onClick(); h.flush();
    assert.deepEqual(calls, ['ultrafast']);
    props.value = 'ultrafast'; tree = h.render(() => h.exports.CodexSpeedSelect(props));
    assert.match(nodes(tree).find((node) => node.props?.className?.startsWith('codex-speed-trigger')).props.className, /ultrafast/);
    assert.equal(nodes(tree).filter((node) => node.type === 'Zap').length, 1);
    h.unmount();
  }
});
