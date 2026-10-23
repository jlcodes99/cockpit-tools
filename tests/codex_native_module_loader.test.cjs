// Code-level Chromium smoke test: an isolated hidden fixture, never the user's app.
// Run: node --test tests/codex_native_module_loader.test.cjs
'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const vm = require('node:vm');

const repository = path.resolve(__dirname, '..');
const enhancements = path.join(repository, 'src-tauri/src/modules/codex_app_enhancements');
const moduleUrl = 'app://-/assets/app-initial-fixture.js';
const primaryUrl = 'app://-/assets/app-primary-fixture.js';
const resultPrefix = 'COCKPIT_MODULE_LOADER_RESULT ';

if (!process.versions.electron) {
  const { test } = require('node:test');
  const { spawn } = require('node:child_process');

  test('small native handoff loads under meta CSP and fails closed under header CSP', { timeout: 30000 }, async () => {
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-native-module-loader-'));
    const env = { ...process.env };
    delete env.ELECTRON_RUN_AS_NODE;
    let child;
    try {
      const output = await new Promise((resolve, reject) => {
        child = spawn(require('electron'), [__filename, profile], {
          env, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true,
        });
        let stdout = '', stderr = '';
        const timeout = setTimeout(() => {
          child.kill('SIGKILL');
          reject(new Error(`Electron smoke test timed out.\n${stderr}`));
        }, 25000);
        child.stdout.on('data', data => { stdout += data; });
        child.stderr.on('data', data => { stderr += data; });
        child.once('error', error => { clearTimeout(timeout); reject(error); });
        child.once('exit', code => {
          clearTimeout(timeout);
          if (code !== 0) reject(new Error(`Electron exited ${code}.\n${stdout}\n${stderr}`));
          else resolve(stdout);
        });
      });
      const line = output.split('\n').find(value => value.startsWith(resultPrefix));
      assert.ok(line, 'Electron must return its observed module result');
      const result = JSON.parse(line.slice(resultPrefix.length));
      assert.equal(result.original.access, null);
      assert.equal(result.original.dictation, true);
      assert.equal(result.waiting.reason, 'ui-busy');
      assert.equal(result.transientProgress.reason, 'conversation-busy');
      assert.equal(result.resumed.ok, true);
      assert.equal(result.adapted.install.ok, true);
      assert.equal(result.adapted.state.nativePatchState, 'active');
      assert.equal(result.adapted.state.nativePatchFeatures, '1011');
      assert.equal(result.adapted.fixture.access.granted, true);
      assert.equal(result.adapted.fixture.dictation, true);
      assert.equal(result.adapted.fixture.dependency, 'static-relative-import-ok');
      assert.equal(result.adapted.fixture.dynamic, 'dynamic-relative-import-ok');
      assert.equal(result.adapted.fixture.importUrl, moduleUrl);
      assert.equal(result.adapted.primary.importUrl, primaryUrl);
      assert.equal(result.prepared.mode, 'full');
      assert.equal(result.adapted.temporaryElements, 0);
      assert.equal(result.adapted.name, 'fixture-original-name');
      assert.ok(result.injectionBytes < 100000, 'new-document script contains no module payload');
      assert.deepEqual(result.repeated.fixture.access, { granted: true }, JSON.stringify(result.repeated));
      assert.equal(result.repeated.state.nativePatchFeatures, '1011');
      assert.equal(result.repeated.name, 'fixture-original-name');
      assert.equal(result.canceled.install.reason, 'handoff-unavailable');
      assert.equal(result.canceled.fixture.access, null);
      assert.equal(result.canceled.name, 'fixture-original-name');
      assert.equal(result.hostReload.fixture.access, null);
      assert.equal(result.hostReload.state.nativePatchState, 'unsupported');
      assert.equal(result.hostReload.state.nativePatchReason, 'handoff-unavailable');
      assert.equal(result.header.fixture.access, null);
      assert.equal(result.header.state.nativePatchState, 'unsupported');
      assert.equal(result.header.state.nativePatchReason, 'module-preload-failed');
      assert.equal(result.header.name, 'fixture-original-name');
      assert.equal(result.unsupported.prepared.ok, false);
      assert.equal(result.unsupported.fixture.access, null, 'unsupported quota must not activate partial Fast patches');
      assert.equal(result.unsupported.fixture.dictation, true, 'native dictation stays available even if quota adaptation is unsupported');
      assert.equal(result.unsupported.temporaryElements, 0);
      assert.equal(result.unsupported.name, 'fixture-original-name');
      assert.deepEqual(result.unsupported.moduleErrors, []);
      assert.equal(result.inlineScriptBlocked, true, 'the original CSP must still block an untrusted inline script');
      assert.equal(result.restored.access, null, 'removing the new-document script restores the unmodified module');
      assert.deepEqual(result.moduleErrors, []);
      console.log(`Electron ${result.versions.electron}; Chromium ${result.versions.chrome}; V8 ${result.versions.v8}`);
    } finally {
      if (child?.pid && child.exitCode === null && child.signalCode === null) {
        await new Promise(resolve => {
          child.once('exit', resolve);
          child.kill('SIGKILL');
        });
      }
      fs.rmSync(profile, { recursive: true, force: true });
    }
  });
} else {
  runElectronFixture().catch(error => {
    console.error(error.stack);
    require('electron').app.exit(1);
  });
}

