import assert from 'node:assert/strict';
import test from 'node:test';
import jsQR from 'jsqr';
import { buildMfaMigrationQrPages } from './mfaMigrationQr';
import { parseGoogleAuthenticatorMigrationInput } from './mfaVault';

test('the generated SVG actually decodes into an importable migration URI', () => {
  const records = [{ id: 'one', accountName: '示例:alice@example.com', secret: 'JBSWY3DPEHPK3PXP', time: 1 }];
  const [page] = buildMfaMigrationQrPages(records);
  assert.equal(decodeURIComponent(page.dataUrl.split(',').slice(1).join(',')), page.svg);
  // Rasterize the generator's square-module SVG output, then use the app's independent decoder.
  const bounds = page.svg.match(/viewBox="0 0 (\d+) (\d+)"/)!;
  const width = Number(bounds[1]);
  const height = Number(bounds[2]);
  const rgba = new Uint8ClampedArray(width * height * 4).fill(255);
  const modules = [...page.svg.matchAll(/M(\d+),(\d+)l(\d+),0 0,\d+ -\d+,0 0,-\d+z/g)];
  assert.ok(modules.length > 0);
  for (const module of modules) {
    const [x, y, size] = module.slice(1).map(Number);
    for (let dy = 0; dy < size; dy++) for (let dx = 0; dx < size; dx++) {
      const index = ((y + dy) * width + x + dx) * 4;
      rgba[index] = rgba[index + 1] = rgba[index + 2] = 0;
    }
  }
  const decoded = jsQR(rgba, width, height);
  assert.equal(decoded?.data, page.uri);
  assert.deepEqual(parseGoogleAuthenticatorMigrationInput(decoded!.data), [{
    accountName: records[0].accountName, secret: records[0].secret,
  }]);
  assert.ok(!page.svg.includes(records[0].secret));
});
