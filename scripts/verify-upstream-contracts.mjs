import assert from 'node:assert/strict';
import { existsSync, lstatSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const lock = JSON.parse(readFileSync(resolve('upstreams.lock.json'), 'utf8'));
assert.equal(lock.schemaVersion, 1, 'unsupported upstream lock schema');
assert.equal(lock.policy.copyThirdPartySource, false, 'third-party source vendoring must stay disabled');
assert.deepEqual(
  lock.policy.authoritativeAutomaticSwitchSources,
  ['openai_exec_json', 'openai_app_server'],
  'only owned OpenAI protocol streams may authorize automatic switching',
);
assert.ok(Array.isArray(lock.upstreams) && lock.upstreams.length >= 6, 'expected pinned upstreams');

const ids = new Set();
const repos = new Set();
for (const upstream of lock.upstreams) {
  assert.match(upstream.id, /^[a-z0-9-]+$/);
  assert.ok(!ids.has(upstream.id), `duplicate upstream id: ${upstream.id}`);
  ids.add(upstream.id);
  assert.match(upstream.repo, /^[^/]+\/[^/]+$/);
  assert.ok(!repos.has(upstream.repo.toLowerCase()), `duplicate repo: ${upstream.repo}`);
  repos.add(upstream.repo.toLowerCase());
  assert.equal(upstream.url, `https://github.com/${upstream.repo}`);
  assert.match(upstream.commitSha, /^[0-9a-f]{40}$/, `${upstream.id} must pin a full commit SHA`);
  assert.ok(['protocol_contract', 'capability_boundary', 'metadata_only'].includes(upstream.integrationMode));
  assert.ok(Array.isArray(upstream.capabilities) && upstream.capabilities.length > 0);
  assert.ok(Array.isArray(upstream.contractTests) && upstream.contractTests.length > 0);
  if (!upstream.license || upstream.license === 'NOASSERTION') {
    assert.equal(upstream.integrationMode, 'metadata_only', `${upstream.id} has no asserted license`);
  }
  for (const contract of upstream.contractTests) {
    assert.match(contract, /^contracts\/upstream-contracts\//);
    const path = resolve(contract);
    assert.ok(existsSync(path), `missing contract fixture: ${contract}`);
    assert.ok(lstatSync(path).isFile(), `contract fixture must be a regular file: ${contract}`);
  }
}

const boundaries = JSON.parse(
  readFileSync(resolve('contracts/upstream-contracts/fixtures/observer-boundaries.json'), 'utf8'),
);
for (const upstream of lock.upstreams.filter((item) => item.id !== 'openai-codex')) {
  const boundary = boundaries.projects[upstream.id];
  assert.ok(boundary, `missing observation boundary for ${upstream.id}`);
  assert.equal(boundary.authoritativeTerminal, false);
  assert.equal(boundary.mayTriggerAutomaticSwitch, false);
  assert.equal(boundary.maximumConfidence, 'suspected');
}

console.log(`Verified ${lock.upstreams.length} pinned upstream contracts and safety boundaries.`);
