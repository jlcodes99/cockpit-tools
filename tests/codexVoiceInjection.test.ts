import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';

const source = readFileSync(new URL('../src-tauri/src/modules/codex_app_injection.rs', import.meta.url), 'utf8');
const script = source.match(/const VOICE_INSTALL_SCRIPT: &str = r#"([\s\S]*?)"#;/)?.[1];
assert.ok(script, 'The shipped voice injection script must be present');
const config = { endpoint: 'http://127.0.0.1:8317/v1/live', apiKey: 'agt_codex_test' };

function harness(location = { protocol: 'app:', host: '-' }) {
  const calls: { receiver: unknown; args: unknown[] }[] = [];
  const result = {};
  class MessagePort {
    postMessage(...args: unknown[]) { calls.push({ receiver: this, args }); return result; }
  }
  const context = vm.createContext({ location, MessagePort, Headers });
  const original = MessagePort.prototype.postMessage;
  const install = () => vm.runInContext(script!.replace('__COCKPIT_VOICE_CONFIG__', JSON.stringify(config)), context);
  return { calls, result, MessagePort, original, install };
}

function voiceRequest() {
  return {
    method: 'POST',
    url: '/wham/realtime/calls?intent=quicksilver&architecture=avas',
    body: JSON.stringify({ sdp: 'v=0\r\n', session: { model: 'gpt-live-1-codex', audio: { output: { voice: 'arbor' } } } }),
    headers: {
      'Content-Type': 'application/json', 'OpenAI-Alpha': 'quicksilver=v2',
      Authorization: 'Bearer original-test-token', 'Chatgpt-Account-Id': 'original-account',
      'OpenAI-Sentinel-Chat-Requirements-Token': 'integrity-test-token',
      'Thread-Id': 'thread-test', 'x-codex-turn-metadata': '{"thread_source":"cli"}',
    },
    expectedIdentity: { type: 'oauth' },
    cancelToken: 17,
  };
}

const pipeline = (request: unknown) => ['push', ['pipeline', 41, ['transport', 'fetch'], [7, request]]];

test('voice creation preserves RPC and request data while using the instance gateway authentication', () => {
  const h = harness();
  assert.equal(h.install(), true);
  const request = voiceRequest();
  request.url = '/next/voice/calls?version=3&mode=client';
  const before = JSON.stringify(request);
  const message = pipeline(request);
  const transfer = [{}];
  const receiver = new h.MessagePort();
  assert.equal(receiver.postMessage(message, transfer), h.result);
  assert.equal(h.calls[0].receiver, receiver);
  assert.equal(h.calls[0].args[1], transfer);
  const forwarded = h.calls[0].args[0] as any[];
  const rewritten = forwarded[1][3][1];
  assert.equal(rewritten.url, config.endpoint);
  assert.equal(rewritten.headers['X-Cockpit-Voice-Target'], request.url);
  assert.equal(rewritten.headers.Authorization, `Bearer ${config.apiKey}`);
  assert.equal(rewritten.headers['thread-id'], 'thread-test');
  assert.equal(rewritten.headers['x-codex-turn-metadata'], request.headers['x-codex-turn-metadata']);
  assert.equal(rewritten.headers['content-type'], 'application/json');
  assert.equal(rewritten.headers['openai-alpha'], 'quicksilver=v2');
  assert.equal(rewritten.headers['chatgpt-account-id'], undefined);
  assert.equal(rewritten.headers['openai-sentinel-chat-requirements-token'], undefined);
  assert.equal(rewritten.expectedIdentity, undefined);
  assert.equal(rewritten.body, request.body);
  assert.equal(rewritten.cancelToken, request.cancelToken);
  assert.equal(forwarded[1][1], 41);
  assert.equal(forwarded[1][2], (message[1] as any[])[2]);
  assert.equal(forwarded[1][3][0], 7);
  assert.notEqual(forwarded, message);
  assert.equal(JSON.stringify(request), before);
  assert.equal((message[1] as any[])[3][1], request);
});

test('unrelated messages and requests retain the original arguments and return value', () => {
  const h = harness();
  h.install();
  const request = voiceRequest();
  const cases = [
    ['ordinary message', ['ordinary', { id: 1 }]],
    ['other RPC operation', ['push', ['pipeline', 41, ['transport', 'other'], [7, request]]]],
    ['GET', pipeline({ ...request, method: 'GET' })],
    ['non-voice POST', pipeline({ ...request, headers: { 'Content-Type': 'application/json' } })],
    ['other alpha', pipeline({ ...request, headers: { 'OpenAI-Alpha': 'other=v2' } })],
    ['malformed JSON', pipeline({ ...request, body: '{' })],
    ['missing SDP', pipeline({ ...request, body: JSON.stringify({ session: {} }) })],
    ['empty SDP', pipeline({ ...request, body: JSON.stringify({ sdp: ' ', session: {} }) })],
    ['invalid session', pipeline({ ...request, body: JSON.stringify({ sdp: 'v=0', session: [] }) })],
  ] as const;
  for (const [name, message] of cases) {
    const receiver = new h.MessagePort(), transfer = [{}];
    assert.equal(receiver.postMessage(message, transfer), h.result, name);
    const call = h.calls[h.calls.length - 1];
    assert.equal(call.receiver, receiver, name);
    assert.equal(call.args[0], message, name);
    assert.equal(call.args[1], transfer, name);
  }
});

test('invalid original URLs are sent to the gateway with an empty target for rejection', () => {
  const h = harness();
  h.install();
  const request = voiceRequest();
  for (const target of ['https://example.test/calls', '//example.test/calls', '/calls#fragment', '/calls\\escape']) {
    new h.MessagePort().postMessage(pipeline({ ...request, url: target }));
    const rewritten = (h.calls[h.calls.length - 1].args[0] as any[])[1][3][1];
    assert.equal(rewritten.url, config.endpoint, target);
    assert.equal(rewritten.headers['X-Cockpit-Voice-Target'], '', target);
  }
});

test('installation is limited to managed Codex pages and avoids wrapping twice', () => {
  for (const location of [{ protocol: 'https:', host: '-' }, { protocol: 'app:', host: 'other' }]) {
    const h = harness(location);
    assert.equal(h.install(), false);
    assert.equal(h.MessagePort.prototype.postMessage, h.original);
  }
  const h = harness();
  assert.equal(h.install(), true);
  const wrapper = h.MessagePort.prototype.postMessage;
  assert.equal(h.install(), true);
  assert.equal(h.MessagePort.prototype.postMessage, wrapper);
  h.MessagePort.prototype.postMessage = h.original;
  assert.equal(h.install(), false);
  assert.equal(h.MessagePort.prototype.postMessage, h.original);
});
