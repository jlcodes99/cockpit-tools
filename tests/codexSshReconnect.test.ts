import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { settlePromises } from './helpers/reactHookHarness';

function fixture(options: {stale?:boolean; fail?:boolean; closed?:boolean; missingHost?:boolean; apiKey?:boolean; manualRead?:boolean}={}) {
  const listeners=new Set<(event:any)=>void>();
  const calls:any[]=[];
  const timers = new Set<ReturnType<typeof setTimeout>>();
  const startTimer = (callback: () => void, delay: number) => {
    const timer = setTimeout(() => { timers.delete(timer); callback(); }, delay);
    timers.add(timer);
    return timer;
  };
  const stopTimer = (timer: ReturnType<typeof setTimeout>) => {
    clearTimeout(timer);
    timers.delete(timer);
  };
  let reads=0;
  const emit=(data:any)=>listeners.forEach(listener=>listener({data}));
  const request={hostId:'ssh-fixture',requestId:'fixture-request',email:'new@example.invalid',timeoutMs:200};
  const source=readFileSync(new URL('../src-tauri/src/modules/codex_app_ssh_reconnect.js',import.meta.url),'utf8');
  const root:any={
    addEventListener(_name:string,listener:any){listeners.add(listener);},
    removeEventListener(_name:string,listener:any){listeners.delete(listener);},
    electronBridge:{async sendMessageFromView(message:any){
      calls.push(message);
      if(options.fail)throw new Error('fixture bridge failed');
      if(options.missingHost)throw new Error('Connection for host ID fixture not found');
      if(message.type==='mcp-request'){
        reads++;
        if(options.manualRead)return;
        queueMicrotask(()=>emit({type:'mcp-response',hostId:'ssh-fixture',message:{id:message.request.id,
          result:options.apiKey?{account:{type:'apiKey'},requiresOpenaiAuth:false}
            :{account:{type:'chatgpt',email:options.stale?'old@example.invalid':'new@example.invalid'}}}}));
      }
    }},
  };
  if(options.closed)delete root.electronBridge;
  const run=(overrides:Record<string,unknown>={})=>runInNewContext(source.replace('__COCKPIT_SSH_RECONNECT__',JSON.stringify({...request,...overrides})),
    {window:root,location:{protocol:'app:',host:'-'},setTimeout:startTimer,clearTimeout:stopTimer,Promise});
  return {run,calls,emit,listeners,timers,reads:()=>reads};
}

test('native reconnect waits for a new connected event and verifies the new account',async()=>{
  const f=fixture();
  let settled=false;
  const result=Promise.resolve(f.run()).then(value=>{settled=true;return value;});
  await settlePromises();
  assert.equal(f.calls[0].type,'codex-app-server-restart');
  assert.equal(f.calls[0].intent,'restart');
  assert.equal(f.calls[0].killCodexProcess,true);
  assert.equal(settled,false,'IPC enqueue acknowledgement is not a refreshed connection');
  f.emit({type:'codex-app-server-connection-changed',hostId:'ssh-other',state:'connected'});
  f.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connected',isSnapshot:true});
  f.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connected',isSnapshot:false});
  await settlePromises();
  assert.equal(f.reads(),0);
  f.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connecting'});
  f.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connected'});
  assert.equal((await result).ok,true);
  assert.equal(f.reads(),1);
  assert.equal(f.listeners.size,0);
});

test('a stale account is reported even after reconnect and timeout removes listeners',async()=>{
  const stale=fixture({stale:true});
  const result=Promise.resolve(stale.run());
  stale.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connecting'});
  stale.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connected'});
  assert.equal((await result).reason,'account_mismatch');
  const silent=fixture();
  assert.equal((await silent.run()).reason,'timeout');
  assert.equal(silent.listeners.size,0);
});

test('duplicate requests share one reconnect and bridge failures clean up their listener',async()=>{
  const f=fixture();
  const first=Promise.resolve(f.run());
  const second=Promise.resolve(f.run());
  await settlePromises();
  assert.equal(f.calls.filter(call=>call.type==='codex-app-server-restart').length,1);
  f.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connecting'});
  f.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connected'});
  assert.equal((await first).ok,true);
  assert.equal((await second).ok,true);
  const failed=fixture({fail:true});
  assert.equal((await failed.run()).reason,'bridge_failed');
  assert.equal(failed.listeners.size,0);
  const missing=fixture({missingHost:true});
  assert.equal((await missing.run()).reason,'not_ready');
  assert.equal(missing.listeners.size,0);
  const closed=fixture({closed:true});
  assert.equal((await closed.run()).reason,'bridge_unavailable');
  assert.equal(closed.calls.length,0);
});

