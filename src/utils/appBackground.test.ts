import assert from 'node:assert/strict';
import test from 'node:test';
import { appBackgroundStyle, DEFAULT_APP_BACKGROUND, normalizeAppBackground, BACKGROUND_PRESETS } from './appBackground.ts';

test('stored background options stay within the controls and unknown presets use the default', () => {
  assert.deepEqual(normalizeAppBackground(), DEFAULT_APP_BACKGROUND);
  assert.deepEqual(normalizeAppBackground({ preset: 'removed' as never, strength: Infinity, blur: -20, position: 'left' as never }), {
    preset: 'default', strength: 55, blur: 0, position: 'center',
  });
  assert.equal(normalizeAppBackground({ strength: 140, blur: 50 }).strength, 100);
  assert.equal(normalizeAppBackground({ strength: 140, blur: 50 }).blur, 20);
});

test('background rendering affects the backdrop only and custom files are never external URLs', () => {
  for (const preset of BACKGROUND_PRESETS.filter(value => value !== 'custom')) {
    const style = appBackgroundStyle({ ...DEFAULT_APP_BACKGROUND, preset }, null);
    assert.equal(style.backgroundImage === 'none', preset === 'default');
    assert.equal(style.opacity, 0.55);
    assert.equal('color' in style, false);
  }
  assert.equal(appBackgroundStyle({ ...DEFAULT_APP_BACKGROUND, preset: 'custom' }, null).backgroundImage, 'none');
  const style = appBackgroundStyle({ preset: 'custom', strength: 70, blur: 12, position: 'bottom' }, 'blob:local-image');
  assert.equal(style.backgroundImage, 'url("blob:local-image")');
  assert.equal(style.backgroundPosition, 'center bottom');
  assert.equal(style.filter, 'blur(12px)');
});
