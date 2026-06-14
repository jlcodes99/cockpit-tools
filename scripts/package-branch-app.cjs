#!/usr/bin/env node

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const scriptRepoRoot = path.resolve(__dirname, '..');
const defaultOutputRoot = path.join(os.homedir(), 'Downloads', 'cockpit-tools-branch-apps');
const alphaStateFile = '.alpha-versions.json';
const generatedConfigRelativePath = path.join('src-tauri', 'tauri.branch-app.generated.conf.json');
const generatedInfoPlistRelativePath = path.join('src-tauri', 'Info.branch-app.generated.plist');

const defaultBundleByPlatform = {
  darwin: 'app',
  linux: 'appimage',
  win32: 'nsis',
};

const artifactSpecs = {
  app: { bundleDir: 'macos', extension: '.app', directory: true },
  dmg: { bundleDir: 'dmg', extension: '.dmg', directory: false },
  appimage: { bundleDir: 'appimage', extension: '.AppImage', directory: false },
  nsis: { bundleDir: 'nsis', extension: '.exe', directory: false },
  msi: { bundleDir: 'msi', extension: '.msi', directory: false },
  deb: { bundleDir: 'deb', extension: '.deb', directory: false },
  rpm: { bundleDir: 'rpm', extension: '.rpm', directory: false },
};

function printHelp() {
  console.log(`Usage:
  npm run package:branch-app -- <branch> [options]

Examples:
  npm run package:branch-app -- startup-page-settings
  npm run package:branch-app -- fix/codex-session-disappearing --install
  npm run package:branch-app -- main --bundle dmg --output ~/Downloads/cockpit-builds

Options:
  --bundle <target>       Override bundle target. Defaults: macOS app, Linux appimage, Windows nsis.
  --output <dir>          Output directory. Default: ~/Downloads/cockpit-tools-branch-apps.
  --install               macOS only. Copy the latest .app to /Applications.
  --install-dir <dir>     Install/overwrite directory for --install. Default: /Applications on macOS.
  --no-open               Do not open the output directory after packaging.
  --skip-install          Do not run npm install in the build worktree.
  --dry-run               Print the resolved build plan without building.
  --help                  Show this help.

Notes:
  - The current branch builds the current working tree, including uncommitted changes.
  - Other branches build an existing worktree for that branch when present, otherwise a temporary detached worktree.
  - Alpha numbers are tracked per branch in dist/branch-apps/.alpha-versions.json.
`);
}

function fail(message) {
  throw new Error(message);
}

function commandName(name) {
  if (process.platform === 'win32' && (name === 'npm' || name === 'npx')) {
    return `${name}.cmd`;
  }
  return name;
}

function run(command, args, options = {}) {
  const result = spawnSync(commandName(command), args, {
    cwd: options.cwd || scriptRepoRoot,
    env: options.env || process.env,
    stdio: options.stdio || 'inherit',
    shell: false,
    encoding: options.encoding,
  });

  if (result.error) {
    throw result.error;
  }

  if (!options.allowFailure && result.status !== 0) {
    const status = typeof result.status === 'number' ? result.status : 1;
    throw new Error(`${command} ${args.join(' ')} failed with exit code ${status}`);
  }

  return result;
}

function capture(command, args, options = {}) {
  const result = run(command, args, {
    ...options,
    stdio: 'pipe',
    encoding: 'utf8',
    allowFailure: true,
  });
  return result.status === 0 ? String(result.stdout || '').trim() : null;
}

function expandHome(input) {
  return input.replace(/^~(?=$|[\\/])/, os.homedir());
}

