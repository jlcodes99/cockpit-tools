import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const reconnectScript = readFileSync(new URL('../src-tauri/src/modules/codex_app_ssh_reconnect.js', import.meta.url), 'utf8');

function fixture(fail = false) {
  const persistedEmail = 'new@example.invalid';
  let daemonEmail = 'old@example.invalid';
  let restarts = 0;
  const listeners = new Set<(event: any) => void>();
  const calls: any[] = [];
  const emit = (data: any) => listeners.forEach(listener => listener({ data }));
  const window = {
    addEventListener(_type: string, listener: any) { listeners.add(listener); },
    removeEventListener(_type: string, listener: any) { listeners.delete(listener); },
    electronBridge: { async sendMessageFromView(message: any) {
      calls.push(message);
      if (message.type === 'codex-app-server-restart') {
        if (fail) throw new Error('Native process restart failed');
        // A transport reconnect keeps the existing process and its cached auth.
        if (message.killCodexProcess) { daemonEmail = persistedEmail; restarts++; }
        emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'restarting' });
        emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'connected' });
      } else if (message.type === 'mcp-request') {
        emit({ type: 'mcp-response', hostId: 'ssh-fixture', message: { id: message.request.id,
          result: { account: { type: 'chatgpt', email: daemonEmail } } } });
      }
    } },
  };
  const request = { hostId: 'ssh-fixture', requestId: 'persisted-daemon', email: 'new@example.invalid', authKind: 'chatgpt' };
  return { run: () => runInNewContext(reconnectScript.replace('__COCKPIT_SSH_RECONNECT__', JSON.stringify(request)),
    { window, location: { protocol: 'app:', host: '-' }, setTimeout, clearTimeout, Promise }), calls, restarts: () => restarts };
}

test('switch refresh adopts the new auth even when native reconnect reuses an existing daemon', async () => {
  const f = fixture();
  assert.equal((await f.run()).ok, true, 'transport reconnect alone must not leave the daemon on its old account');
  assert.equal(f.restarts(), 1);
  assert.equal(f.calls[0].intent, 'restart');
  assert.equal(f.calls[0].killCodexProcess, true);
});

test('failed native daemon restart is reported without claiming a refreshed account', async () => {
  const f = fixture(true);
  assert.equal((await f.run()).reason, 'bridge_failed');
  assert.equal(f.calls.some(call => call.type === 'mcp-request'), false);
  assert.equal(f.restarts(), 0);
});
