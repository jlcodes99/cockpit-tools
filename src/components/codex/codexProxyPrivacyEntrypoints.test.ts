import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const entrypoints = [
  'CodexProxyLogsSection.tsx',
  'CodexProxySettingsSection.tsx',
  'CodexProxyTestsSection.tsx',
  'CodexProxyResources.tsx',
  'CodexProxyStrategyPanel.tsx',
] as const;

test('all privacy-sensitive proxy entrypoints use the shared masked account-name hook', () => {
  for (const filename of entrypoints) {
    const source = readFileSync(fileURLToPath(new URL(`./${filename}`, import.meta.url)), 'utf8');
    assert.match(source, /useCodexProxyAccountName/, `${filename} must import/use the shared privacy-aware label`);
    assert.match(source, /const\s+resolveName\s*=\s*useCodexProxyAccountName\(\)/, `${filename} must instantiate the masking hook`);
  }
});
