import { readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const write = process.argv.includes('--write');
const path = resolve('upstreams.lock.json');
const lock = JSON.parse(readFileSync(path, 'utf8'));
const headers = {
  Accept: 'application/vnd.github+json',
  'User-Agent': 'cockpit-upstream-contract-check',
  'X-GitHub-Api-Version': '2022-11-28',
};
if (process.env.GITHUB_TOKEN) headers.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;

async function github(route, optional = false) {
  const response = await fetch(`https://api.github.com/${route}`, { headers });
  if (optional && response.status === 404) return null;
  if (!response.ok) throw new Error(`${route}: GitHub returned ${response.status}`);
  return response.json();
}

const changes = [];
for (const upstream of lock.upstreams) {
  const repository = await github(`repos/${upstream.repo}`);
  const commit = await github(`repos/${upstream.repo}/commits/${repository.default_branch}`);
  const release = await github(`repos/${upstream.repo}/releases/latest`, true);
  const next = {
    defaultBranch: repository.default_branch,
    commitSha: commit.sha,
    version: release?.tag_name ?? upstream.version ?? null,
    license: repository.license?.spdx_id ?? 'NOASSERTION',
  };
  const changedFields = Object.entries(next)
    .filter(([key, value]) => upstream[key] !== value)
    .map(([key]) => key);
  if (changedFields.length === 0) continue;
  changes.push({ id: upstream.id, fields: changedFields, before: upstream.commitSha, after: next.commitSha });
  Object.assign(upstream, next);
  if (!next.license || next.license === 'NOASSERTION') upstream.integrationMode = 'metadata_only';
}

if (changes.length > 0 && write) {
  lock.observedAt = new Date().toISOString();
  writeFileSync(path, `${JSON.stringify(lock, null, 2)}\n`, 'utf8');
}

console.log(JSON.stringify({ changed: changes.length > 0, changes }, null, 2));
