#!/usr/bin/env node
// Reuse this run's two thin release binaries; leave app/DMG bundling to Tauri.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawnSync } = require('node:child_process');
const { parseArgs } = require('./inject-dmg-readme.cjs');

const TARGETS = { 'aarch64-apple-darwin': 'arm64', 'x86_64-apple-darwin': 'x86_64' };
const FILES = ['cockpit-tools', 'cockpit-cliproxy'];

function sha256(file) {
  const hash = crypto.createHash('sha256');
  const buffer = Buffer.alloc(1024 * 1024);
  const fd = fs.openSync(file, 'r');
  try {
    let size;
    while ((size = fs.readSync(fd, buffer)) > 0) hash.update(buffer.subarray(0, size));
    return hash.digest('hex');
  } finally { fs.closeSync(fd); }
}

function lipo(args) {
  const result = spawnSync('lipo', args, { encoding: 'utf8', timeout: 60_000 });
  if (result.error || result.status !== 0) throw new Error(`lipo failed: ${result.error?.message || result.stderr}`);
  return result.stdout.trim();
}

function requireArchitectures(file, expected, run) {
  const actual = run(['-archs', file]).trim().split(/\s+/).sort();
  if (JSON.stringify(actual) !== JSON.stringify([...expected].sort())) {
    throw new Error(`Wrong architectures in ${file}: ${actual.join(', ')}`);
  }
}

function identity({ version, commit, runId }) {
  if (!/^\d+\.\d+\.\d+$/.test(version) || !/^[a-f\d]{40}$/i.test(commit) || !/^\d+$/.test(String(runId))) {
    throw new Error('Require an exact version, 40-character commit SHA and workflow run ID');
  }
  return { schema: 1, version, commit, runId: String(runId) };
}

function stageInputs(options, run = lipo) {
  const { repoRoot, target, outputDir } = options;
  const info = identity(options);
  if (!TARGETS[target]) throw new Error(`Unsupported target: ${target}`);
  if (JSON.parse(fs.readFileSync(path.join(repoRoot, 'package.json'))).version !== info.version) {
    throw new Error('Input version differs from package.json');
  }
  const sources = [path.join(repoRoot, 'target', target, 'release', FILES[0]),
    path.join(repoRoot, 'sidecars/cockpit-cliproxy/bin', `${FILES[1]}-${target}`)];
  for (const source of sources) requireArchitectures(source, [TARGETS[target]], run);
  fs.mkdirSync(outputDir, { recursive: true });
  if (fs.readdirSync(outputDir).length) throw new Error(`Input directory is not empty: ${outputDir}`);
  const files = {};
  for (const [index, name] of FILES.entries()) {
    const destination = path.join(outputDir, name);
    fs.copyFileSync(sources[index], destination);
    fs.chmodSync(destination, 0o755);
    files[name] = sha256(destination);
  }
  fs.writeFileSync(path.join(outputDir, 'manifest.json'), JSON.stringify({ ...info, target, files }, null, 2));
}

function mergeInputs(options, run = lipo) {
  const { repoRoot, inputsDir } = options;
  const expected = identity(options);
  const targetDir = options.targetDir || path.join(repoRoot, 'target');
  const sidecarDir = options.sidecarDir || path.join(repoRoot, 'sidecars/cockpit-cliproxy/bin');
  const inputs = [];
  for (const [target, architecture] of Object.entries(TARGETS)) {
    const directory = path.join(inputsDir, target);
    const manifest = JSON.parse(fs.readFileSync(path.join(directory, 'manifest.json')));
    for (const [key, value] of Object.entries({ ...expected, target })) {
      if (manifest[key] !== value) throw new Error(`Input ${target} has mismatched ${key}`);
    }
    for (const name of FILES) {
      const file = path.join(directory, name);
      if (fs.lstatSync(file).isSymbolicLink() || sha256(file) !== manifest.files?.[name]) {
        throw new Error(`Input checksum mismatch: ${file}`);
      }
      requireArchitectures(file, [architecture], run);
    }
    inputs.push({ directory, architecture });
  }
  const releaseDir = path.join(targetDir, 'universal-apple-darwin/release');
  fs.mkdirSync(releaseDir, { recursive: true });
  fs.mkdirSync(sidecarDir, { recursive: true });
  const work = fs.mkdtempSync(path.join(releaseDir, '.merge-'));
  const outputs = [path.join(releaseDir, FILES[0]), path.join(sidecarDir, `${FILES[1]}-universal-apple-darwin`)];
  try {
    // Check both merged binaries, including each original slice, before publishing either.
    for (const name of FILES) {
      const output = path.join(work, name);
      run(['-create', ...inputs.map((input) => path.join(input.directory, name)), '-output', output]);
      requireArchitectures(output, ['arm64', 'x86_64'], run);
      for (const input of inputs) {
        const slice = path.join(work, `${name}-${input.architecture}`);
        run([output, '-thin', input.architecture, '-output', slice]);
        if (sha256(slice) !== sha256(path.join(input.directory, name))) {
          throw new Error(`Merged slice mismatch: ${name}/${input.architecture}`);
        }
      }
      fs.chmodSync(output, 0o755);
    }
    for (const [index, name] of FILES.entries()) fs.renameSync(path.join(work, name), outputs[index]);
  } finally { fs.rmSync(work, { recursive: true, force: true }); }
  return outputs;
}

if (require.main === module) {
  try {
    if (process.platform !== 'darwin') throw new Error('macOS is required');
    const args = parseArgs(process.argv.slice(2));
    const options = {
      repoRoot: path.resolve(__dirname, '../..'), version: args.version,
      commit: process.env.GITHUB_SHA, runId: process.env.GITHUB_RUN_ID,
    };
    if (args.mode === 'stage' && args.target && args.output) {
      stageInputs({ ...options, target: args.target, outputDir: path.resolve(args.output) });
    } else if (args.mode === 'merge' && args.inputs) {
      mergeInputs({ ...options, inputsDir: path.resolve(args.inputs) });
    } else throw new Error('Require --mode stage --target <triple> --output <dir>, or --mode merge --inputs <dir>, plus --version');
  } catch (error) { console.error(error); process.exitCode = 1; }
}

module.exports = { stageInputs, mergeInputs };
