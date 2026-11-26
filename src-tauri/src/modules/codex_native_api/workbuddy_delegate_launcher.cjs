'use strict';
// Embedded by the Rust owner. No account files are read and no secret is written.
// The first stdin line is a private bootstrap; every byte after it remains ACP.
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { StringDecoder } = require('node:string_decoder');

function bootstrapLine(stream, limit = 65536) {
  stream.pause();
  return new Promise((resolve, reject) => {
    let pending = Buffer.alloc(0);
    function clean() {
      stream.removeListener('readable', readable);
      stream.removeListener('end', ended);
      stream.removeListener('error', failed);
    }
    function failed() { clean(); reject(new Error('bootstrap unavailable')); }
    function ended() { failed(); }
    function readable() {
      let chunk;
      while ((chunk = stream.read()) !== null) {
        pending = Buffer.concat([pending, chunk]);
        const newline = pending.indexOf(10);
        if (newline < 0) {
          if (pending.length > limit) return failed();
          continue;
        }
        if (newline > limit) return failed();
        clean();
        stream.pause();
        const remainder = pending.subarray(newline + 1);
        const line = pending.subarray(0, newline).toString('utf8');
        // Do not use readline: it may eagerly consume the first ACP frame.
        if (remainder.length) stream.unshift(remainder);
        pending = Buffer.alloc(0);
        try { resolve(JSON.parse(line)); } catch { reject(new Error('invalid bootstrap')); }
        return;
      }
    }
    stream.on('readable', readable);
    stream.once('end', ended);
    stream.once('error', failed);
    readable();
  });
}

function acpOnlyStdout() {
  const write = process.stdout.write.bind(process.stdout);
  const decoder = new StringDecoder('utf8');
  let pending = '';
  process.stdout.write = function (chunk, encoding, callback) {
    const done = typeof encoding === 'function' ? encoding : callback;
    pending += typeof chunk === 'string' ? chunk : decoder.write(chunk);
    if (pending.length > 2 * 1024 * 1024) process.exit(75);
    let newline;
    while ((newline = pending.indexOf('\n')) >= 0) {
      const line = pending.slice(0, newline);
      pending = pending.slice(newline + 1);
      if (!line.trimStart().startsWith('{')) continue;
      try {
        if (JSON.parse(line).jsonrpc === '2.0') write(line + '\n');
      } catch { /* CLI diagnostics are deliberately discarded. */ }
    }
    if (done) process.nextTick(done);
    return true;
  };
  // CLI logs can include auth diagnostics. Never persist or forward them.
  process.stderr.write = function (_chunk, encoding, callback) {
    const done = typeof encoding === 'function' ? encoding : callback;
    if (done) process.nextTick(done);
    return true;
  };
}