test('a newer switch retires the old waiter, and API key mode rejects a stale ChatGPT account',async()=>{
  const f=fixture();
  const old=Promise.resolve(f.run());
  const fresh=Promise.resolve(f.run({requestId:'next-request',email:null,authKind:'apiKey'}));
  assert.equal((await old).reason,'superseded');
  f.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connecting'});
  f.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connected'});
  assert.equal((await fresh).reason,'account_mismatch');
  const api=fixture({apiKey:true});
  const result=Promise.resolve(api.run({email:null,authKind:'apiKey'}));
  api.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connecting'});
  api.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connected'});
  assert.equal((await result).ok,true);
  const local=fixture();
  assert.equal((await local.run({hostId:'local'})).reason,'bridge_unavailable');
  assert.equal(local.calls.length,0);
});

test('a startup host that becomes ready can retry the same operation identity',async()=>{
  const options={missingHost:true};
  const f=fixture(options);
  assert.equal((await f.run()).reason,'not_ready');
  options.missingHost=false;
  const retry=Promise.resolve(f.run());
  f.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'restarting'});
  f.emit({type:'codex-app-server-connection-changed',hostId:'ssh-fixture',state:'connected'});
  assert.equal((await retry).ok,true);
  assert.equal(f.calls.filter(call=>call.type==='codex-app-server-restart').length,2);
});

test('native auth handoff retries only account reads and ignores replies from the retired read', async () => {
  const f = fixture({ manualRead: true });
  let settled = false;
  const result = Promise.resolve(f.run({ timeoutMs: 2000 })).then(value => {
    settled = true;
    return value;
  });
  f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'restarting' });
  f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'connected' });
  const firstId = f.calls.at(-1).request.id;
  f.emit({ type: 'mcp-response', hostId: 'ssh-fixture',
    message: { id: firstId, error: { code: -32000, message: 'App server account changed' } } });
  await new Promise(resolve => setTimeout(resolve, 300));
  assert.equal(settled, false, 'a transient native auth handoff must keep waiting for verification');
  assert.equal(f.reads(), 2);
  const retryId = f.calls.at(-1).request.id;
  assert.notEqual(retryId, firstId);
  f.emit({ type: 'mcp-response', hostId: 'ssh-fixture', message: { id: firstId,
    result: { account: { type: 'chatgpt', email: 'old@example.invalid' } } } });
  f.emit({ type: 'mcp-response', hostId: 'ssh-fixture', message: { id: retryId,
    result: { account: { type: 'chatgpt', email: 'new@example.invalid' } } } });
  assert.equal((await result).ok, true);
  assert.equal(f.calls.filter(call => call.type === 'codex-app-server-restart').length, 1);
  assert.equal(f.listeners.size, 0);
});

test('a connection replacement retires its pending account read until the new connection is ready', async () => {
  const f = fixture({ manualRead: true });
  const result = Promise.resolve(f.run({ timeoutMs: 2000 }));
  f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'restarting' });
  f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'connected' });
  const firstId = f.calls.at(-1).request.id;
  f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'disconnected' });
  f.emit({ type: 'mcp-response', hostId: 'ssh-fixture',
    message: { id: firstId, error: { code: -32000, message: 'Codex app-server is not available' } } });
  await new Promise(resolve => setTimeout(resolve, 300));
  assert.equal(f.reads(), 1, 'a disconnected host must wait for its next connected event');
  f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'connecting' });
  f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'connected' });
  assert.equal(f.reads(), 2);
  f.emit({ type: 'mcp-response', hostId: 'ssh-fixture', message: { id: f.calls.at(-1).request.id,
    result: { account: { type: 'chatgpt', email: 'new@example.invalid' } } } });
  assert.equal((await result).ok, true);
  assert.equal(f.listeners.size, 0);
});