function parseArgs(argv) {
  const options = {
    branch: null,
    bundleTarget: defaultBundleByPlatform[process.platform] || 'appimage',
    outputRoot: defaultOutputRoot,
    install: false,
    installDir: process.platform === 'darwin' ? '/Applications' : null,
    openOutput: true,
    skipInstall: false,
    dryRun: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];

    if (arg === '--help' || arg === '-h') {
      printHelp();
      process.exit(0);
    }
    if (arg === '--install') {
      options.install = true;
      continue;
    }
    if (arg === '--no-open') {
      options.openOutput = false;
      continue;
    }
    if (arg === '--skip-install') {
      options.skipInstall = true;
      continue;
    }
    if (arg === '--dry-run') {
      options.dryRun = true;
      continue;
    }
    if (arg === '--bundle' || arg === '--output' || arg === '--install-dir') {
      const value = argv[index + 1];
      if (!value || value.startsWith('--')) {
        fail(`${arg} requires a value`);
      }
      index += 1;
      if (arg === '--bundle') {
        options.bundleTarget = value.toLowerCase();
      } else if (arg === '--output') {
        options.outputRoot = path.resolve(expandHome(value));
      } else {
        options.installDir = path.resolve(expandHome(value));
        options.install = true;
      }
      continue;
    }
    if (arg.startsWith('--bundle=')) {
      options.bundleTarget = arg.slice('--bundle='.length).toLowerCase();
      continue;
    }
    if (arg.startsWith('--output=')) {
      options.outputRoot = path.resolve(expandHome(arg.slice('--output='.length)));
      continue;
    }
    if (arg.startsWith('--install-dir=')) {
      options.installDir = path.resolve(expandHome(arg.slice('--install-dir='.length)));
      options.install = true;
      continue;
    }
    if (arg.startsWith('-')) {
      fail(`Unknown option: ${arg}`);
    }
    if (options.branch) {
      fail(`Only one branch name is supported. Extra value: ${arg}`);
    }
    options.branch = arg;
  }

  if (!artifactSpecs[options.bundleTarget]) {
    fail(`Unsupported bundle target "${options.bundleTarget}". Supported: ${Object.keys(artifactSpecs).join(', ')}`);
  }
  if (options.install && process.platform !== 'darwin' && !options.installDir) {
    fail('--install requires --install-dir on this platform');
  }
  return options;
}