async function runElectronFixture() {
  const { app, BrowserWindow, protocol } = require('electron');
  const profile = process.argv[2];
  assert.ok(profile && path.basename(profile).startsWith('cockpit-native-module-loader-'));
  app.setPath('userData', profile);
  app.setActivationPolicy?.('prohibited');
  app.commandLine.appendSwitch('disable-background-networking');
  app.commandLine.appendSwitch('disable-component-update');
  // No bypassCSP flag: this models the packaged app's ordinary secure scheme.
  protocol.registerSchemesAsPrivileged([{
    scheme: 'app', privileges: { standard: true, secure: true, supportFetchAPI: true, corsEnabled: true },
  }]);

  const fast = fs.readFileSync(path.join(enhancements, 'fast.js'), 'utf8');
  const guard = fs.readFileSync(path.join(enhancements, 'startup_guard.js'), 'utf8');
  const quota = fs.readFileSync(path.join(enhancements, 'quota.js'), 'utf8');
  const runtime = `${guard};${fast};${quota}`;
  const unitTest = fs.readFileSync(path.join(enhancements, 'fast.test.cjs'), 'utf8');
  const fixtureLiteral = unitTest.match(/const fixture = ([\s\S]*?);\n\nfunction load/);
  assert.ok(fixtureLiteral, 'reuse the same four structural gate fixture as the unit tests');
  const gates = vm.runInNewContext(fixtureLiteral[1]);
  const dictationUnitTest = fs.readFileSync(path.join(enhancements, 'native_dictation.test.cjs'), 'utf8');
  const dictationLiteral = dictationUnitTest.match(/const fixture = ([\s\S]*?);\n\nfunction load/);
  assert.ok(dictationLiteral);
  const dictationFixture = vm.runInNewContext(dictationLiteral[1]);
  const quotaFixture = require('./helpers/codexNativeQuotaFixture.cjs')();
  // The real client's script policy does not allow blob:. It is intentionally
  // present during every navigation, including the adapted document.
  const csp = "default-src 'self'; script-src 'self' 'sha256-Z2/iFzh9VMlVkEOar1f/oSHWwQk3ve1qk/C2WdsC4Xk=' 'wasm-unsafe-eval'; style-src 'self';";
  const html = `<!doctype html><html><head><meta http-equiv="Content-Security-Policy" content="${csp}"><script type="module" src="./assets/index-fixture.js"></script></head><body><textarea></textarea></body></html>`;
  const nativeModule = `import {dependency} from './dependency-fixture.js';
${gates}
function fixtureDictation(){
const Y=()=>({}),U={},wc=()=>({}),u6u={c:()=>[]},Eu=()=>({isLoading:false,isCapable:true});
${dictationFixture}
return l6u()}
${quotaFixture.initial}
const primaryPath='./app-primary-fixture.js';
const defaultHost='local';
function readIdentity(){return {authMethod:null}}
function readRequirements(){return {data:{granted:true},isPending:false}}
function selectAccess(value){return value}
const dynamic=await import('./dynamic-fixture.js');
window.fixtureDictation=fixtureDictation();
window.fixtureResult={access:ui({}).serviceTierAccess,dictation:window.fixtureDictation,dependency,dynamic:dynamic.dynamic,importUrl:import.meta.url};`;
  // This dependency is evaluated just like a native lazily loaded primary
  // module; the separate completion proves both import-map entries execute.
  const primaryModule = `const atom=()=>({}),store={},identity={},status={};\n${quotaFixture.primary}
window.fixturePrimary={importUrl:import.meta.url};`;
  const files = new Map([
    ['/', ['text/html', html]],
    ['/assets/index-fixture.js', ['text/javascript', 'import "./app-initial-fixture.js";import "./app-primary-fixture.js";']],
    ['/assets/app-initial-fixture.js', ['text/javascript', nativeModule]],
    ['/assets/app-primary-fixture.js', ['text/javascript', primaryModule]],
    ['/assets/dependency-fixture.js', ['text/javascript', 'export const dependency="static-relative-import-ok";']],
    ['/assets/dynamic-fixture.js', ['text/javascript', 'export const dynamic="dynamic-relative-import-ok";']],
  ]);

  let headerCsp = false, holdModuleRead = false, releaseModuleRead, signalModuleRead;
  await app.whenReady();
  app.dock?.hide();
  protocol.handle('app', async request => {
    const pathname = new URL(request.url).pathname;
    if (holdModuleRead && pathname === '/assets/app-initial-fixture.js') {
      holdModuleRead = false;
      await new Promise(resolve => { releaseModuleRead = resolve; signalModuleRead(); });
    }
    const resource = files.get(pathname);
    return resource ? new Response(resource[1], { headers: { 'Content-Type': resource[0],
      ...(pathname === '/' && headerCsp ? { 'Content-Security-Policy': csp } : {}) } })
      : new Response('not found', { status: 404 });
  });
  const window = new BrowserWindow({
    show: false, webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true },
  });
  const messages = [];
  window.webContents.on('console-message', event => messages.push(event.message));
  window.webContents.session.webRequest.onBeforeRequest((details, callback) => {
    callback({ cancel: !['app:', 'blob:', 'data:'].includes(new URL(details.url).protocol) });
  });
  window.webContents.debugger.attach('1.3');
  const send = (method, params = {}) => window.webContents.debugger.sendCommand(method, params);
  const evaluate = async expression => {
    const response = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (response.exceptionDetails) throw Error(JSON.stringify(response.exceptionDetails));
    return response.result.value;
  };
  const waitForModule = () => evaluate(`(async()=>{
    for(let i=0;i<200&&(!window.fixtureResult||!window.fixturePrimary);i++)await new Promise(resolve=>setTimeout(resolve,10));
    if(!window.fixtureResult||!window.fixturePrimary)throw Error('both fixture modules must execute');
    return window.fixtureResult;
  })()`);
  await window.loadURL('app://-/');
  await send('Page.enable');
  const original = await waitForModule();
  const bootstrap = `window.__cockpitEnhancements={nativeFast:true,quotaCompat:false,apiService:true,apiKeyAuthVerified:true,owner:'loader-test'};${runtime}`;
  await evaluate(`window.name='fixture-original-name';${bootstrap}`);
  // Reproduce the real failure: ordinary input while module preparation is
  // still pending. This is an isolated renderer, never a user's application.
  const moduleRead = new Promise(resolve => { signalModuleRead = resolve; });
  holdModuleRead = true;
  const preparation = evaluate("window.__cockpitNativeFast.prepareHandoff('loader-nonce')");
  await moduleRead;
  await send('Input.dispatchMouseEvent', { type: 'mousePressed', x: 15, y: 15, button: 'left', clickCount: 1 });
  await send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 15, y: 15, button: 'left', clickCount: 1 });
  await send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'ArrowRight', code: 'ArrowRight', windowsVirtualKeyCode: 39 });
  await send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'ArrowRight', code: 'ArrowRight', windowsVirtualKeyCode: 39 });
  await evaluate(`(()=>{const menu=document.createElement('div');menu.id='fixture-menu';menu.role='menu';menu.textContent='Speed';document.body.appendChild(menu);return true})()`);
  releaseModuleRead();
  const prepared = await preparation;
  assert.equal(prepared.ok, true, JSON.stringify(prepared));
  assert.equal(prepared.mode, 'full', JSON.stringify(prepared));
  assert.equal(prepared.patchedSource, undefined);
  assert.equal(await evaluate('window.name'), 'fixture-original-name');
  const injection = `${bootstrap};window.installResult=window.__cockpitNativeFast.consumeHandoff('loader-nonce');`;
  const registered = await send('Page.addScriptToEvaluateOnNewDocument', { source: injection });
  const waiting = await evaluate(`(async()=>{await new Promise(resolve=>setTimeout(resolve,1100));
    return window.__cockpitNativeFast.inspectStartupReload()})()`);
  // The first inspection can also observe a newly incremented event counter.
  const menuInspection = waiting.reason === 'interaction-settling'
    ? await evaluate(`(async()=>{await new Promise(resolve=>setTimeout(resolve,1100));
        return window.__cockpitNativeFast.inspectStartupReload()})()`)
    : waiting;
  assert.equal(menuInspection.reason, 'ui-busy', JSON.stringify(menuInspection));
  const transientProgress = await evaluate(`(()=>{document.getElementById('fixture-menu').remove();
    const progress=document.createElement('div');progress.id='fixture-progress';progress.role='progressbar';
    progress.textContent='Loading';document.body.appendChild(progress);
    return window.__cockpitNativeFast.reloadPreparedDocument('loader-nonce')})()`);
  assert.equal(transientProgress.reason, 'conversation-busy', JSON.stringify(transientProgress));
  const resumed = await evaluate(`(()=>{document.getElementById('fixture-progress').remove();
    return window.__cockpitNativeFast.inspectStartupReload()})()`);
  assert.equal(resumed.ok, true, JSON.stringify(resumed));
  // Perform the same real navigation as the startup-only production path.
  await new Promise((resolve, reject) => {
    window.webContents.once('did-finish-load', resolve);
    evaluate("window.__cockpitNativeFast.reloadPreparedDocument('loader-nonce')").then(result => {
      if (!result?.ok) reject(Error(JSON.stringify(result)));
    }, reject);
  });
  await waitForModule();
  const adapted = await evaluate(`({fixture:window.fixtureResult,primary:window.fixturePrimary,install:window.installResult,
    state:window.__cockpitEnhancements,name:window.name,
    temporaryElements:document.querySelectorAll('script[type=importmap],link[rel=modulepreload]').length})`);
  const moduleErrors = [...messages];
  const inlineScriptBlocked = await evaluate(`(()=>{
    const script=document.createElement('script');script.textContent='window.__unexpectedInlineExecution=true';
    document.head.appendChild(script);script.remove();return window.__unexpectedInlineExecution!==true;
  })()`);
  const manualReload = () => new Promise((resolve, reject) => {
    window.webContents.once('did-finish-load', resolve);
    evaluate('location.reload()').catch(reject);
  });
  await manualReload(); await waitForModule();
  const repeated = await evaluate('({fixture:window.fixtureResult,state:window.__cockpitEnhancements,name:window.name})');
  assert.equal(await evaluate("window.__cockpitNativeFast.cancelHandoff('loader-nonce')"), true);
  await manualReload(); await waitForModule();
  const canceled = await evaluate('({fixture:window.fixtureResult,install:window.installResult,name:window.name})');
  // Host/browser reload snapshots window.name before beforeunload in this
  // engine. Record the supported boundary instead of reporting false success.
  await evaluate("window.__cockpitNativeFast.prepareHandoff('loader-nonce')");
  await new Promise((resolve, reject) => {
    window.webContents.once('did-finish-load', resolve);
    evaluate("window.__cockpitNativeFast.reloadPreparedDocument('loader-nonce')").then(result => {
      if (!result?.ok) reject(Error(JSON.stringify(result)));
    }, reject);
  });
  await waitForModule();
  await new Promise(resolve => {
    window.webContents.once('did-finish-load', resolve);
    window.webContents.reload();
  });
  await waitForModule();
  const hostReload = await evaluate('({fixture:window.fixtureResult,state:window.__cockpitEnhancements})');
  await evaluate("window.__cockpitNativeFast.cancelHandoff('loader-nonce')");
  assert.equal(await evaluate('window.name'), 'fixture-original-name');
  await send('Page.removeScriptToEvaluateOnNewDocument', { identifier: registered.identifier });
  await window.loadURL('app://-/');
  const restored = await waitForModule();

  // A response-header policy is already active at document start. The loader
  // must report failure instead of treating an unmodified module as success.
  await evaluate(bootstrap);
  assert.equal((await evaluate("window.__cockpitNativeFast.prepareHandoff('header-nonce')")).ok, true);
  const headerRegistration = await send('Page.addScriptToEvaluateOnNewDocument', {
    source: `${bootstrap};window.installResult=window.__cockpitNativeFast.consumeHandoff('header-nonce');`,
  });
  headerCsp = true;
  await new Promise((resolve, reject) => {
    window.webContents.once('did-finish-load', resolve);
    evaluate("window.__cockpitNativeFast.reloadPreparedDocument('header-nonce')").then(result => {
      if (!result?.ok) reject(Error(JSON.stringify(result)));
    }, reject);
  });
  await waitForModule();
  const header = await evaluate(`(async()=>{for(let i=0;i<100&&window.__cockpitEnhancements.nativePatchState!=='unsupported';i++)await new Promise(r=>setTimeout(r,10));
    return {fixture:window.fixtureResult,state:window.__cockpitEnhancements,name:window.name};})()`);
  await evaluate("window.__cockpitNativeFast.cancelHandoff('header-nonce')");
  await send('Page.removeScriptToEvaluateOnNewDocument', { identifier: headerRegistration.identifier });
  // Unsupported quota adaptation must leave the entire native module intact,
  // including its dictation capability, rather than install a partial mode.
  const unsupportedMessagesStart = messages.length;
  headerCsp = false;
  files.set('/assets/app-primary-fixture.js', ['text/javascript', primaryModule.replace('submitDisabled:disabled', 'unsupportedDisabled:disabled')]);
  await window.loadURL('app://-/');
  await waitForModule();
  await evaluate(`window.name='fixture-original-name';${bootstrap}`);
  const unsupportedPrepared = await evaluate("window.__cockpitNativeFast.prepareHandoff('unsupported-nonce')");
  assert.equal(unsupportedPrepared.ok, false, JSON.stringify(unsupportedPrepared));
  const unsupported = await evaluate(`({fixture:window.fixtureResult,
    state:window.__cockpitEnhancements,name:window.name,
    temporaryElements:document.querySelectorAll('script[type=importmap],link[rel=modulepreload]').length})`);
  unsupported.prepared = unsupportedPrepared;
  unsupported.moduleErrors = messages.slice(unsupportedMessagesStart);
  console.log(resultPrefix + JSON.stringify({
    versions: { electron: process.versions.electron, chrome: process.versions.chrome, v8: process.versions.v8 },
    original, waiting: menuInspection, transientProgress, resumed, adapted, inlineScriptBlocked, restored, moduleErrors,
    repeated, canceled, hostReload, header, unsupported, prepared, injectionBytes: injection.length,
  }));
  window.destroy();
  app.exit(0);
}
