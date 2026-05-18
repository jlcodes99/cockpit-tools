const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const repoRoot = path.resolve(__dirname, '..');
const proxyRoot = path.join(repoRoot, 'src-tauri', 'sidecars', 'codex-proxy');
const ensureOnly = process.argv.includes('--ensure');
const optional = process.argv.includes('--optional');
const force = process.env.CODEX_PROXY_FORCE_BUILD === '1';
const skip = process.env.CODEX_PROXY_SKIP_BUILD === '1';
const binaryName = process.platform === 'win32' ? 'codex-proxy.exe' : 'codex-proxy';
const resourceDir = path.join(repoRoot, 'src-tauri', 'resources', 'codex-proxy');
const resourceBinary = path.join(resourceDir, binaryName);

function log(message) {
  process.stdout.write(`[codex-proxy] ${message}\n`);
}

function run(command, args, options = {}) {
  log(`${command} ${args.join(' ')}`);
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    stdio: 'inherit',
    shell: process.platform === 'win32',
    env: {
      ...process.env,
      ...(options.env || {}),
    },
  });
  if (result.error) {
    process.stderr.write(`[codex-proxy] Failed to run ${command}: ${result.error.message}\n`);
    process.exit(1);
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function newestMtime(target) {
  if (!fs.existsSync(target)) return 0;
  const stat = fs.statSync(target);
  if (!stat.isDirectory()) return stat.mtimeMs;

  const ignored = new Set(['.git', 'node_modules', 'dist', 'coverage', 'tmp']);
  let newest = stat.mtimeMs;
  for (const entry of fs.readdirSync(target, { withFileTypes: true })) {
    if (ignored.has(entry.name)) continue;
    newest = Math.max(newest, newestMtime(path.join(target, entry.name)));
  }
  return newest;
}

function hasCommand(command, args = ['--version']) {
  const result = spawnSync(command, args, {
    stdio: 'ignore',
    shell: process.platform === 'win32',
  });
  return !result.error && result.status === 0;
}

function stopBecauseMissingBuildTool(tool, installHint) {
  const message = `${tool} is required to build the bundled Codex Proxy. ${installHint}`;
  if (optional) {
    log(`${message} Skipping optional proxy build for local development.`);
    process.exit(0);
  }
  process.stderr.write(`[codex-proxy] ${message}\n`);
  process.exit(1);
}

if (skip) {
  log('Skipping Codex Proxy build because CODEX_PROXY_SKIP_BUILD=1.');
  process.exit(0);
}

if (!fs.existsSync(path.join(proxyRoot, 'main.go'))) {
  const message = 'Codex Proxy source was not found at src-tauri/sidecars/codex-proxy.';
  if (fs.existsSync(resourceBinary)) {
    log(`${message} Existing proxy binary will be reused.`);
    process.exit(0);
  }
  process.stderr.write(`[codex-proxy] ${message}\n`);
  process.exit(1);
}

fs.mkdirSync(resourceDir, { recursive: true });

if (!hasCommand('go', ['version'])) {
  if (ensureOnly && fs.existsSync(resourceBinary) && !force) {
    log(`Go was not found. Using existing ${path.relative(repoRoot, resourceBinary)}.`);
    process.exit(0);
  }
  stopBecauseMissingBuildTool('Go', 'Install Go, then rerun `npm run prepare-codex-proxy`.');
}

if (ensureOnly && !force && fs.existsSync(resourceBinary)) {
  const binaryTime = fs.statSync(resourceBinary).mtimeMs;
  const sourceTime = Math.max(
    newestMtime(path.join(proxyRoot, 'go.mod')),
    newestMtime(path.join(proxyRoot, 'go.sum')),
    newestMtime(path.join(proxyRoot, 'main.go')),
    newestMtime(path.join(proxyRoot, 'internal')),
  );
  if (binaryTime >= sourceTime) {
    log(`Using existing ${path.relative(repoRoot, resourceBinary)}.`);
    process.exit(0);
  }
}

const versionPath = path.join(proxyRoot, 'VERSION');
const version = fs.existsSync(versionPath)
  ? fs.readFileSync(versionPath, 'utf8').trim()
  : 'v0.0.0-cockpit';
const buildTime = new Date().toISOString();
let gitCommit = 'unknown';
const gitResult = spawnSync('git', ['rev-parse', '--short', 'HEAD'], {
  cwd: repoRoot,
  encoding: 'utf8',
});
if (gitResult.status === 0) {
  gitCommit = gitResult.stdout.trim() || gitCommit;
}

const ldflags = [
  `-X main.Version=${version}`,
  `-X main.BuildTime=${buildTime}`,
  `-X main.GitCommit=${gitCommit}`,
  '-s',
  '-w',
].join(' ');

run('go', ['build', '-ldflags', ldflags, '-o', resourceBinary, '.'], {
  cwd: proxyRoot,
  env: { CGO_ENABLED: '0' },
});

if (process.platform !== 'win32') {
  fs.chmodSync(resourceBinary, 0o755);
}

log(`Prepared bundled proxy: ${path.relative(repoRoot, resourceBinary)}`);
