#!/usr/bin/env node
'use strict';

// Observe the selected Codex CLI against synthetic loopback responses only.
// This script never opens the user's normal Codex home or authenticates upstream.
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const crypto = require('node:crypto');
const { spawn, spawnSync } = require('node:child_process');

const usage = 'Usage: node scripts/tests/verify-client-retry-policy.cjs --codex <absolute CLI path> --output <absolute new or empty dedicated directory>';
function options(argv) {
  const result = {};
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i];
    if (!['--codex', '--output'].includes(key) || !argv[i + 1] || result[key]) throw new Error(usage);
    result[key] = argv[i + 1];
  }
  if (!result['--codex'] || !result['--output']) throw new Error(usage);
  for (const value of Object.values(result)) if (!path.isAbsolute(value)) throw new Error(`Both paths must be absolute. ${usage}`);
  return { binary: path.resolve(result['--codex']), output: path.resolve(result['--output']) };
}
function assertPlainPath(target) {
  // lstat catches symlinks and Windows junctions; native realpath also catches
  // redirected ancestors. Windows additionally rejects every reparse attribute.
  let cursor = path.resolve(target);
  for (;;) {
    if (fs.existsSync(cursor)) {
      if (fs.lstatSync(cursor).isSymbolicLink()) throw new Error(`Linked path refused: ${cursor}`);
      const actual = fs.realpathSync.native(cursor);
      const equal = process.platform === 'win32' ? actual.toLowerCase() === cursor.toLowerCase() : actual === cursor;
      if (!equal) throw new Error(`Redirected path refused: ${cursor}`);
    }
    const parent = path.dirname(cursor);
    if (parent === cursor) break;
    cursor = parent;
  }
  if (process.platform === 'win32') {
    if (!process.env.SystemRoot) throw new Error('SystemRoot is required to verify Windows reparse attributes');
    const shell = path.join(process.env.SystemRoot, 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe');
    const command = "$ErrorActionPreference='Stop'; $json=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String(($input | Out-String))); $p=[IO.Path]::GetFullPath(($json | ConvertFrom-Json).path); while($p){if((Test-Path -LiteralPath $p) -and (([IO.File]::GetAttributes($p) -band [IO.FileAttributes]::ReparsePoint) -ne 0)){throw 'Reparse point refused'}; $p=[IO.Path]::GetDirectoryName($p)}";
    const check = spawnSync(shell, ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(command, 'utf16le').toString('base64')], {
      input: Buffer.from(JSON.stringify({ path: target }), 'utf8').toString('base64'), encoding: 'utf8', timeout: 10000, windowsHide: true,
    });
    if (check.error || check.status !== 0) throw new Error(`Windows reparse check failed for the supplied path: ${check.error?.message || check.stderr.trim()}`);
  }
}
function emptyEnvironment(home) {
  const env = {};
  for (const key of ['SystemRoot', 'WINDIR', 'ComSpec', 'PATH', 'PATHEXT', 'PROCESSOR_ARCHITECTURE', 'NUMBER_OF_PROCESSORS']) {
    if (process.env[key]) env[key] = process.env[key];
  }
  Object.assign(env, {
    HOME: home, USERPROFILE: home, CODEX_HOME: path.join(home, '.codex'),
    APPDATA: path.join(home, 'AppData', 'Roaming'), LOCALAPPDATA: path.join(home, 'AppData', 'Local'),
    TEMP: path.join(home, 'temp'), TMP: path.join(home, 'temp'), NO_PROXY: '127.0.0.1,localhost',
  });
  return env;
}
function sha256(buffer) { return crypto.createHash('sha256').update(buffer).digest('hex'); }
function sse(type, payload) { return `event: ${type}\ndata: ${JSON.stringify({ type, ...payload })}\n\n`; }
function success(res) {
  const item = { id: 'msg_synthetic', type: 'message', status: 'completed', role: 'assistant', content: [{ type: 'output_text', text: 'Synthetic complete.', annotations: [] }] };
  res.end(sse('response.created', { sequence_number: 0, response: { id: 'resp_success', object: 'response', status: 'in_progress', output: [] } }) +
    sse('response.output_item.added', { sequence_number: 1, output_index: 0, item: { ...item, status: 'in_progress', content: [] } }) +
    sse('response.content_part.added', { sequence_number: 2, output_index: 0, item_id: item.id, content_index: 0, part: { type: 'output_text', text: '', annotations: [] } }) +
    sse('response.output_text.delta', { sequence_number: 3, output_index: 0, item_id: item.id, content_index: 0, delta: 'Synthetic complete.' }) +
    sse('response.output_text.done', { sequence_number: 4, output_index: 0, item_id: item.id, content_index: 0, text: 'Synthetic complete.' }) +
    sse('response.content_part.done', { sequence_number: 5, output_index: 0, item_id: item.id, content_index: 0, part: item.content[0] }) +
    sse('response.output_item.done', { sequence_number: 6, output_index: 0, item }) +
    sse('response.completed', { sequence_number: 7, response: { id: 'resp_success', object: 'response', status: 'completed', output: [item], usage: { input_tokens: 1, output_tokens: 3, total_tokens: 4 } } }));
}
const children = new Set();
const servers = new Set();
const watchers = new Set();
let interrupted = false;
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
const watcherSource = String.raw`
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
function Emit($value){[Console]::WriteLine(($value|ConvertTo-Json -Compress -Depth 8));[Console]::Out.Flush()}
function Identity($item,$depth){
  $proc=$null
  try{
    $proc=[Diagnostics.Process]::GetProcessById([int]$item.ProcessId)
    $ticks=$proc.StartTime.ToUniversalTime().Ticks.ToString()
    $text=if($null -eq $item.CommandLine){''}else{[string]$item.CommandLine}
    $sha=[Security.Cryptography.SHA256]::Create()
    try{$hash=([BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($text)))).Replace('-','')}finally{$sha.Dispose()}
    return @{pid=[int]$item.ProcessId;parent=[int]$item.ParentProcessId;created=$item.CreationDate.ToUniversalTime().Ticks.ToString();start=$ticks;path=[string]$item.ExecutablePath;commandHash=$hash;depth=$depth}
  }catch{return $null}finally{if($proc){$proc.Dispose()}}
}
function Lookup($number){Get-CimInstance Win32_Process -Filter ('ProcessId = '+[int]$number) -ErrorAction Stop}
function Same($a,$b){$a -and $b -and $a.pid -eq $b.pid -and $a.parent -eq $b.parent -and $a.created -eq $b.created -and $a.start -eq $b.start -and $a.path -eq $b.path -and $a.commandHash -eq $b.commandHash}
$owned=@{};$pending=@{};$races=[Collections.Generic.List[object]]::new();$errors=[Collections.Generic.List[string]]::new();$killed=[Collections.Generic.List[int]]::new();$mismatches=[Collections.Generic.List[int]]::new()
function ResolvePending {
  foreach($key in @($pending.Keys)){
    $candidate=$pending[$key];$item=Lookup $candidate.pid
    if(-not $item -or $item.CreationDate.ToUniversalTime().Ticks.ToString() -ne $candidate.created){
      $races.Add(@{pid=$candidate.pid;created=$candidate.created;resolution='exited-before-full-identity'});$pending.Remove($key);continue
    }
    $identity=Identity $item $candidate.depth
    if($identity){$owned[$key]=$identity;$pending.Remove($key);Emit @{type='observed';identity=$identity}}
  }
}
function Sample {
  ResolvePending
  foreach($known in @($owned.Values)){
    $item=Lookup $known.pid
    if(-not $item){continue}
    if(-not (Same $known (Identity $item $known.depth))){
      $again=Lookup $known.pid
      if($again -and $again.CreationDate.ToUniversalTime().Ticks.ToString() -eq $known.created -and -not $mismatches.Contains($known.pid)){$mismatches.Add($known.pid)}
      continue
    }
    foreach($child in @(Get-CimInstance Win32_Process -Filter ('ParentProcessId = '+$known.pid) -ErrorAction Stop)){
      if($child.CreationDate.ToUniversalTime().Ticks -lt [long]$known.created){continue}
      $identity=Identity $child ($known.depth+1)
      if(-not $identity){
        $candidate=@{pid=[int]$child.ProcessId;parent=[int]$child.ParentProcessId;created=$child.CreationDate.ToUniversalTime().Ticks.ToString();depth=$known.depth+1}
        $pending[$candidate.pid.ToString()+':'+$candidate.created]=$candidate;continue
      }
      $key=$identity.pid.ToString()+':'+$identity.created
      if(-not $owned.ContainsKey($key)){$owned[$key]=$identity;Emit @{type='observed';identity=$identity}}
    }
  }
}
Import-Module CimCmdlets -ErrorAction Stop
[void](Lookup $PID)
Emit @{type='ready'}
$line=[Console]::ReadLine()
if(-not $line){exit 1}
$registration=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($line))|ConvertFrom-Json
$item=Lookup $registration.pid
$root=if($item){Identity $item 0}else{$null}
if(-not $root -or $root.parent -ne $registration.parent -or [long]$root.created -lt [datetime]::Parse($registration.after).ToUniversalTime().Ticks){Emit @{type='final';complete=$false;error='Owned CLI identity could not be established';observed=@();killed=@();survivors=@()};exit 1}
$owned[$root.pid.ToString()+':'+$root.created]=$root
Emit @{type='observed';identity=$root}
$until=[datetime]::UtcNow.AddSeconds(35)
try{
  while([datetime]::UtcNow -lt $until){Sample;if(Test-Path -LiteralPath $registration.stopFile){break};Start-Sleep -Milliseconds 120}
  Sample
  # Stop deepest recorded descendants first, then re-sample any still-live
  # recorded parent. Never kill a PID whose complete identity has changed.
  for($round=0;$round -lt 2;$round++){
    foreach($known in @($owned.Values|Where-Object {$_.depth -gt 0}|Sort-Object depth -Descending)){
      $item=Lookup $known.pid
      if(-not $item){continue}
      if(-not (Same $known (Identity $item $known.depth))){$mismatches.Add($known.pid);continue}
      $proc=$null
      try{
        $proc=[Diagnostics.Process]::GetProcessById($known.pid)
        [void]$proc.Handle
        if($proc.StartTime.ToUniversalTime().Ticks.ToString() -ne $known.start){$mismatches.Add($known.pid);continue}
        $proc.Kill();[void]$proc.WaitForExit(1000);$killed.Add($known.pid)
      }catch{$errors.Add('Owned descendant cleanup failed: '+$known.pid)}finally{if($proc){$proc.Dispose()}}
    }
    Sample
  }
  ResolvePending
  $survivors=@(foreach($known in $owned.Values){$item=Lookup $known.pid;if($item -and (Same $known (Identity $item $known.depth))){$known.pid}})
  Emit @{type='final';complete=($survivors.Count -eq 0 -and $errors.Count -eq 0 -and $mismatches.Count -eq 0 -and $pending.Count -eq 0);observed=@($owned.Values);killed=@($killed);survivors=$survivors;identityMismatches=@($mismatches);errors=@($errors);unresolvedIdentities=@($pending.Values);observationRaces=@($races)}
}catch{Emit @{type='final';complete=$false;error=$_.Exception.Message;observed=@($owned.Values);killed=@($killed)}}
`;
async function startWatcher(home) {
  if (process.platform !== 'win32') return null;
  const shell = path.join(process.env.SystemRoot, 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe');
  const helper = spawn(shell, ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(watcherSource, 'utf16le').toString('base64')], {
    env: emptyEnvironment(home), cwd: home, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'],
  });
  const state = { helper, records: [], final: null, exited: false, errors: '', registered: false, stopFile: path.join(home, `.watcher-stop-${crypto.randomBytes(8).toString('hex')}`) };
  watchers.add(state);
  let buffer = '', ready = false;
  helper.stdout.on('data', bytes => {
    buffer += bytes.toString();
    for (;;) {
      const newline = buffer.indexOf('\n'); if (newline < 0) break;
      const line = buffer.slice(0, newline).trim(); buffer = buffer.slice(newline + 1);
      try { const record = JSON.parse(line); state.records.push(record); if (record.type === 'ready') ready = true; if (record.type === 'final') state.final = record; } catch { state.errors += line; }
    }
  });
  helper.stderr.on('data', bytes => { state.errors += bytes.toString(); });
  helper.stdin.on('error', error => { state.errors += error.message; });
  helper.on('error', error => { state.errors += error.message; });
  helper.on('exit', () => { state.exited = true; });
  const until = Date.now() + 8000;
  while (!ready && !state.exited && Date.now() < until) await wait(25);
  if (!ready) { await finishWatcher(state); throw new Error(`Owned descendant watcher did not become ready: ${state.errors}`); }
  return state;
}
async function finishWatcher(state) {
  if (!state) return { complete: false, unsupported: 'Process ancestry validation is Windows-only' };
  if (!state.exited) {
    if (state.registered && !fs.existsSync(state.stopFile)) fs.writeFileSync(state.stopFile, 'cleanup', { flag: 'wx' });
    if (!state.helper.stdin.destroyed) state.helper.stdin.end();
  }
  const until = Date.now() + 8000;
  while (!state.exited && Date.now() < until) await wait(25);
  if (!state.exited) { state.helper.kill('SIGKILL'); await wait(100); }
  state.helper.stdout.destroy(); state.helper.stderr.destroy(); state.helper.stdin.destroy();
  if (state.exited) watchers.delete(state);
  return { ...(state.final || { complete: false, error: 'No final descendant cleanup receipt' }), watcherExited: state.exited, watcherError: state.errors, observations: state.records.filter(x => x.type === 'observed').map(x => x.identity) };
}
function interrupt() {
  interrupted = true;
  for (const child of children) if (child.exitCode === null && child.signalCode === null) child.kill();
  for (const { server, sockets } of servers) { server.close(); for (const socket of sockets) socket.destroy(); }
}
async function finishOwnedProcesses() {
  await Promise.all([...children].map(child => new Promise(resolve => {
    if (child.exitCode !== null || child.signalCode !== null) { children.delete(child); child.stdout?.destroy(); child.stderr?.destroy(); resolve(); return; }
    const force = setTimeout(() => child.kill('SIGKILL'), 2000);
    const stopWaiting = setTimeout(() => { clearTimeout(force); resolve(); }, 5000);
    child.once('exit', () => { clearTimeout(force); clearTimeout(stopWaiting); children.delete(child); child.stdout?.destroy(); child.stderr?.destroy(); resolve(); });
    child.kill();
  })));
}