async function main() {
  const secret = await bootstrapLine(process.stdin);
  if (!secret || typeof secret.accessToken !== 'string' || !secret.accessToken.trim()
      || typeof secret.cliPath !== 'string' || !path.isAbsolute(secret.cliPath)
      || !fs.statSync(secret.cliPath).isFile()
      || typeof secret.model !== 'string' || typeof secret.effort !== 'string') {
    throw new Error('invalid bootstrap');
  }
  const root = fs.realpathSync(__dirname);
  const home = path.join(root, 'home');
  const profile = path.join(root, 'profile');
  for (const dir of [home, profile, path.join(profile, 'app')]) {
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  }
  // Rust already cleared the OS environment. Rebuild the application environment
  // as well, so neither Electron nor future launcher changes inherit credentials.
  const platform = {};
  for (const key of ['PATH', 'SystemRoot', 'SYSTEMROOT', 'WINDIR', 'COMSPEC', 'PATHEXT']) {
    if (process.env[key]) platform[key] = process.env[key];
  }
  for (const key of Object.keys(process.env)) delete process.env[key];
  Object.assign(process.env, platform, {
    HOME: home, USERPROFILE: home, APPDATA: path.join(home, 'AppData', 'Roaming'),
    LOCALAPPDATA: path.join(home, 'AppData', 'Local'),
    TMPDIR: path.join(root, 'tmp'), TMP: path.join(root, 'tmp'), TEMP: path.join(root, 'tmp'),
    CODEBUDDY_CONFIG_DIR: profile, WORKBUDDY_CONFIG_DIR: profile,
    WORKBUDDY_USER_DATA_DIR: path.join(profile, 'app'),
    CODEBUDDY_PROJECT_CONFIG_DIR: '.workbuddy', WORKBUDDY_DATA_FOLDER_NAME: '.workbuddy',
    CODEBUDDY_HOST: 'workbuddy-desktop', CODEBUDDY_FORCE_LITE_WB_BUNDLE: '1',
    CODEBUDDY_AUTH_TOKEN: secret.accessToken,
    SERVER__HOST: '127.0.0.1',
    CODEBUDDY_DISABLE_PRODUCT_CACHE: '1', CODEBUDDY_DISABLE_SESSION_HISTORY_CLEANUP: '1',
    CODEBUDDY_PLUGIN_BACKGROUND_MANAGEMENT: '0', CODEBUDDY_DISABLE_PLUGIN_INSTALLS: '1',
    CODEBUDDY_DISABLE_EXTENDED_PLUGIN_HOOKS: '1',
    CODEBUDDY_CODE_DISABLE_BACKGROUND_TASKS: '1', CODEBUDDY_DISABLE_BUILTIN_SUBAGENTS: '1',
    CODEBUDDY_DISABLE_AUTO_MEMORY: '1', CODEBUDDY_DISABLE_PARENT_MEMORY: '1',
    CODEBUDDY_DISABLE_CRON: '1', CODEBUDDY_DISABLE_WORKFLOWS: '1',
    CODEBUDDY_DISABLE_FILE_LOGS: '1', CODEBUDDY_DISABLE_TRACE_COLLECTOR: '1',
    CODEBUDDY_CODE_DISABLE_FILE_CHECKPOINTING: '1',
    CODEBUDDY_CODE_DISABLE_TERMINAL_TITLE: '1', CODEBUDDY_BASH_AUTO_BACKGROUND_DISABLED: '1',
    CODEBUDDY_MAX_RETRIES: '0', CODEBUDDY_MODEL_401_MAX_RETRIES: '0', CODEBUDDY_RETRY_WATCHDOG: '0',
    DISABLE_AUTOUPDATER: '1', DISABLE_TELEMETRY: '1', DISABLE_GALILEO: '1',
    DO_NOT_TRACK: '1', OTEL_SDK_DISABLED: 'true', LANG: 'en_US.UTF-8',
  });
  if (secret.domestic === true) {
    process.env.CODEBUDDY_INTERNET_ENVIRONMENT = 'internal';
    process.env.CODEBUDDY_COPILOT_INTERNET_ENVIRONMENT = 'internal';
  }
  fs.mkdirSync(path.join(root, 'tmp'), { recursive: true, mode: 0o700 });
  // This settings file contains no credentials. The CLI cannot load caller
  // hooks/plugins/MCP/agents from either the real HOME or a real project.
  fs.writeFileSync(path.join(profile, 'settings.json'), JSON.stringify({
    autoUpdates: false, daemon: { preventSleep: false }, autoCompactEnabled: false,
    promptSuggestionEnabled: false, fileCheckpointingEnabled: false,
    enabledPlugins: {}, extraKnownMarketplaces: {}, hooks: {},
    enableAllProjectMcpServers: false, enabledMcpjsonServers: [],
    disableWorkflows: true,
    memory: { enabled: false, autoMemoryEnabled: false, parentDiscovery: false },
    subagents: {},
    permissions: { defaultMode: 'dontAsk', allow: ['DelegateTool'] },
  }), { mode: 0o600 });
  const cliPath = fs.realpathSync(secret.cliPath);
  const model = secret.model;
  const effort = secret.effort;
  secret.accessToken = '';
  acpOnlyStdout();
  process.argv = [process.execPath, cliPath,
    '--serve', '--host', '127.0.0.1', '--port', '0', '--auth', 'password',
    '--acp', '--acp-transport', 'stdio', '--tools', 'DelegateTool',
    '--allowedTools', 'DelegateTool', '--permission-mode', 'dontAsk',
    '--strict-mcp-config', '--setting-sources', '', '--no-session-persistence',
    '--settings', path.join(profile, 'settings.json'),
    '--system-prompt', 'Use only the explicitly registered external DelegateTool tools. '
      + 'The client executes all file and terminal operations. Never use built-in tools, '
      + 'plugins, subagents, MCP servers, or other execution paths. Return tool results accurately.'];
  if (model) process.argv.push('--model', model);
  if (effort) process.argv.push('--effort', effort);
  if (cliPath.endsWith('.mjs')) await import(pathToFileURL(cliPath).href);
  else require(cliPath);
}

main().catch(() => { process.exitCode = 70; });
