#!/usr/bin/env node

const { spawnSync } = require('node:child_process');
const path = require('node:path');

const argv = new Set(process.argv.slice(2));

function hasFlag(name) {
  return argv.has(name);
}

function logTitle(title) {
  console.log(`\n=== ${title} ===`);
}

function runStep(step) {
  const cmd = step.command;
  const args = step.args;
  const cwd = step.cwd || process.cwd();

  logTitle(step.name);
  console.log(`$ ${cmd} ${args.join(' ')}`);
  console.log(`cwd: ${cwd}`);

  const result = spawnSync(cmd, args, {
    cwd,
    stdio: 'inherit',
    shell: process.platform === 'win32',
    env: process.env,
  });

  if (typeof result.status === 'number' && result.status !== 0) {
    return {
      ok: false,
      message: `Step failed: ${step.name} (exit=${result.status})`,
    };
  }

  if (result.error) {
    return {
      ok: false,
      message: `Step failed: ${step.name} (${result.error.message})`,
    };
  }

  return { ok: true };
}

const steps = [];

if (!hasFlag('--skip-locales')) {
  steps.push({
    name: 'Locale completeness check',
    command: 'node',
    args: ['scripts/check_locales.cjs'],
  });
}

if (!hasFlag('--skip-typecheck')) {
  steps.push({
    name: 'TypeScript typecheck',
    command: 'npm',
    args: ['run', 'typecheck'],
  });
}

if (!hasFlag('--skip-build')) {
  steps.push({
    name: 'Web build',
    command: 'npm',
    args: ['run', 'build'],
  });
}

if (!hasFlag('--skip-cargo')) {
  steps.push({
    name: 'Rust cargo check',
    command: 'cargo',
    args: ['check'],
    cwd: path.join(process.cwd(), 'src-tauri'),
  });
}

if (!hasFlag('--skip-cargo-test')) {
  steps.push({
    name: 'Rust cargo test (lib)',
    command: 'cargo',
    args: ['test', '--lib'],
    cwd: path.join(process.cwd(), 'src-tauri'),
  });
}

if (steps.length === 0) {
  console.log('No steps enabled. Use without --skip-* flags to run checks.');
  process.exit(0);
}

console.log('Switchme release preflight started.');
console.log(
  'Enabled steps:',
  steps.map((item) => item.name).join(' | ')
);

for (const step of steps) {
  const result = runStep(step);
  if (!result.ok) {
    console.error(`\n[FAILED] ${result.message}`);
    process.exit(1);
  }
}

console.log('\n[OK] Release preflight completed.');
