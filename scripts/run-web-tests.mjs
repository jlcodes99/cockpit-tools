import { readdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';

const matchArgument = process.argv.find((argument) => argument.startsWith('--match='));
const match = matchArgument?.slice('--match='.length).toLowerCase();

function findTests(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return findTests(path);
    if (!entry.isFile() || !entry.name.endsWith('.test.ts')) return [];
    if (match && !entry.name.toLowerCase().includes(match)) return [];
    return [path];
  });
}

const tests = findTests(resolve('src'));
if (tests.length === 0) {
  throw new Error(`No TypeScript tests matched${match ? ` ${match}` : ''}`);
}

const tsxCli = resolve('node_modules/tsx/dist/cli.mjs');
const result = spawnSync(process.execPath, [tsxCli, '--test', ...tests], {
  cwd: process.cwd(),
  stdio: 'inherit',
  shell: false,
});

if (result.error) throw result.error;
process.exit(result.status ?? 1);
