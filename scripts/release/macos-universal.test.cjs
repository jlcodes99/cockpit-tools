const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const { stageInputs, mergeInputs } = require('./macos-universal.cjs');

const targets = { 'aarch64-apple-darwin': 'arm64', 'x86_64-apple-darwin': 'x86_64' };
const files = ['cockpit-tools', 'cockpit-cliproxy'];

function fakeLipo(args) {
  const read = (file) => JSON.parse(fs.readFileSync(file));
  if (args[0] === '-archs') {
    const value = read(args[1]);
    return value.arch || Object.keys(value.slices).join(' ');
  }
  if (args[0] === '-create') {
    const slices = {};
    for (const file of args.slice(1, args.indexOf('-output'))) {
      slices[read(file).arch] = fs.readFileSync(file).toString('base64');
    }
    fs.writeFileSync(args.at(-1), JSON.stringify({ slices }));
    return '';
  }
  if (args[1] === '-thin') {
    fs.writeFileSync(args.at(-1), Buffer.from(read(args[0]).slices[args[2]], 'base64'));
    return '';
  }
  throw new Error(`Unexpected lipo: ${args}`);
}

function fixture(t) {
  const repoRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'universal-inputs-'));
  t.after(() => fs.rmSync(repoRoot, { recursive: true, force: true }));
  fs.writeFileSync(path.join(repoRoot, 'package.json'), JSON.stringify({ version: '1.2.3' }));
  const options = { repoRoot, version: '1.2.3', commit: 'a'.repeat(40), runId: '12345', inputsDir: path.join(repoRoot, 'inputs') };
  for (const [target, arch] of Object.entries(targets)) {
    const host = path.join(repoRoot, 'target', target, 'release', files[0]);
    const sidecar = path.join(repoRoot, 'sidecars/cockpit-cliproxy/bin', `${files[1]}-${target}`);
    for (const file of [host, sidecar]) {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.writeFileSync(file, JSON.stringify({ arch, value: path.basename(file) }));
    }
    stageInputs({ ...options, target, outputDir: path.join(options.inputsDir, target) }, fakeLipo);
  }
  return options;
}

test('merges the exact host and sidecar slices without invoking a compiler', (t) => {
  const options = fixture(t);
  const outputs = mergeInputs(options, fakeLipo);
  assert.equal(outputs.length, 2);
  for (const output of outputs) {
    assert.deepEqual(fakeLipo(['-archs', output]).split(' ').sort(), ['arm64', 'x86_64']);
    if (process.platform !== 'win32') assert.equal(fs.statSync(output).mode & 0o777, 0o755);
  }
});

for (const [field, value] of [['version', '1.2.4'], ['commit', 'b'.repeat(40)], ['runId', '54321'], ['target', 'wrong-target']]) {
  test(`rejects mixed ${field} before publishing output`, (t) => {
    const options = fixture(t);
    const manifest = path.join(options.inputsDir, 'x86_64-apple-darwin/manifest.json');
    const valueBefore = JSON.parse(fs.readFileSync(manifest));
    fs.writeFileSync(manifest, JSON.stringify({ ...valueBefore, [field]: value }));
    assert.throws(() => mergeInputs(options, fakeLipo), new RegExp(`mismatched ${field}`));
    assert.ok(!fs.existsSync(path.join(options.repoRoot, 'target/universal-apple-darwin')));
  });
}

test('rejects corrupted artifacts and missing architecture inputs', (t) => {
  const options = fixture(t);
  const host = path.join(options.inputsDir, 'x86_64-apple-darwin/cockpit-tools');
  fs.appendFileSync(host, 'corruption');
  assert.throws(() => mergeInputs(options, fakeLipo), /checksum mismatch/);
  fs.rmSync(path.dirname(host), { recursive: true });
  assert.throws(() => mergeInputs(options, fakeLipo), /ENOENT/);
});

test('checks actual architecture even when metadata checksums match', (t) => {
  const options = fixture(t);
  const run = (args) => args[0] === '-archs' ? 'x86_64' : fakeLipo(args);
  assert.throws(() => mergeInputs(options, run), /Wrong architectures/);
});

test('detects a damaged merged slice and preserves previous outputs', (t) => {
  const options = fixture(t);
  const previous = path.join(options.repoRoot, 'target/universal-apple-darwin/release/cockpit-tools');
  fs.mkdirSync(path.dirname(previous), { recursive: true });
  fs.writeFileSync(previous, 'previous valid output');
  const run = (args) => {
    const result = fakeLipo(args);
    if (args[1] === '-thin' && args[0].endsWith('cockpit-cliproxy')) fs.appendFileSync(args.at(-1), 'bad slice');
    return result;
  };
  assert.throws(() => mergeInputs(options, run), /Merged slice mismatch/);
  assert.equal(fs.readFileSync(previous, 'utf8'), 'previous valid output');
});

test('rejects missing release identity and refuses to overwrite staged inputs', (t) => {
  const options = fixture(t);
  const target = 'aarch64-apple-darwin';
  const outputDir = path.join(options.inputsDir, target);
  assert.throws(() => stageInputs({ ...options, target, outputDir, commit: '' }, fakeLipo), /Require an exact version/);
  assert.throws(() => stageInputs({ ...options, target, outputDir }, fakeLipo), /not empty/);
});