function sanitizeBranchName(branch) {
  const safe = branch
    .trim()
    .replace(/^refs\/heads\//, '')
    .replace(/^origin\//, '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64);
  return safe || 'branch';
}

function displayBranchName(branch) {
  const safe = branch
    .trim()
    .replace(/^refs\/heads\//, '')
    .replace(/^origin\//, '')
    .replace(/[^\w.-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 48);
  return safe || 'branch';
}

function bundleIdentifierSuffix(branchSlug) {
  return branchSlug.replace(/[^a-z0-9-]+/g, '-').replace(/-+/g, '.').replace(/^\.+|\.+$/g, '') || 'branch';
}

function getCurrentBranch(cwd) {
  return capture('git', ['branch', '--show-current'], { cwd }) || 'HEAD';
}

function resolveBranchRef(branch) {
  if (capture('git', ['rev-parse', '--verify', `${branch}^{commit}`])) {
    return branch;
  }
  if (capture('git', ['rev-parse', '--verify', `origin/${branch}^{commit}`])) {
    return `origin/${branch}`;
  }
  fail(`Cannot resolve branch "${branch}" locally or from origin/${branch}`);
}

function findExistingWorktreeForBranch(branch) {
  const normalizedBranch = branch.replace(/^refs\/heads\//, '').replace(/^origin\//, '');
  const output = capture('git', ['worktree', 'list', '--porcelain']);
  if (!output) {
    return null;
  }

  for (const record of output.split(/\n\s*\n/u)) {
    const lines = record.split('\n');
    const worktreeLine = lines.find((line) => line.startsWith('worktree '));
    const branchLine = lines.find((line) => line.startsWith('branch '));
    if (!worktreeLine || !branchLine) {
      continue;
    }
    const worktreePath = worktreeLine.slice('worktree '.length).trim();
    const worktreeBranch = branchLine.slice('branch '.length).trim().replace(/^refs\/heads\//, '');
    if (worktreeBranch === normalizedBranch) {
      return worktreePath;
    }
  }
  return null;
}

function readJsonFile(filePath, fallback) {
  try {
    return JSON.parse(fs.readFileSync(filePath, 'utf8'));
  } catch {
    return fallback;
  }
}

function writeJsonFile(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`);
}

function nextAlpha(outputRoot, branchSlug) {
  const statePath = path.join(outputRoot, alphaStateFile);
  const state = readJsonFile(statePath, {});
  const current = Number.isInteger(state[branchSlug]) ? state[branchSlug] : 0;
  const next = current + 1;
  state[branchSlug] = next;
  writeJsonFile(statePath, state);
  return next;
}

function rollbackAlpha(outputRoot, branchSlug, alpha) {
  const statePath = path.join(outputRoot, alphaStateFile);
  const state = readJsonFile(statePath, {});
  if (state[branchSlug] !== alpha) {
    return;
  }
  if (alpha <= 1) {
    delete state[branchSlug];
  } else {
    state[branchSlug] = alpha - 1;
  }
  writeJsonFile(statePath, state);
}

function previewNextAlpha(outputRoot, branchSlug) {
  const statePath = path.join(outputRoot, alphaStateFile);
  const state = readJsonFile(statePath, {});
  const current = Number.isInteger(state[branchSlug]) ? state[branchSlug] : 0;
  return current + 1;
}

function readPackageVersion(worktreePath) {
  const pkg = readJsonFile(path.join(worktreePath, 'package.json'), null);
  if (!pkg?.version) {
    fail('package.json version is missing');
  }
  return String(pkg.version);
}

function writeGeneratedTauriConfig(worktreePath, buildInfo) {
  const configPath = path.join(worktreePath, generatedConfigRelativePath);
  const plistPath = path.join(worktreePath, generatedInfoPlistRelativePath);
  const infoPlistName = path.basename(generatedInfoPlistRelativePath);

  const config = {
    productName: buildInfo.productName,
    version: buildInfo.version,
    identifier: buildInfo.identifier,
    bundle: {
      createUpdaterArtifacts: false,
      macOS: {
        bundleName: buildInfo.productName,
        infoPlist: infoPlistName,
      },
    },
  };

  const plist = `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key>
  <string>${escapeXml(buildInfo.productName)}</string>
  <key>CFBundleName</key>
  <string>${escapeXml(buildInfo.productName)}</string>
  <key>LSRequiresCarbon</key>
  <false/>
</dict>
</plist>
`;

  writeJsonFile(configPath, config);
  fs.writeFileSync(plistPath, plist);
  return { configPath, plistPath };
}

function escapeXml(value) {
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;');
}

function removeGeneratedFiles(worktreePath) {
  fs.rmSync(path.join(worktreePath, generatedConfigRelativePath), { force: true });
  fs.rmSync(path.join(worktreePath, generatedInfoPlistRelativePath), { force: true });
}

function ensureDependencies(worktreePath, skipInstall) {
  if (skipInstall || fs.existsSync(path.join(worktreePath, 'node_modules'))) {
    return;
  }
  const npmArgs = fs.existsSync(path.join(worktreePath, 'package-lock.json')) ? ['ci'] : ['install'];
  run('npm', npmArgs, { cwd: worktreePath });
}

function findBuiltArtifact(worktreePath, bundleTarget) {
  const spec = artifactSpecs[bundleTarget];
  const bundleDirs = [
    path.join(worktreePath, 'target', 'release', 'bundle', spec.bundleDir),
    path.join(worktreePath, 'src-tauri', 'target', 'release', 'bundle', spec.bundleDir),
  ];
  const bundleDir = bundleDirs.find((candidate) => fs.existsSync(candidate));
  if (!bundleDir) {
    fail(`Bundle output directory not found. Checked: ${bundleDirs.join(', ')}`);
  }
  const candidates = fs.readdirSync(bundleDir)
    .filter((name) => name.endsWith(spec.extension))
    .map((name) => path.join(bundleDir, name))
    .sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs);
  if (candidates.length === 0) {
    fail(`No ${spec.extension} artifact found in ${bundleDir}`);
  }
  return candidates[0];
}

function copyArtifact(sourcePath, destinationPath, isDirectory) {
  fs.rmSync(destinationPath, { recursive: true, force: true });
  fs.mkdirSync(path.dirname(destinationPath), { recursive: true });
  if (isDirectory) {
    fs.cpSync(sourcePath, destinationPath, { recursive: true });
  } else {
    fs.copyFileSync(sourcePath, destinationPath);
  }
}

function chmodExecutableFiles(appPath) {
  const macosDir = path.join(appPath, 'Contents', 'MacOS');
  if (!fs.existsSync(macosDir)) {
    return;
  }
  for (const name of fs.readdirSync(macosDir)) {
    const filePath = path.join(macosDir, name);
    if (fs.statSync(filePath).isFile()) {
      fs.chmodSync(filePath, 0o755);
    }
  }
}

function prepareMacApp(appPath) {
  if (process.platform !== 'darwin' || !appPath.endsWith('.app')) {
    return;
  }
  chmodExecutableFiles(appPath);
  run('xattr', ['-dr', 'com.apple.quarantine', appPath], { allowFailure: true });
  run('codesign', ['--force', '--deep', '--sign', '-', appPath]);
  run('codesign', ['--verify', '--deep', '--strict', '--verbose=2', appPath]);
}

function openOutputDirectory(outputDir) {
  if (process.platform === 'darwin') {
    run('open', [outputDir], { allowFailure: true });
  } else if (process.platform === 'win32') {
    run('explorer', [outputDir], { allowFailure: true });
  } else {
    run('xdg-open', [outputDir], { allowFailure: true });
  }
}

function installMacApp(appPath, installDir) {
  if (process.platform !== 'darwin') {
    fail('--install currently supports macOS .app bundles only');
  }
  if (!appPath.endsWith('.app')) {
    fail('--install requires --bundle app on macOS');
  }
  const destination = path.join(installDir || '/Applications', path.basename(appPath));
  copyArtifact(appPath, destination, true);
  prepareMacApp(destination);
  console.log(`Installed: ${destination}`);
}

function resolveBuildWorktree(branch) {
  const currentBranch = getCurrentBranch(scriptRepoRoot);
  if (!branch || branch === currentBranch) {
    return {
      branch: branch || currentBranch,
      worktreePath: scriptRepoRoot,
      tempWorktreePath: null,
      usesCurrentWorktree: true,
    };
  }

  const existingWorktree = findExistingWorktreeForBranch(branch);
  if (existingWorktree) {
    return {
      branch,
      worktreePath: existingWorktree,
      tempWorktreePath: null,
      usesCurrentWorktree: false,
    };
  }

  const branchRef = resolveBranchRef(branch);
  const branchSlug = sanitizeBranchName(branch);
  const tempWorktreePath = path.join(os.tmpdir(), `cockpit-tools-${branchSlug}-${Date.now()}`);
  run('git', ['worktree', 'add', '--detach', tempWorktreePath, branchRef], { cwd: scriptRepoRoot });
  return {
    branch,
    worktreePath: tempWorktreePath,
    tempWorktreePath,
    usesCurrentWorktree: false,
  };
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const buildWorktree = resolveBuildWorktree(options.branch);
  const branchName = buildWorktree.branch;
  const branchSlug = sanitizeBranchName(branchName);
  const branchDisplay = displayBranchName(branchName);
  const alpha = options.dryRun
    ? previewNextAlpha(options.outputRoot, branchSlug)
    : nextAlpha(options.outputRoot, branchSlug);
  const baseVersion = readPackageVersion(buildWorktree.worktreePath);
  const version = `${baseVersion}-alpha${alpha}`;
  const productName = `Cockpit Tools ${branchDisplay} alpha${alpha}`;
  const latestProductName = `Cockpit Tools ${branchDisplay}`;
  const identifier = `com.jlcodes.cockpit-tools.beta.${bundleIdentifierSuffix(branchSlug)}`;
  const spec = artifactSpecs[options.bundleTarget];
  const branchOutputRoot = path.join(options.outputRoot, branchSlug);
  const alphaOutputDir = path.join(branchOutputRoot, `alpha${alpha}`);
  const latestOutputDir = path.join(branchOutputRoot, 'latest');

  const alphaArtifactName = spec.directory
    ? `${productName}${spec.extension}`
    : `${productName.replace(/\s+/g, '.')}${spec.extension}`;
  const latestArtifactName = spec.directory
    ? `${latestProductName}${spec.extension}`
    : `${latestProductName.replace(/\s+/g, '.')}${spec.extension}`;
  const alphaArtifactPath = path.join(alphaOutputDir, alphaArtifactName);
  const latestArtifactPath = path.join(latestOutputDir, latestArtifactName);

  const plan = {
    branch: branchName,
    worktree: buildWorktree.worktreePath,
    bundleTarget: options.bundleTarget,
    version,
    productName,
    identifier,
    alphaOutput: alphaArtifactPath,
    latestOutput: latestArtifactPath,
    install: options.install ? options.installDir : false,
  };
  console.log(JSON.stringify(plan, null, 2));

  if (options.dryRun) {
    return;
  }

  let generated = null;
  let completed = false;
  try {
    ensureDependencies(buildWorktree.worktreePath, options.skipInstall);
    generated = writeGeneratedTauriConfig(buildWorktree.worktreePath, {
      productName,
      version,
      identifier,
    });

    run('npx', ['tauri', 'build', '--bundles', options.bundleTarget, '--config', generated.configPath], {
      cwd: buildWorktree.worktreePath,
    });

    const builtArtifact = findBuiltArtifact(buildWorktree.worktreePath, options.bundleTarget);
    copyArtifact(builtArtifact, alphaArtifactPath, spec.directory);
    copyArtifact(builtArtifact, latestArtifactPath, spec.directory);
    prepareMacApp(alphaArtifactPath);
    prepareMacApp(latestArtifactPath);

    if (options.install) {
      installMacApp(latestArtifactPath, options.installDir);
    }

    if (options.openOutput) {
      openOutputDirectory(branchOutputRoot);
    }

    completed = true;
    console.log(`Packaged alpha${alpha}: ${alphaArtifactPath}`);
    console.log(`Updated latest: ${latestArtifactPath}`);
  } finally {
    if (!completed && !options.dryRun) {
      rollbackAlpha(options.outputRoot, branchSlug, alpha);
    }
    if (generated) {
      removeGeneratedFiles(buildWorktree.worktreePath);
    }
    if (buildWorktree.tempWorktreePath) {
      run('git', ['worktree', 'remove', '--force', buildWorktree.tempWorktreePath], {
        cwd: scriptRepoRoot,
        allowFailure: true,
      });
    }
  }
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
}