test('permanent account read failures remain errors', async () => {
  const f = fixture({ manualRead: true });
  const result = Promise.resolve(f.run());
  f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'restarting' });
  f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'connected' });
  f.emit({ type: 'mcp-response', hostId: 'ssh-fixture',
    message: { id: f.calls.at(-1).request.id, error: { code: -32602, message: 'Invalid params' } } });
  assert.equal((await result).reason, 'account_read_failed');
  assert.equal(f.reads(), 1);
  assert.equal(f.listeners.size, 0);
});

test('verification retries native request errors without depending on their text', async () => {
  for (const error of [
    { code: -32000, message: 'client-disconnected' },
    { code: -32603, message: 'Internal error while replacing the connection' },
  ]) {
    const f = fixture({ manualRead: true });
    const result = Promise.resolve(f.run({ timeoutMs: 2000 }));
    f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'restarting' });
    f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'connected' });
    const firstId = f.calls.at(-1).request.id;
    f.emit({ type: 'mcp-response', hostId: 'ssh-fixture', message: { id: firstId, error } });
    await new Promise(resolve => setTimeout(resolve, 300));
    assert.equal(f.reads(), 2, 'the verification deadline must allow another idempotent read');
    const retryId = f.calls.at(-1).request.id;
    assert.notEqual(retryId, firstId);
    f.emit({ type: 'mcp-response', hostId: 'ssh-fixture', message: { id: retryId,
      result: { account: { type: 'chatgpt', email: 'new@example.invalid' } } } });
    assert.equal((await result).ok, true);
    assert.equal(f.calls.filter(call => call.type === 'codex-app-server-restart').length, 1);
    assert.equal(f.listeners.size, 0);
    assert.equal(f.timers.size, 0);
  }
});

test('a repeated native read failure remains a failure with bounded redacted diagnostics', async () => {
  const f = fixture({ manualRead: true });
  const result = Promise.resolve(f.run({ timeoutMs: 650 }));
  f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'restarting' });
  f.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'connected' });
  const message = 'client-disconnected user@example.invalid eyJhbGciOiJub25lIn0.eyJzdWIiOiIxIn0.signature access_token=secret ' + 'x'.repeat(800);
  const failRead = () => f.emit({ type: 'mcp-response', hostId: 'ssh-fixture',
    message: { id: f.calls.at(-1).request.id, error: { code: -32000, message } } });
  failRead();
  await new Promise(resolve => setTimeout(resolve, 300));
  assert.equal(f.reads(), 2);
  failRead();
  const failure = await result;
  assert.equal(failure.reason, 'account_read_failed');
  assert.equal(failure.readError.code, -32000);
  assert.ok(failure.readError.message.startsWith('client-disconnected'));
  assert.ok(failure.readError.message.length <= 512);
  assert.doesNotMatch(failure.readError.message, /user@example|eyJ|secret/);
  assert.equal(f.listeners.size, 0);
  assert.equal(f.timers.size, 0);
});

test('transient read retries keep the original deadline and stop when superseded', async () => {
  const timed = fixture({ manualRead: true });
  const timedResult = Promise.resolve(timed.run({ timeoutMs: 100 }));
  timed.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'restarting' });
  timed.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'connected' });
  timed.emit({ type: 'mcp-response', hostId: 'ssh-fixture',
    message: { id: timed.calls.at(-1).request.id, error: { code: -32000, message: 'not-connected' } } });
  assert.equal((await timedResult).reason, 'account_read_failed');
  assert.equal(timed.timers.size, 0);
  await new Promise(resolve => setTimeout(resolve, 300));
  assert.equal(timed.reads(), 1);
  assert.equal(timed.listeners.size, 0);

  const cancelled = fixture({ manualRead: true });
  const first = Promise.resolve(cancelled.run({ timeoutMs: 2000 }));
  cancelled.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'restarting' });
  cancelled.emit({ type: 'codex-app-server-connection-changed', hostId: 'ssh-fixture', state: 'connected' });
  cancelled.emit({ type: 'mcp-response', hostId: 'ssh-fixture',
    message: { id: cancelled.calls.at(-1).request.id, error: { code: -32000, message: 'App server account changed' } } });
  const second = Promise.resolve(cancelled.run({ requestId: 'replacement', timeoutMs: 100 }));
  assert.equal((await first).reason, 'superseded');
  assert.equal((await second).reason, 'timeout');
  assert.equal(cancelled.timers.size, 0);
  await new Promise(resolve => setTimeout(resolve, 300));
  assert.equal(cancelled.reads(), 1);
  assert.equal(cancelled.listeners.size, 0);
});
