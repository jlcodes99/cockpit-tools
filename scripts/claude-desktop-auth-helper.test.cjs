const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const fs = require('node:fs');
const test = require('node:test');
const vm = require('node:vm');

const source = fs.readFileSync(require.resolve('./claude-desktop-auth-helper.cjs'), 'utf8');
const tick = () => new Promise((resolve) => setImmediate(resolve));
const organization = '00000000-0000-0000-0000-000000000001';
const claudeCookies = (sessionKey) => [
  { domain: '.claude.ai', name: 'sessionKey', value: sessionKey },
  { domain: '.claude.ai', name: 'lastActiveOrg', value: organization },
];

function startLoginHelper() {
  const files = new Map();
  const cookies = new EventEmitter();
  const profileRequests = [];
  let currentCookies = [];
  let cookieReads = 0;
  let loginWindow;

  cookies.get = async () => {
    cookieReads += 1;
    return currentCookies;
  };

  class FakeWindow extends EventEmitter {
    constructor() {
      super();
      loginWindow = this;
      this.webContents = Object.assign(new EventEmitter(), {
        isDestroyed: () => false,
        getURL: () => 'https://claude.ai/',
        setWindowOpenHandler() {},
        executeJavaScript: () => new Promise((resolve) => profileRequests.push(resolve)),
      });
    }
    async loadURL() {
      this.webContents.emit('did-finish-load');
    }
    isDestroyed() { return false; }
    setTitle(title) { this.title = title; }
  }

  const app = Object.assign(new EventEmitter(), {
    commandLine: { appendSwitch() {} },
    setName() {},
    setPath() {},
    quit() {},
    whenReady: async () => {},
  });
  const fakeFs = {
    mkdirSync() {},
    writeFileSync(file, data) { files.set(file, data); },
    rmSync(file) { files.delete(file); },
    renameSync(from, to) {
      files.set(to, files.get(from));
      files.delete(from);
    },
  };
  vm.runInNewContext(source, {
    require(id) {
      if (id === 'electron') return { app, BrowserWindow: FakeWindow, session: { defaultSession: { cookies } } };
      if (id === 'node:fs') return fakeFs;
      return require(id);
    },
    process: { argv: ['', '', '--user-data-dir', '/test', '--status-file', '/test/status', '--export-file', '/test/export'] },
    setImmediate,
    setInterval() { assert.fail('interactive login should not poll cookies'); },
  });

  return {
    cookies, profileRequests,
    get reads() { return cookieReads; },
    get win() { return loginWindow; },
    get exported() { return files.has('/test/export') ? JSON.parse(files.get('/test/export')) : null; },
    changeCookies(next) {
      const previous = currentCookies;
      currentCookies = next;
      for (const cookie of [...previous, ...next]) cookies.emit('changed', {}, cookie);
    },
  };
}

test('interactive login coalesces navigation and cookie events and ignores unrelated cookie changes', async () => {
  const helper = startLoginHelper();
  await tick();
  await tick();
  const idleReads = helper.reads;
  helper.cookies.emit('changed', {}, { domain: '.google.com', name: 'example' });
  await tick();
  assert.equal(helper.reads, idleReads);
  assert.equal(helper.exported, null);

  helper.changeCookies(claudeCookies('first-session'));
  helper.win.webContents.emit('did-navigate', {}, 'https://claude.ai/');
  helper.win.webContents.emit('did-navigate-in-page', {}, 'https://claude.ai/', true);
  await tick();
  assert.equal(helper.reads, idleReads + 1);
  assert.equal(helper.profileRequests.length, 1);
  assert.equal(helper.exported.cookies[0].value, 'first-session');
  assert.equal(helper.exported.webProfile, null);
  assert.match(helper.win.title, /authenticated/);

  helper.profileRequests[0]({ account: 'first' });
  await tick();
  await tick();
  assert.deepEqual(helper.exported.webProfile, { account: 'first' });
  const finishedReads = helper.reads;
  helper.win.emit('closed');
  helper.changeCookies(claudeCookies('after-close'));
  await tick();
  assert.equal(helper.reads, finishedReads);
});

test('a previous login profile result cannot replace the current session export', async () => {
  const helper = startLoginHelper();
  await tick();
  await tick();

  helper.changeCookies(claudeCookies('old-session'));
  await tick();
  helper.changeCookies(claudeCookies('new-session'));
  await tick();
  assert.equal(helper.profileRequests.length, 2);
  assert.equal(helper.exported.cookies[0].value, 'new-session');
  assert.equal(helper.exported.webProfile, null);

  helper.profileRequests[1]({ account: 'current' });
  await tick();
  await tick();
  helper.profileRequests[0]({ account: 'old' });
  await tick();
  await tick();
  assert.equal(helper.exported.cookies[0].value, 'new-session');
  assert.deepEqual(helper.exported.webProfile, { account: 'current' });
});

test('signing out invalidates the temporary login export even if a profile request is still running', async () => {
  const helper = startLoginHelper();
  await tick();
  await tick();

  helper.changeCookies(claudeCookies('signed-out-session'));
  await tick();
  assert.equal(helper.exported.cookies[0].value, 'signed-out-session');
  helper.changeCookies([]);
  await tick();
  assert.equal(helper.exported, null);

  helper.profileRequests[0]({ account: 'signed-out' });
  await tick();
  await tick();
  assert.equal(helper.exported, null);
});
