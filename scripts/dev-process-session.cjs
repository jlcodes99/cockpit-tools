const { execFile, spawn } = require('node:child_process');
const { randomUUID } = require('node:crypto');
const path = require('node:path');
const { promisify } = require('node:util');
const exec = promisify(execFile);
const SESSION_ENV = 'COCKPIT_DEV_SESSION';

function parseUnixProcesses(output) {
  return output.split('\n').flatMap(line => {
    const match = line.match(/^\s*(\d+)\s+(\d+)\s+(\S+)\s+(\S+\s+\S+\s+\d+\s+\S+\s+\d{4})\s+(.+?)\s*$/);
    return match && !match[3].startsWith('Z') ? [{
      pid: Number(match[1]), parent: Number(match[2]), started: match[4], executable: match[5],
    }] : [];
  });
}

async function readProcesses(platform = process.platform) {
  if (platform === 'win32') {
    const script = 'Get-CimInstance Win32_Process | Where-Object { $_.ProcessId -gt 0 -and $_.CreationDate } | ForEach-Object { [pscustomobject]@{ pid=$_.ProcessId; parent=$_.ParentProcessId; started=$_.CreationDate.ToUniversalTime().ToString("o"); executable=$_.ExecutablePath } } | ConvertTo-Json -Compress';
    const { stdout } = await exec('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
      windowsHide: true, timeout: 3000, maxBuffer: 4 * 1024 * 1024, encoding: 'utf8',
    });
    const parsed = JSON.parse(stdout.replace(/^\uFEFF/, ''));
    return (Array.isArray(parsed) ? parsed : [parsed]).filter(row => row && row.pid > 0);
  }
  const { stdout } = await exec('ps', ['-ww', '-axo', 'pid=,ppid=,stat=,lstart=,comm='], {
    timeout: 2000, maxBuffer: 4 * 1024 * 1024, env: { ...process.env, LC_ALL: 'C' },
  });
  return parseUnixProcesses(stdout);
}

function isApplication(entry) {
  // Stop at the app boundary: user-launched Codex/IDE clients must outlive the dev launcher.
  return /^(cockpit-tools|cockpit_tools)(\.exe)?$/i.test(path.win32.basename(entry.executable || ''));
}

function isCompanion(entry) {
  return /^(cockpit-cliproxy(?:-[\w-]+)?|mihomo|sing-box)(\.exe)?$/i.test(path.win32.basename(entry.executable || ''));
}

async function hasSession(entry, token) {
  const { stdout } = await exec('ps', ['eww', '-p', String(entry.pid), '-o', 'command='], {
    timeout: 2000, maxBuffer: 1024 * 1024,
  }).catch(error => {
    if (error.code === 1) return { stdout: '' }; // Process already exited.
    throw error;
  });
  return stdout.split(/\s+/).includes(`${SESSION_ENV}=${token}`);
}

