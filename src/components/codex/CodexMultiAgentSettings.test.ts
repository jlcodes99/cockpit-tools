import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import test from 'node:test';

// These capabilities are automatic backend behavior. No settings entrance should
// reintroduce a component that waits for a read or writes an opt-out flag.
test('automatic compatibility and quota sync have no per-entrance enable controls', () => {
  for (const file of ['../../pages/SettingsCodexPlatformPanel.tsx', '../../pages/CodexApiServiceView.tsx', '../CodexLocalAccessModal.tsx', '../QuickSettingsPopover.tsx']) {
    const source = readFileSync(new URL(file, import.meta.url), 'utf8');
    assert.doesNotMatch(source, /CodexMultiAgentSettings|CodexCapabilityFallbackSettings|CodexQuotaSyncSettingsControl/);
  }
  for (const name of ['CodexMultiAgentSettings', 'CodexCapabilityFallbackSettings', 'CodexQuotaSyncSettingsControl']) {
    assert.equal(existsSync(new URL(`./${name}.tsx`, import.meta.url)), false, `${name} must not remain as a dormant opt-out editor`);
  }
});
