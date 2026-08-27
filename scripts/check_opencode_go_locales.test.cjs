#!/usr/bin/env node

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const {
  collectLocaleValidation,
  getAllKeys,
  getInterpolationTokens,
} = require('./check_locales.cjs');

const localesDir = path.join(__dirname, '..', 'src', 'locales');
const localeFiles = fs.readdirSync(localesDir).filter((file) => file.endsWith('.json')).sort();
const openCodeGoKeys = [
  'openCodeGo.title',
  'openCodeGo.subtitle',
  'openCodeGo.connections',
  'openCodeGo.emptyProvider',
  'openCodeGo.emptyConnections',
  'openCodeGo.connectionFallback',
  'openCodeGo.errors.authentication',
  'openCodeGo.errors.rateLimit',
  'openCodeGo.errors.network',
  'openCodeGo.errors.unavailable',
  'openCodeGo.errors.configuration',
];

test('OpenCode Go translations are complete in every shipped locale', () => {
  const validation = collectLocaleValidation(localesDir, 'en-US.json');

  assert.deepEqual(validation.parseErrors, []);
  assert.deepEqual(validation.differences, new Map());
  for (const file of localeFiles) {
    const keys = getAllKeys(validation.localeData.get(file).openCodeGo, 'openCodeGo');
    for (const key of openCodeGoKeys) {
      assert.ok(keys.has(key), `${file} is missing ${key}`);
    }
  }
});

test('English fallback validation rejects missing keys and interpolation drift', () => {
  const fixtureDir = fs.mkdtempSync(path.join(process.cwd(), '.tmp-opencode-go-locales-'));
  try {
    fs.writeFileSync(
      path.join(fixtureDir, 'en-US.json'),
      JSON.stringify({ openCodeGo: { title: 'OpenCode Go', connectionFallback: 'Connection {{index}}' } }),
    );
    fs.writeFileSync(
      path.join(fixtureDir, 'fr.json'),
      JSON.stringify({ openCodeGo: { connectionFallback: 'Connexion {{position}}' } }),
    );

    const validation = collectLocaleValidation(fixtureDir, 'en-US.json');
    assert.deepEqual(validation.differences.get('fr.json').missing, ['openCodeGo.title']);
    assert.deepEqual(validation.interpolationIssues, [
      {
        file: 'fr.json',
        key: 'openCodeGo.connectionFallback',
        expected: ['index'],
        actual: ['position'],
      },
    ]);
  } finally {
    fs.rmSync(fixtureDir, { recursive: true, force: true });
  }
});

test('interpolation tokens are normalized and de-duplicated', () => {
  assert.deepEqual(getInterpolationTokens('{{count}} / {{ count }} / {{name}}'), ['count', 'name']);
});
