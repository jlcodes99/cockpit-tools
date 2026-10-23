// Optional installed-asset integration test. It imports the real module and its
// dependencies in a hidden, isolated Electron renderer without starting the app.
// No native bridge, account files, application initialization or remote traffic.
'use strict';

const assert = require('node:assert/strict');
const path = require('node:path');
const os = require('node:os');
const fs = require('node:fs');
const repository = path.resolve(__dirname, '..');
const archivePath = process.env.COCKPIT_NATIVE_ASAR || '/Applications/ChatGPT.app/Contents/Resources/app.asar';
const resultPrefix = 'COCKPIT_NATIVE_ASSET_HANDOFF ';

if (!process.versions.electron) {
  const { test } = require('node:test');
  const { spawn } = require('node:child_process');
  test('small handoff loads the complete installed native module under real meta CSP', {
    timeout: 60000,
    skip: !fs.existsSync(archivePath) && 'installed native ASAR unavailable; set COCKPIT_NATIVE_ASAR',
  }, async () => {
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-native-asset-handoff-'));
    const env = { ...process.env, COCKPIT_NATIVE_ASAR: archivePath };
    delete env.ELECTRON_RUN_AS_NODE;
    let child;
    try {
      const stdout = await new Promise((resolve, reject) => {
        child = spawn(require('electron'), [__filename, profile], {
          env, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true,
        });
        let output = '', errors = '';
        const timeout = setTimeout(() => {
          child.kill('SIGKILL');
          reject(new Error(`Native asset integration timed out.\n${errors}`));
        }, 55000);
        child.stdout.on('data', data => { output += data; });
        child.stderr.on('data', data => { errors += data; });
        child.once('error', error => { clearTimeout(timeout); reject(error); });
        child.once('exit', code => {
          clearTimeout(timeout);
          if (code !== 0) reject(new Error(`Electron exited ${code}.\n${output}\n${errors}`));
          else resolve(output);
        });
      });
      const line = stdout.split('\n').find(value => value.startsWith(resultPrefix));
      assert.ok(line, 'renderer must return observed native module results');
      const result = JSON.parse(line.slice(resultPrefix.length));
      assert.ok(result.sourceLength > 1_000_000, 'exercise the full installed module');
      assert.ok(result.moduleExports > 1000, 'module and its real dependencies must actually instantiate');
      assert.ok(result.primaryExports > 0, 'the real primary module must also instantiate before activation');
      assert.equal(result.state.nativePatchState, 'active');
      assert.equal(result.state.nativePatchFeatures, '1111');
      assert.equal(result.install.ok, true);
      assert.equal(result.name, 'asset-fixture-original-name');
      assert.equal(result.repeated.name, 'asset-fixture-original-name');
      assert.equal(result.repeated.state.nativePatchFeatures, '1111');
      assert.ok(result.injectionBytes < 100000, 'new-document script must not contain the 14MB module');
      assert.equal(result.preparedSourceReturned, false);
      assert.equal(result.canceled.state.nativePatchState, 'unsupported');
      assert.equal(result.canceled.state.nativePatchReason, 'handoff-unavailable');
      assert.equal(result.canceled.name, 'asset-fixture-original-name');
      assert.equal(result.csp, result.expectedCsp);
      assert.equal(result.temporaryElements, 0);
      assert.equal(result.inlineBlocked, true);
      assert.deepEqual(result.errors, []);
      assert.deepEqual(result.moduleConsole, []);
      assert.deepEqual(result.blockedExternalRequests, []);
      console.log(`Installed asset ${result.asset}: ${result.sourceLength} chars, ${result.moduleExports} exports; `
        + `Electron ${result.versions.electron}, Chromium ${result.versions.chrome}. `
        + 'This validates this test engine, not the installed application runtime.');
    } finally {
      if (child?.pid && child.exitCode === null && child.signalCode === null) {
        await new Promise(resolve => { child.once('exit', resolve); child.kill('SIGKILL'); });
      }
      fs.rmSync(profile, { recursive: true, force: true });
    }
  });
} else {
  runFixture().catch(error => {
    console.error(error.stack);
    require('electron').app.exit(1);
  });
}