async function main() {
  // Validate all arguments before creating output or starting the selected CLI.
  const { binary, output } = options(process.argv.slice(2));
  if (process.platform !== 'win32') throw new Error('This harness currently requires Windows for owned descendant verification');
  assertPlainPath(binary); assertPlainPath(output);
  if (!fs.statSync(binary).isFile()) throw new Error('--codex must name an existing file');
  if (output === path.parse(output).root) throw new Error('Filesystem root is not a dedicated output directory');
  if (fs.existsSync(output) && (!fs.statSync(output).isDirectory() || fs.readdirSync(output).length)) throw new Error('--output must be a new or empty dedicated directory');
  fs.mkdirSync(output, { recursive: true });
  const runId = `${new Date().toISOString().replace(/[:.]/g, '-')}-${crypto.randomBytes(4).toString('hex')}`;
  const runRoot = path.join(output, runId);
  fs.mkdirSync(runRoot); // Exclusive ownership; never overwrite a previous run.
  const metadata = { runId, binaryName: path.basename(binary), sha256: sha256(fs.readFileSync(binary)), scope: 'Synthetic loopback only; empty homes; no production config/auth/chat read or external model request', cases: [] };
  function owned(relative) {
    const full = path.resolve(runRoot, relative);
    if (!full.startsWith(runRoot + path.sep)) throw new Error('Output escaped the owned run directory');
    return full;
  }
  function write(relative, value) { fs.writeFileSync(owned(relative), value); }
  function receipt() { write('results.json', JSON.stringify(metadata, null, 2)); }
  function makeHome(dir) {
    const home = owned(path.join(dir, 'home'));
    for (const rel of ['', '.codex', 'temp', 'AppData/Roaming', 'AppData/Local']) fs.mkdirSync(path.join(home, rel), { recursive: true });
    return home;
  }
  process.on('SIGINT', interrupt); process.on('SIGTERM', interrupt);
  try {
    const versionHome = makeHome('version');
    const version = spawnSync(binary, ['--version'], { env: emptyEnvironment(versionHome), cwd: versionHome, encoding: 'utf8', timeout: 10000, windowsHide: true, maxBuffer: 1024 * 1024 });
    if (version.error || version.status !== 0) throw new Error(`CLI version check failed: ${version.error?.message || version.stderr.trim()}`);
    metadata.version = version.stdout.trim(); receipt();

    async function scenario(kind, requestRetries, streamRetries) {
      if (interrupted) throw new Error('Interrupted; no further scenarios started');
      const name = `${kind}-r${requestRetries}-s${streamRetries}`;
      const home = makeHome(name), work = owned(path.join(name, 'workspace'));
      fs.mkdirSync(work);
      const requests = [], timers = new Set(), sockets = new Set();
      const marker = `synthetic_terminal_${name}`;
      let postCount = 0, child, timer, forceTimer, exited = false, closed = false, watcher;
      let stdioHeldAfterExit = false, descendantCleanup = null;
      let stdout = '', stderr = '', timedOut = false, exitCode = null, signal = null, spawnError = null;
      const started = Date.now();
      const server = http.createServer((req, res) => {
        const chunks = []; let bytes = 0;
        req.on('error', () => {});
        req.on('data', chunk => { bytes += chunk.length; if (bytes > 4 * 1024 * 1024) req.destroy(); else chunks.push(chunk); });
        req.on('end', () => {
          const body = Buffer.concat(chunks); let parsed = {}; try { parsed = JSON.parse(body); } catch {}
          requests.push({ method: req.method, path: req.url, bytes: body.length, bodySha256: sha256(body), inputItems: Array.isArray(parsed.input) ? parsed.input.length : null, previousResponseId: parsed.previous_response_id ?? null });
          if (req.method === 'GET') { res.writeHead(200, { 'content-type': 'application/json' }); return res.end(JSON.stringify({ object: 'list', data: [{ id: 'gpt-5.4', object: 'model', owned_by: 'synthetic' }] })); }
          postCount++;
          if (kind === 'http-terminal') { res.writeHead(504, { 'content-type': 'application/json', 'x-should-retry': 'false' }); return res.end(JSON.stringify({ error: { type: 'server_error', code: 'network_recovery_exhausted', message: marker, retryable: false } })); }
          res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' });
          if (kind === 'sse-terminal') return res.end(sse('response.failed', { sequence_number: 0, status: 504, response: { id: 'resp_terminal', object: 'response', status: 'failed', output: [], error: { type: 'server_error', code: 'network_recovery_exhausted', message: marker, retryable: false } } }));
          if (postCount > 1) return success(res);
          res.write(sse('response.created', { sequence_number: 0, response: { id: 'resp_partial', object: 'response', status: 'in_progress', output: [] } }));
          res.write(sse('response.output_item.added', { sequence_number: 1, output_index: 0, item: { id: 'msg_partial', type: 'message', role: 'assistant', status: 'in_progress', content: [] } }));
          res.write(sse('response.content_part.added', { sequence_number: 2, output_index: 0, item_id: 'msg_partial', content_index: 0, part: { type: 'output_text', text: '', annotations: [] } }));
          res.write(sse('response.output_text.delta', { sequence_number: 3, output_index: 0, item_id: 'msg_partial', content_index: 0, delta: 'Synthetic partial output.' }));
          const drop = setTimeout(() => { timers.delete(drop); res.destroy(); }, 120); timers.add(drop);
        });
      });
      server.on('connection', socket => { sockets.add(socket); socket.on('close', () => sockets.delete(socket)); });
      const owner = { server, sockets }; servers.add(owner);
      let port;
      try {
        await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
        port = server.address().port;
        fs.writeFileSync(path.join(home, '.codex', 'config.toml'), `model = "gpt-5.4"\nmodel_provider = "synthetic_loopback"\napproval_policy = "never"\nsandbox_mode = "read-only"\nproject_doc_max_bytes = 0\nweb_search = "disabled"\n[model_providers.synthetic_loopback]\nname = "Synthetic isolated loopback"\nbase_url = "http://127.0.0.1:${port}/v1"\nwire_api = "responses"\nrequires_openai_auth = false\nsupports_websockets = false\nrequest_max_retries = ${requestRetries}\nstream_max_retries = ${streamRetries}\nstream_idle_timeout_ms = 3000\n`);
        const args = ['exec', '--strict-config', '--skip-git-repo-check', '--ephemeral', '--ignore-rules', '--json', '--color', 'never', '-C', work, 'Synthetic retry verification. Do not use tools.'];
        watcher = await startWatcher(home);
        const launchedAfter = new Date(Date.now() - 1000).toISOString();
        child = spawn(binary, args, { cwd: work, env: emptyEnvironment(home), windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
        children.add(child);
        if (watcher && child.pid) { watcher.registered = true; watcher.helper.stdin.write(Buffer.from(JSON.stringify({ pid: child.pid, parent: process.pid, after: launchedAfter, stopFile: watcher.stopFile })).toString('base64') + '\n'); }
        console.log(JSON.stringify({ stage: 'started', name, pid: child.pid, port }));
        child.stdout.on('data', x => { stdout += x.toString(); }); child.stderr.on('data', x => { stderr += x.toString(); });
        timer = setTimeout(() => {
          timedOut = true; child.kill();
          forceTimer = setTimeout(() => { if (!exited) child.kill('SIGKILL'); }, 2000);
        }, 25000);
        child.once('close', () => { closed = true; });
        await new Promise(resolve => {
          const bound = setTimeout(() => { timedOut = true; child.kill('SIGKILL'); resolve(); }, 28000);
          child.once('error', error => { spawnError = error.message; clearTimeout(bound); resolve(); });
          child.once('exit', (code, sig) => { exitCode = code; signal = sig; exited = true; clearTimeout(bound); resolve(); });
        });
        // A grandchild can inherit stdout/stderr and keep Node's `close` event
        // pending after the CLI has exited. Bound draining independently.
        const drainUntil = Date.now() + 200;
        while (!closed && Date.now() < drainUntil) await wait(20);
        stdioHeldAfterExit = exited && !closed;
        child.stdout.destroy(); child.stderr.destroy();
      } finally {
        clearTimeout(timer); clearTimeout(forceTimer);
        for (const pending of timers) clearTimeout(pending);
        if (child && !exited && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        if (child && exited) children.delete(child);
        child?.stdout?.destroy(); child?.stderr?.destroy();
        descendantCleanup = await finishWatcher(watcher);
        await new Promise(resolve => { server.close(resolve); for (const socket of sockets) socket.destroy(); });
        servers.delete(owner);
      }
      write(path.join(name, 'stdout.jsonl'), stdout); write(path.join(name, 'stderr.txt'), stderr);
      const events = stdout.split(/\r?\n/).filter(Boolean).map(line => { try { return JSON.parse(line); } catch { return { type: 'unparsed', line }; } });
      if (stdioHeldAfterExit && !descendantCleanup.observed?.some(process => process.depth > 0)) {
        descendantCleanup.complete = false;
        descendantCleanup.unexplainedHeldStdio = true;
      }
      const result = { name, kind, requestRetries, streamRetries, port, pid: child?.pid, elapsedMs: Date.now() - started, exitCode, signal, timedOut, spawnError, postCount, requests,
        terminalEvents: events.filter(event => /error|fail|complete/.test(event.type || '')),
        markerVisible: (stdout + stderr).includes(marker), partialVisible: (stdout + stderr).includes('Synthetic partial output.'), successVisible: stdout.includes('Synthetic complete.'),
        serverClosed: !server.listening, childExited: exited, stdioHeldAfterExit, descendantCleanup,
        cleanupIntervention: stdioHeldAfterExit || !!descendantCleanup?.killed?.length };
      metadata.cases.push(result); receipt();
      console.log(JSON.stringify({ name, postCount, exitCode, partialVisible: result.partialVisible, successVisible: result.successVisible, childExited: exited, cleanupIntervention: result.cleanupIntervention, descendantsClean: descendantCleanup.complete }));
      const expectSuccess = kind === 'partial-drop' && streamRetries === 1;
      result.expectedTerminalPresent = events.some(event => event.type === (expectSuccess ? 'turn.completed' : 'turn.failed'));
      result.expectedOutputPresent = expectSuccess ? result.successVisible : kind === 'partial-drop' || result.markerVisible;
      receipt();
      if (timedOut || spawnError || !exited || interrupted || postCount === 0 || !descendantCleanup.complete || !result.expectedTerminalPresent || !result.expectedOutputPresent) throw new Error(`Scenario did not finish cleanly, lacked terminal evidence, or never reached the loopback fixture: ${name}`);
    }
    for (const kind of ['http-terminal', 'sse-terminal', 'partial-drop']) for (const [r, s] of [[0, 0], [0, 1], [1, 0]]) await scenario(kind, r, s);
    metadata.completedAt = new Date().toISOString();
  } catch (error) {
    metadata.failure = error.message;
    throw error;
  } finally {
    interrupt();
    await finishOwnedProcesses();
    for (const watcher of [...watchers]) await finishWatcher(watcher);
    process.removeListener('SIGINT', interrupt); process.removeListener('SIGTERM', interrupt);
    metadata.cleanup = { allChildrenExited: children.size === 0 && metadata.cases.every(x => x.childExited), allListenersClosed: servers.size === 0 && metadata.cases.every(x => x.serverClosed), allDescendantsChecked: watchers.size === 0 && metadata.cases.every(x => x.descendantCleanup?.complete), interventionCases: metadata.cases.filter(x => x.cleanupIntervention).map(x => x.name) };
    receipt();
    console.log(JSON.stringify({ resultFile: owned('results.json'), cleanup: metadata.cleanup }));
    if (!metadata.cleanup.allChildrenExited || !metadata.cleanup.allListenersClosed || !metadata.cleanup.allDescendantsChecked) process.exitCode = 1;
  }
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
