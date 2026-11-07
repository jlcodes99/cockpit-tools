'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

// Reuse the structural contracts exercised by quota's unit tests, so the
// Chromium handoff fixture cannot silently drift into capability-only mode.
module.exports = function loadQuotaFixture() {
  const source = fs.readFileSync(path.join(__dirname,
    '../../src-tauri/src/modules/codex_app_enhancements/quota.test.cjs'), 'utf8');
  const start = source.indexOf('const sharedBanner = ');
  const end = source.indexOf('\nfunction load()', start);
  assert.ok(start >= 0 && end > start, 'quota module fixtures must remain available');
  return vm.runInNewContext(source.slice(start, end) + '\n({ initial, primary })');
};