function openArchive() {
  // Electron patches node:fs to treat ASAR as a virtual directory. Read raw
  // bytes using original-fs; never extract into or modify the installed app.
  const raw = require('original-fs');
  const fd = raw.openSync(archivePath, 'r');
  const readExactly = (position, length) => {
    const data = Buffer.alloc(length);
    let read = 0;
    while (read < length) {
      const count = raw.readSync(fd, data, read, length - read, position + read);
      if (!count) throw Error('Unexpected end of ASAR');
      read += count;
    }
    return data;
  };
  const prefix = readExactly(0, 16);
  const header = JSON.parse(readExactly(16, prefix.readUInt32LE(12)));
  const dataOffset = 8 + prefix.readUInt32LE(4);
  return {
    names: Object.keys(header.files.webview.files.assets.files),
    read(file) {
      let entry = header;
      for (const part of file.split('/')) {
        if (!Object.hasOwn(entry.files || {}, part)) throw Error(`Missing ASAR entry: ${file}`);
        entry = entry.files[part];
      }
      if (entry.unpacked) return raw.readFileSync(`${archivePath}.unpacked/${file}`);
      return readExactly(dataOffset + Number(entry.offset), entry.size);
    },
    close() { raw.closeSync(fd); },
  };
}

async function runFixture() {
  const { app, BrowserWindow, protocol } = require('electron');
  const profile = process.argv[2];
  assert.ok(profile && path.basename(profile).startsWith('cockpit-native-asset-handoff-'));
  app.setPath('userData', profile);
  app.setActivationPolicy?.('prohibited');
  app.commandLine.appendSwitch('disable-background-networking');
  app.commandLine.appendSwitch('disable-component-update');
  protocol.registerSchemesAsPrivileged([{
    scheme: 'app', privileges: { standard: true, secure: true, supportFetchAPI: true, corsEnabled: true },
  }]);
  const archive = openArchive();
  const asset = archive.names.find(name => /^app-initial-[\w-]+\.js$/.test(name));
  assert.ok(asset);
  const moduleUrl = `app://-/assets/${asset}`;
  const initialSource = archive.read(`webview/assets/${asset}`).toString();
  const primaryNames = new Set([...initialSource.matchAll(/["'`]\.\/app-primary-[A-Za-z0-9_-]+\.js["'`]/g)]
    .map(match => match[0].slice(3, -1)));
  assert.equal(primaryNames.size, 1, 'the native initial module must reference one primary contract');
  const primaryAsset = [...primaryNames][0];
  assert.ok(archive.names.includes(primaryAsset));
  const primaryUrl = `app://-/assets/${primaryAsset}`;
  const originalHtml = archive.read('webview/index.html').toString();
  const cspMatch = originalHtml.match(/http-equiv="Content-Security-Policy" content="([^"]+)"/);
  assert.ok(cspMatch);
  const expectedCsp = cspMatch[1].replaceAll('&#39;', "'").replaceAll('&quot;', '"').replaceAll('&amp;', '&');
  // Keep the exact installed CSP, style and modulepreload dependencies, but
  // replace the application initializer with an import-only probe entry.
  const html = originalHtml.replace(/<script\b[^>]*type="module"/g, '<script type="application/cockpit-inert"')
    .replace('</body>', '<textarea aria-label="Fixture composer"></textarea></body>');
  const runtime = ['startup_guard.js', 'fast.js', 'quota.js']
    .map(name => fs.readFileSync(path.join(repository, 'src-tauri/src/modules/codex_app_enhancements', name), 'utf8')).join('\n;');
  const bootstrap = 'window.__cockpitEnhancements={nativeFast:true,quotaCompat:true,apiService:true,apiKeyAuthVerified:true,owner:"asset-test"};' + runtime;
  let serveEntry = false;
  const errors = [], blockedExternalRequests = [], messages = [];
  await app.whenReady();
  app.dock?.hide();
  protocol.handle('app', request => {
    const pathname = new URL(request.url).pathname;
    try {
      const data = pathname === '/' ? Buffer.from(serveEntry
        ? html.replace('</head>', '<script type="module" src="/assets/fixture-entry.js"></script></head>') : html)
        : pathname === '/assets/fixture-entry.js'
          ? Buffer.from(`import * as mod from ${JSON.stringify(moduleUrl)};import * as primary from ${JSON.stringify(primaryUrl)};
window.fixtureModuleExports=Object.keys(mod).length;window.fixturePrimaryExports=Object.keys(primary).length;`)
          : archive.read(`webview${pathname}`);
      const type = pathname === '/' ? 'text/html' : pathname.endsWith('.js') ? 'text/javascript'
        : pathname.endsWith('.css') ? 'text/css' : 'application/octet-stream';
      return new Response(data, { headers: {
        'Content-Type': type, 'Cache-Control': 'no-store',
      } });
    } catch (error) {
      errors.push(error.message);
      return new Response('', { status: 404 });
    }
  });
  const window = new BrowserWindow({
    show: false, webPreferences: { nodeIntegration: false, contextIsolation: true, sandbox: true },
  });
  window.webContents.on('console-message', event => messages.push(event.message));
  window.webContents.session.webRequest.onBeforeRequest((details, callback) => {
    const cancel = !['app:', 'data:', 'blob:'].includes(new URL(details.url).protocol);
    if (cancel) blockedExternalRequests.push(details.url);
    callback({ cancel });
  });
  window.webContents.debugger.attach('1.3');
  const send = (method, params = {}) => window.webContents.debugger.sendCommand(method, params);
  const evaluate = async expression => {
    const response = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (response.exceptionDetails) throw Error(JSON.stringify(response.exceptionDetails));
    return response.result.value;
  };
  await window.loadURL('app://-/');
  await send('Page.enable');
  await evaluate(`window.name='asset-fixture-original-name';${bootstrap}`);
  const prepared = await evaluate("window.__cockpitNativeFast.prepareHandoff('asset-nonce')");
  assert.equal(prepared.ok, true, JSON.stringify(prepared));
  assert.equal(prepared.mode, 'full', JSON.stringify(prepared));
  assert.equal(await evaluate('window.name'), 'asset-fixture-original-name');
  const injection = `${bootstrap};window.fixtureInstall=window.__cockpitNativeFast.consumeHandoff('asset-nonce');`;
  const registered = await send('Page.addScriptToEvaluateOnNewDocument', { source: injection });
  serveEntry = true;
  await new Promise((resolve, reject) => {
    window.webContents.once('did-finish-load', resolve);
    evaluate("window.__cockpitNativeFast.reloadPreparedDocument('asset-nonce')").then(result => {
      if (!result?.ok) reject(Error(JSON.stringify(result)));
    }, reject);
  });
  const result = await evaluate(`(async()=>{
    for(let i=0;i<1000&&(!window.fixtureModuleExports||!window.fixturePrimaryExports);i++)await new Promise(resolve=>setTimeout(resolve,10));
    return {moduleExports:window.fixtureModuleExports,primaryExports:window.fixturePrimaryExports,state:window.__cockpitEnhancements,install:window.fixtureInstall,name:window.name,
      csp:document.querySelector('meta[http-equiv="Content-Security-Policy"]').content,
      temporaryElements:document.querySelectorAll('script[type=importmap],link[href^="blob:"]').length};
  })()`);
  const moduleConsole = [...messages];
  const inlineBlocked = await evaluate(`(()=>{const script=document.createElement('script');
    script.textContent='window.unexpectedInline=true';document.head.appendChild(script);script.remove();
    return window.unexpectedInline!==true;})()`);
  const manualReload = () => new Promise((resolve, reject) => {
    window.webContents.once('did-finish-load', resolve);
    evaluate('location.reload()').catch(reject);
  });
  const capture = () => evaluate(`(async()=>{
    for(let i=0;i<1000&&!window.fixtureModuleExports;i++)await new Promise(resolve=>setTimeout(resolve,10));
    return {moduleExports:window.fixtureModuleExports,state:window.__cockpitEnhancements,name:window.name};
  })()`);
  await manualReload();
  const repeated = await capture();
  await evaluate("window.__cockpitNativeFast.cancelHandoff('asset-nonce')");
  await manualReload();
  const canceled = await capture();
  await send('Page.removeScriptToEvaluateOnNewDocument', { identifier: registered.identifier });
  console.log(resultPrefix + JSON.stringify({
    ...result, asset, sourceLength: prepared.sourceLength, expectedCsp, inlineBlocked,
    preparedSourceReturned: Object.hasOwn(prepared, 'patchedSource'), injectionBytes: injection.length, repeated, canceled,
    errors, moduleConsole, blockedExternalRequests,
    versions: { electron: process.versions.electron, chrome: process.versions.chrome },
  }));
  window.destroy();
  archive.close();
  app.exit(0);
}