function createDevProcessSession({ rootPid, token, platform = process.platform,
  snapshot = () => readProcesses(platform), matchesSession = hasSession,
  signal = (pid, name) => process.kill(pid, name), pause = ms => new Promise(resolve => setTimeout(resolve, ms)),
  graceMs = 2000, forceMs = 1000 }) {
  const owned = new Map();
  const checked = new Map();
  let pending;
  let stopped = false;
  let cleanup;
  const same = (a, b) => a.started === b.started && a.executable === b.executable;
  const capture = () => pending ??= (async () => {
    const entries = await snapshot();
    const live = new Map(entries.map(entry => [entry.pid, entry]));
    const root = owned.get(rootPid);
    const parents = new Set(!root || (live.has(rootPid) && same(root, live.get(rootPid))) ? [rootPid] : []);
    const apps = new Set();
    for (const [pid, old] of owned) {
      if (live.has(pid) && same(old, live.get(pid))) {
        if (isApplication(old)) apps.add(pid); else parents.add(pid);
      }
    }
    let changed = true;
    while (changed) {
      changed = false;
      for (const entry of entries) {
        if (owned.has(entry.pid) && !same(owned.get(entry.pid), entry)) continue;
        if (entry.pid !== rootPid && !parents.has(entry.parent) && !(apps.has(entry.parent) && isCompanion(entry))) continue;
        if (!owned.has(entry.pid)) owned.set(entry.pid, entry);
        if (isApplication(entry)) apps.add(entry.pid);
        if (!isApplication(entry) && !parents.has(entry.pid)) { parents.add(entry.pid); changed = true; }
      }
    }
    // macOS/Tauri may detach the app before its parent exits. Its inherited random token
    // proves ownership without matching another checkout or an installed app by name.
    if (platform !== 'win32') {
      for (const entry of entries.filter(entry => isApplication(entry) || isCompanion(entry))) {
        if (owned.has(entry.pid) && same(owned.get(entry.pid), entry)) continue;
        const key = `${entry.pid}:${entry.started}`;
        if (!checked.has(key)) checked.set(key, await matchesSession(entry, token));
        if (checked.get(key)) owned.set(entry.pid, entry);
      }
    }
    return entries.filter(entry => owned.has(entry.pid) && same(owned.get(entry.pid), entry));
  })().finally(() => { pending = null; });
  const send = (entries, name) => {
    // Signal descendants first; PID + creation time checks exclude reused PIDs.
    for (const entry of entries.sort((a, b) => Number(a.pid === rootPid) - Number(b.pid === rootPid))) {
      try { signal(entry.pid, name); } catch (error) { if (error.code !== 'ESRCH') throw error; }
    }
  };
  const watcher = setInterval(() => {
    if (!stopped) void capture().catch(() => {}); // Shutdown retries a failed observation.
  }, platform === 'win32' ? 1000 : 500);
  watcher.unref();
  void capture().catch(() => {});
  return {
    capture,
    stop() {
      return cleanup ??= (async () => {
        stopped = true; clearInterval(watcher);
        let entries = await capture();
        const deadline = Date.now() + graceMs;
        do {
          send(entries, 'SIGTERM');
          if (!entries.length) return;
          await pause(100);
          entries = await capture();
        } while (entries.length && Date.now() < deadline);
        send(entries, 'SIGKILL');
        const forceDeadline = Date.now() + forceMs;
        while (entries.length && Date.now() < forceDeadline) {
          await pause(100);
          entries = await capture();
          send(entries, 'SIGKILL');
        }
        if (entries.length) throw new Error('Development processes did not exit within the cleanup deadline');
      })();
    },
  };
}

async function runDevCommand(command, args, { cwd, env = process.env } = {}) {
  const token = randomUUID();
  const child = spawn(command, args, {
    cwd, env: { ...env, [SESSION_ENV]: token }, stdio: 'inherit',
    detached: process.platform !== 'win32', windowsHide: true,
  });
  const session = child.pid ? createDevProcessSession({ rootPid: child.pid, token }) : null;
  return new Promise(resolve => {
    let finishing;
    const finish = code => finishing ??= (async () => {
      try { await session?.stop(); }
      catch (error) {
        console.error(`[tauri-dev] Cleanup failed: ${error.message}`);
        if (child.exitCode === null && child.signalCode === null) child.kill('SIGTERM');
        code = 1;
      } finally {
        process.off('SIGINT', interrupt); process.off('SIGTERM', terminate);
      }
      resolve(code);
    })();
    const interrupt = () => { void finish(130); };
    const terminate = () => { void finish(143); };
    process.on('SIGINT', interrupt); process.on('SIGTERM', terminate);
    child.once('error', error => { console.error(`[tauri-dev] ${error.message}`); void finish(1); });
    child.once('exit', (code, signal) => { void finish(signal === 'SIGINT' ? 130 : signal === 'SIGTERM' ? 143 : code ?? 1); });
  });
}

module.exports = { SESSION_ENV, parseUnixProcesses, createDevProcessSession, runDevCommand };
