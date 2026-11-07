const assert = require('node:assert/strict');
const { spawn, execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');
const { createDevProcessSession, parseUnixProcesses } = require('./dev-process-session.cjs');

const entry = (pid, parent, executable = '/bin/node', started = 'Mon Oct 26 12:00:00 2026') => ({ pid, parent, executable, started });

test('process parsing preserves paths with spaces and excludes zombies', () => {
  assert.deepEqual(parseUnixProcesses(' 101 1 S Mon Oct 26 12:00:00 2026 /Applications/Cockpit Tools.app/Contents/MacOS/cockpit-tools\n 102 1 Z Mon Oct 26 12:00:01 2026 /bin/node'), [
    entry(101, 1, '/Applications/Cockpit Tools.app/Contents/MacOS/cockpit-tools'),
  ]);
});

test('cleanup owns only its tree and tagged orphan; user clients and another installed app survive', async () => {
  let entries = [entry(100, 1), entry(101, 100), entry(102, 101, '/repo/target/debug/cockpit-tools'),
    entry(103, 102, '/Applications/ChatGPT.app/Contents/MacOS/Codex'),
    entry(104, 1, '/repo/target/debug/cockpit-tools'), entry(105, 1, '/Applications/Cockpit Tools.app/Contents/MacOS/cockpit-tools'),
    entry(106, 102, '/repo/sidecars/cockpit-cliproxy/bin/cockpit-cliproxy')];
  const signals = [];
  const session = createDevProcessSession({ rootPid: 100, token: 'mine',
    snapshot: async () => entries,
    matchesSession: async value => value.pid === 104,
    signal: (pid, name) => { signals.push([pid, name]); entries = entries.filter(value => value.pid !== pid); },
    pause: async () => {},
  });
  await session.capture();
  // Parent gone and app moved into another process group: remembered ownership remains valid.
  entries = entries.filter(value => value.pid !== 101).map(value => value.pid === 102 ? { ...value, parent: 1 } : value);
  const first = session.stop();
  assert.equal(session.stop(), first);
  await first;
  assert.deepEqual(signals.map(value => value[0]).sort((a, b) => a - b), [100, 102, 104, 106]);
  assert.deepEqual(entries.map(value => value.pid), [103, 105]);
});

test('reused process IDs are never signalled or used to adopt another process tree', async () => {
  let entries = [entry(100, 1), entry(101, 100)];
  const session = createDevProcessSession({ rootPid: 100, token: 'mine', snapshot: async () => entries,
    matchesSession: async () => false, signal: () => assert.fail('PID was reused'), pause: async () => {} });
  await session.capture();
  entries = [entry(100, 1, '/bin/node', 'Tue Oct 27 12:00:00 2026'), entry(101, 100, '/bin/node', 'Tue Oct 27 12:00:00 2026'), entry(102, 101)];
  await session.stop();
});

test('unresponsive descendants receive a bounded force-stop', async () => {
  let entries = [entry(100, 1), entry(101, 100)];
  const signals = [];
  const session = createDevProcessSession({ rootPid: 100, token: 'mine', snapshot: async () => entries,
    matchesSession: async () => false, graceMs: 0, pause: async () => {},
    signal: (pid, name) => { signals.push(name); if (name === 'SIGKILL') entries = entries.filter(value => value.pid !== pid); } });
  await session.stop();
  assert.ok(signals.includes('SIGTERM'));
  assert.ok(signals.includes('SIGKILL'));
});

test('Windows tree ownership does not need environment inspection and stops at application boundaries', async () => {
  let entries = [entry(100, 1), entry(101, 100, 'C:\\repo\\target\\debug\\cockpit-tools.exe'),
    entry(102, 101, 'C:\\Apps\\Codex.exe'), entry(103, 101, 'C:\\repo\\cockpit-cliproxy.exe')];
  const session = createDevProcessSession({ rootPid: 100, token: 'mine', platform: 'win32', snapshot: async () => entries,
    matchesSession: async () => assert.fail('Windows must not call ps'), pause: async () => {},
    signal: pid => { entries = entries.filter(value => value.pid !== pid); } });
  await session.stop();
  assert.deepEqual(entries.map(value => value.pid), [102]);
});

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
function running(pid) {
  try { process.kill(pid, 0); return true; } catch (error) { if (error.code === 'ESRCH') return false; throw error; }
}

for (const mode of ['interrupt', 'natural-exit']) {
  test(`real headless processes: ${mode} reaps a detached, SIGTERM-resistant descendant`, { skip: process.platform === 'win32', timeout: 12000 }, async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-dev-process-test-'));
    const pidFile = path.join(dir, 'pids.json');
    const cliFile = path.join(dir, 'cli.cjs');
    const supervisorFile = path.join(dir, 'supervisor.cjs');
    fs.writeFileSync(cliFile, `
      const { spawn } = require('node:child_process');
      const worker = spawn(process.execPath, ['-e', "process.on('SIGTERM', () => {}); setInterval(() => {}, 1000)"], { detached: true, stdio: 'ignore' });
      worker.unref();
      require('node:fs').writeFileSync(${JSON.stringify(pidFile)}, JSON.stringify([process.pid, worker.pid]));
      ${mode === 'natural-exit' ? 'setTimeout(() => process.exit(7), 1400);' : "process.on('SIGTERM', () => {}); setInterval(() => {}, 1000);"}
    `);
    fs.writeFileSync(supervisorFile, `
      const { runDevCommand } = require(${JSON.stringify(require.resolve('./dev-process-session.cjs'))});
      runDevCommand(process.execPath, [${JSON.stringify(cliFile)}]).then(code => { process.exitCode = code; });
    `);
    const supervisor = spawn(process.execPath, [supervisorFile], { stdio: ['ignore', 'pipe', 'pipe'] });
    let errors = '';
    supervisor.stderr.on('data', data => { errors += data; });
    const exited = new Promise(resolve => supervisor.once('exit', (code, signal) => resolve({ code, signal })));
    let pids = [];
    try {
      const deadline = Date.now() + 4000;
      while (!fs.existsSync(pidFile) && Date.now() < deadline) await sleep(30);
      pids = JSON.parse(fs.readFileSync(pidFile, 'utf8'));
      await sleep(900); // At least one ownership sample before terminating the parent.
      if (mode === 'interrupt') supervisor.kill('SIGINT');
      assert.deepEqual(await exited, { code: mode === 'interrupt' ? 130 : 7, signal: null }, errors);
      assert.equal(running(pids[0]), false);
      assert.equal(running(pids[1]), false);
    } finally {
      for (const pid of [...pids, supervisor.pid]) { try { process.kill(pid, 'SIGKILL'); } catch {} }
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
}

test('an immediately orphaned app is found by its real session token, leaving an untagged app running', { skip: process.platform === 'win32', timeout: 10000 }, async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-dev-orphan-test-'));
  const executable = path.join(dir, 'cockpit-tools');
  fs.copyFileSync('/bin/sleep', executable);
  fs.chmodSync(executable, 0o755);
  // A relocated Apple platform binary needs a local signature before it can run reliably.
  if (process.platform === 'darwin') execFileSync('codesign', ['--force', '--sign', '-', executable], { stdio: 'ignore' });
  const unrelated = spawn(executable, ['30'], { stdio: 'ignore' });
  const pidFile = path.join(dir, 'orphan.pid');
  const cliFile = path.join(dir, 'cli.cjs');
  const supervisorFile = path.join(dir, 'supervisor.cjs');
  fs.writeFileSync(cliFile, `
    const worker = require('node:child_process').spawn(${JSON.stringify(executable)}, ['30'], { detached: true, stdio: 'ignore' });
    require('node:fs').writeFileSync(${JSON.stringify(pidFile)}, String(worker.pid));
    worker.unref(); process.exit(0);
  `);
  fs.writeFileSync(supervisorFile, `
    require(${JSON.stringify(require.resolve('./dev-process-session.cjs'))}).runDevCommand(process.execPath, [${JSON.stringify(cliFile)}])
      .then(code => { process.exitCode = code; });
  `);
  const supervisor = spawn(process.execPath, [supervisorFile], { stdio: 'ignore' });
  let orphanPid;
  try {
    const result = await new Promise(resolve => supervisor.once('exit', (code, signal) => resolve({ code, signal })));
    orphanPid = Number(fs.readFileSync(pidFile, 'utf8'));
    assert.deepEqual(result, { code: 0, signal: null });
    assert.equal(running(orphanPid), false);
    assert.equal(running(unrelated.pid), true);
  } finally {
    for (const pid of [orphanPid, unrelated.pid, supervisor.pid].filter(Boolean)) { try { process.kill(pid, 'SIGKILL'); } catch {} }
    fs.rmSync(dir, { recursive: true, force: true });
  }
});
