/**
 * 布局持久化回归入口。
 * 运行: node scripts/test-platform-layout-persist.mjs
 * 或: npm run test:platform-layout
 */
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import fs from 'node:fs';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, '..');
const runner = path.join(root, 'scripts/test-platform-layout-persist-runner.ts');
const logHint = 'scripts/test-platform-layout-persist.mjs';

function runWith(args, env = {}) {
  return spawnSync(process.execPath, args, {
    cwd: root,
    encoding: 'utf8',
    env: { ...process.env, NODE_NO_WARNINGS: '1', ...env },
  });
}

// 1) 优先 --import tsx（tsx 作为 devDependency 或 npx 缓存）
const attempts = [
  ['--import', 'tsx', runner],
  ['--import', 'tsx/esm', runner],
];

// 若本地有 node_modules/tsx
const tsxEsm = path.join(root, 'node_modules/tsx/dist/esm/index.mjs');
if (fs.existsSync(path.join(root, 'node_modules/tsx'))) {
  attempts.unshift(['--import', 'tsx', runner]);
}

let last = null;
for (const args of attempts) {
  last = runWith(args);
  if (last.status === 0) {
    process.stdout.write(last.stdout || '');
    if (last.stderr) process.stderr.write(last.stderr);
    process.exit(0);
  }
}

// 2) npx tsx
last = spawnSync('npx', ['--yes', 'tsx', runner], {
  cwd: root,
  encoding: 'utf8',
  env: { ...process.env, NODE_NO_WARNINGS: '1' },
  shell: true,
});
if (last.status === 0) {
  process.stdout.write(last.stdout || '');
  if (last.stderr) process.stderr.write(last.stderr);
  process.exit(0);
}

console.error(`[${logHint}] platform layout tests failed`);
if (last?.stdout) process.stdout.write(last.stdout);
if (last?.stderr) process.stderr.write(last.stderr);
process.exit(last?.status ?? 1);
