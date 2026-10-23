#!/usr/bin/env node
'use strict';

// Run the unchanged Go reference paths before comparing Rust with their fixtures.
// Fixture updates are deliberately unavailable through this verification entry.
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const root = path.resolve(__dirname, '..');
const manifestPath = 'src-tauri/src/modules/codex_native_api/fixtures/parity22_coverage.json';

function readLocal(relative) {
  const absolute = path.resolve(root, relative);
  if (!absolute.startsWith(`${root}${path.sep}`)) {
    throw new Error(`Path escapes repository: ${relative}`);
  }
  return fs.readFileSync(absolute, 'utf8');
}

function verifyCoverage() {
  const manifest = JSON.parse(readLocal(manifestPath));
  if (manifest.schemaVersion !== 1) throw new Error('Unsupported coverage manifest');
  const ids = manifest.coverage.map(item => item.id).sort((a, b) => a - b);
  if (ids.length !== 22 || ids.some((id, index) => id !== index + 1)) {
    throw new Error('Coverage must contain every audit item from 1 through 22 exactly once');
  }
  for (const reference of Object.values(manifest.goReferences)) {
    JSON.parse(readLocal(reference.fixture));
    if (!/^TestParity22\w+Reference$/.test(reference.test)
        || !readLocal(reference.file).includes(`func ${reference.test}(t *testing.T)`)) {
      throw new Error(`Go reference is missing: ${reference.test}`);
    }
  }
  const rustTests = new Set();
  for (const item of manifest.coverage) {
    if (!manifest.goReferences[item.reference] || !item.rustTests.length) {
      throw new Error(`Missing Go/Rust coverage for item ${item.id}`);
    }
    for (const test of item.rustTests) {
      if (!/^parity22_\w+$/.test(test.name)
          || !readLocal(test.file).includes(`fn ${test.name}(`)) {
        throw new Error(`Rust regression is missing: ${test.name}`);
      }
      rustTests.add(test.name);
    }
  }
  return rustTests;
}

function run(command, args, cwd, env) {
  console.log(`\n> ${command} ${args.join(' ')}`);
  const result = spawnSync(command, args, {
    cwd: path.resolve(root, cwd), env, encoding: 'utf8', shell: false,
    timeout: 15 * 60 * 1000, maxBuffer: 64 * 1024 * 1024,
  });
  if (result.stdout) process.stdout.write(result.stdout);
  if (result.stderr) process.stderr.write(result.stderr);
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status ?? result.signal})`);
  return result.stdout;
}

try {
  const expectedTests = verifyCoverage();
  const env = { ...process.env };
  for (const key of Object.keys(env)) {
    if (/^COCKPIT_(?:UPDATE_)?PARITY22/.test(key)) delete env[key];
  }
  delete env.COCKPIT_SKIP_CLIPROXY_BUILD;
  console.log(`Checked all 22 audit items and ${expectedTests.size} Rust regression names.`);
  run('go', ['test', '.', '-run', '^TestParity22.*Reference$', '-count=1'],
    'sidecars/cockpit-cliproxy', env);
  run('go', ['test', './internal/runtime/executor', './sdk/api/handlers/openai',
    './sdk/cliproxy/auth', './internal/client/codex/live',
    '-run', '^TestParity22.*Reference$', '-count=1'],
    'sidecars/cockpit-cliproxy/third_party/CLIProxyAPI', env);
  const output = run('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml',
    '--lib', 'parity22_', '--', '--test-threads=1'], '.', env);
  for (const name of expectedTests) {
    if (!new RegExp(`^test [\\w:]+::${name} \\.\\.\\. ok$`, 'm').test(output)) {
      throw new Error(`Regression did not execute successfully: ${name}`);
    }
  }
  console.log(`\nAll 22 audit items passed Go reference drift checks and Rust differential regressions.`);
} catch (error) {
  console.error(`\nParity verification failed: ${error.message}`);
  process.exitCode = 1;
}
