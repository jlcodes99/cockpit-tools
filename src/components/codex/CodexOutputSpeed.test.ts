import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

const t = (key: string, values?: { value?: string }) => ({
  'common.outputSpeed.label': '速度',
  'common.outputSpeed.value': `≈ ${values?.value} tok/s`,
  'common.outputSpeed.hint': 'approximate observed window',
  'common.outputSpeed.unavailableHint': 'unavailable measurement',
  'common.outputSpeed.unavailableReasons.failed': 'failed request',
  'common.outputSpeed.unavailableReasons.canceled': 'canceled request',
  'common.outputSpeed.unavailableReasons.shortOutput': 'under 100 output tokens',
  'common.outputSpeed.unavailableReasons.missingTiming': 'no observed timing',
}[key] ?? key);

test('speed UI marks measured rates approximate and explains unavailable records', () => {
  const hook = loadHookModule(new URL('./CodexOutputSpeed.tsx', import.meta.url), {});
  const event = { success: true, errorCategory: '', outputTokens: 430, outputGenerationMs: 10000 };
  let tree = hook.render(() => hook.exports.CodexOutputSpeed({ event, t }));
  assert.equal(tree.props.title, 'approximate observed window');
  assert.equal(tree.props.children.join(''), '速度 ≈ 43 tok/s');
  event.success = false;
  tree = hook.flush();
  assert.equal(tree.props.title, 'failed request');
  assert.equal(tree.props.children.join(''), '速度 —');
  hook.unmount();
});

test('unmeasured rates explain short output, missing timing and cancellation in list and detail presentations', () => {
  const hook = loadHookModule(new URL('./CodexOutputSpeed.tsx', import.meta.url), {});
  let event = { success: true, errorCategory: '', outputTokens: 58, outputGenerationMs: null as number | null };
  hook.render(() => hook.exports.CodexOutputSpeed({ event, t, hideLabel: true }));
  assert.equal(hook.flush().props.title, 'under 100 output tokens');
  assert.equal(hook.flush().props.children.join(''), '—');
  event = { ...event, outputTokens: 401 };
  assert.equal(hook.flush().props.title, 'no observed timing');
  event = { ...event, errorCategory: 'client_canceled' };
  assert.equal(hook.flush().props.title, 'canceled request');
  hook.unmount();
});
