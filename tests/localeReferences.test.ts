import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { test } from 'node:test';
const require = createRequire(import.meta.url);
const { requiredTranslationReferences, findMissingTranslationReferences } = require('../scripts/locale_references.cjs');

test('matching locale files still fail when a UI key is missing from every language', () => {
  const references = requiredTranslationReferences("t('codex.deviceIdentity.resetTitle');", 'Card.tsx');
  const missing = findMissingTranslationReferences(references, new Map([
    ['en', { common: { confirm: 'Confirm' } }], ['zh-CN', { common: { confirm: '确认' } }],
  ]));
  assert.equal(missing.length, 1);
  assert.deepEqual(missing[0].languages, ['en', 'zh-CN']);
  assert.equal(missing[0].key, 'codex.deviceIdentity.resetTitle');
});

test('required references cover conditional labels, accessible text and i18n calls', () => {
  const source = "t(retry ? 'common.retry' : 'common.confirm'); i18n.t('common.show'); t('legacy.title', 'Title'); t('legacy.other', {defaultValue: 'Other'});";
  const keys = requiredTranslationReferences(source, 'Card.tsx').map((item: { key: string }) => item.key);
  assert.deepEqual(keys, ['common.retry', 'common.confirm', 'common.show']);
  assert.deepEqual(findMissingTranslationReferences([{ key: 'common.show' }], new Map([['en', { common: { show: 'Show' } }]])), []);
});

test('backend messages include digit-bearing codes and ignore dynamic prefixes', () => {
  const { backendTranslationReferences } = require('../scripts/locale_references.cjs');
  const references = backendTranslationReferences('"codex.proxyQuality.messages.ping0Risk"; "pelican.error.invalidConcurrency"; format!("codex.proxyQuality.summary.{}", status);', 'backend.rs');
  assert.deepEqual(references.map((item: {key: string}) => item.key), ['codex.proxyQuality.messages.ping0Risk', 'pelican.error.invalidConcurrency']);
  assert.equal(findMissingTranslationReferences(references, new Map([['en', {}]])).length, 2);
});
