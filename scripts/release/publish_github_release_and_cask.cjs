#!/usr/bin/env node

const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawnSync } = require('node:child_process');

const DEFAULT_REPO = 'adxptived/Switchme';
const DEFAULT_CASK_PATH = 'Casks/switchme.rb';
const DEFAULT_TARGET = 'universal-apple-darwin';
const DEFAULT_RELEASE_ASSET_PREFIX = 'Switchme';

function printHelp() {
  console.log('Usage: node scripts/release/publish_github_release_and_cask.cjs [options]');
  console.log('');
  console.log('Build universal DMG, upload it to GitHub Release, and update Homebrew cask.');
  console.log('');
  console.log('Options:');
  console.log('  --skip-build            Skip Tauri universal build step');
  console.log('  --skip-gh               Skip GitHub Release create/upload step');
  console.log('  --skip-cask             Skip cask file update step');
  console.log('  --dry-run               Print planned actions without writing/uploading');
  console.log('  --draft                 Create release as draft when release does not exist');
  console.log('  --generate-notes        Use GitHub generated release notes when creating release');
  console.log('  --notes-file <path>     Pass release notes file to gh release create');
  console.log('  --tag <tag>             Override release tag (default: v<package.json.version>)');
  console.log('  --repo <owner/repo>     GitHub repo for release (default: adxptived/Switchme)');
  console.log('  --cask <path>           Homebrew cask file path (default: Casks/switchme.rb)');
  console.log('  --asset-path <path>     Use an existing universal .dmg instead of default output path');
  console.log('  -h, --help              Show this help');
}

function parseArgs(rawArgs) {
  const options = {
    skipBuild: false,
    skipGh: false,
    skipCask: false,
    dryRun: false,
    draft: false,
    generateNotes: false,
    notesFile: null,
    tag: null,
    repo: DEFAULT_REPO,
    caskPath: DEFAULT_CASK_PATH,
    assetPath: null,
  };

  const args = [...rawArgs];
  while (args.length > 0) {
    const token = args.shift();
    switch (token) {
      case '--skip-build':
        options.skipBuild = true;
        break;
      case '--skip-gh':
        options.skipGh = true;
        break;
      case '--skip-cask':
        options.skipCask = true;
        break;
      case '--dry-run':
        options.dryRun = true;
        break;
      case '--draft':
        options.draft = true;
        break;
      case '--generate-notes':
        options.generateNotes = true;
        break;
      case '--notes-file':
        options.notesFile = requireValue(token, args.shift());
        break;
      case '--tag':
        options.tag = requireValue(token, args.shift());
        break;
      case '--repo':
        options.repo = requireValue(token, args.shift());
        break;
      case '--cask':
        options.caskPath = requireValue(token, args.shift());
        break;
      case '--asset-path':
        options.assetPath = requireValue(token, args.shift());
        break;
      case '-h':
      case '--help':
        printHelp();
        process.exit(0);
        break;
      default:
        throw new Error(`Unknown argument: ${token}`);
    }
  }

  if (options.notesFile && options.generateNotes) {
    throw new Error('Use either --notes-file or --generate-notes, not both.');
  }

  return options;
}

function requireValue(flagName, value) {
  if (!value) {
    throw new Error(`Missing value for ${flagName}`);
  }
  return value;
}

function readPackageVersion() {
  const pkgPath = path.resolve('package.json');
  const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
  if (!pkg.version) {
    throw new Error('package.json.version is missing');
  }
  return pkg.version;
}

function logSection(title) {
  console.log(`\n=== ${title} ===`);
}

function formatCommand(cmd, args) {
  return [cmd, ...args].join(' ');
}

function runCommand(cmd, args, options = {}) {
  const cwd = options.cwd || process.cwd();
  const stdio = options.stdio || 'inherit';
  const shell = process.platform === 'win32';
  const dryRun = Boolean(options.dryRun);
  const allowFailure = Boolean(options.allowFailure);

  console.log(`$ ${formatCommand(cmd, args)}`);
  console.log(`cwd: ${cwd}`);

  if (dryRun) {
    console.log('[dry-run] skipped');
    return { status: 0, stdout: '', stderr: '' };
  }

  const result = spawnSync(cmd, args, {
    cwd,
    env: process.env,
    stdio,
    shell,
    encoding: stdio === 'pipe' ? 'utf8' : undefined,
  });

  if (result.error) {
    if (allowFailure) {
      return result;
    }
    throw result.error;
  }

  if (typeof result.status === 'number' && result.status !== 0 && !allowFailure) {
    throw new Error(`Command failed (exit=${result.status}): ${formatCommand(cmd, args)}`);
  }

  return result;
}

function ensureFileExists(filePath, label) {
  if (!fs.existsSync(filePath)) {
    throw new Error(`${label} not found: ${filePath}`);
  }
}

function buildUniversalIfNeeded(version, options) {
  if (options.skipBuild) {
    console.log('[skip] universal build');
    return;
  }

  logSection('Build Universal DMG');
  runCommand(
    'npm',
    ['run', 'tauri', 'build', '--', '--target', DEFAULT_TARGET],
    { dryRun: options.dryRun }
  );
}

function resolveSourceDmgPath(version, options) {
  if (options.assetPath) {
    const custom = path.resolve(options.assetPath);
    ensureFileExists(custom, 'Asset');
    return custom;
  }

  const defaultPath = path.resolve(
    'src-tauri',
    'target',
    DEFAULT_TARGET,
    'release',
    'bundle',
    'dmg',
    `Switchme_${version}_universal.dmg`
  );

  ensureFileExists(defaultPath, 'Universal DMG');
  return defaultPath;
}

function stageReleaseAsset(sourcePath, version, options) {
  const releaseArtifactsDir = path.resolve('release-artifacts');
  const stagedName = `${DEFAULT_RELEASE_ASSET_PREFIX}_${version}_universal.dmg`;
  const stagedPath = path.join(releaseArtifactsDir, stagedName);

  logSection('Stage Release Asset');
  console.log(`source: ${sourcePath}`);
  console.log(`staged: ${stagedPath}`);

  if (options.dryRun) {
    console.log('[dry-run] skipped copy');
    return { stagedPath, stagedName };
  }

  fs.mkdirSync(releaseArtifactsDir, { recursive: true });
  if (path.resolve(sourcePath) !== path.resolve(stagedPath)) {
    fs.copyFileSync(sourcePath, stagedPath);
  }
  ensureFileExists(stagedPath, 'Staged DMG');
  return { stagedPath, stagedName };
}

function sha256(filePath) {
  const hash = crypto.createHash('sha256');
  const input = fs.readFileSync(filePath);
  hash.update(input);
  return hash.digest('hex');
}

function ensureGhAvailable(options) {
  if (options.skipGh) {
    console.log('[skip] GitHub Release upload');
    return;
  }

  logSection('Check GitHub CLI');
  runCommand('gh', ['--version'], { dryRun: options.dryRun });
  runCommand('gh', ['auth', 'status'], { dryRun: options.dryRun });
}

function ghReleaseExists(tag, repo, options) {
  const result = runCommand(
    'gh',
    ['release', 'view', tag, '--repo', repo],
    {
      stdio: 'pipe',
      dryRun: options.dryRun,
      allowFailure: true,
    }
  );

  if (options.dryRun) {
    console.log('[dry-run] assume release exists check is skipped');
    return false;
  }

  if (result.status === 0) {
    return true;
  }

  const stderr = (result.stderr || '').toString();
  const stdout = (result.stdout || '').toString();
  const combined = `${stdout}\n${stderr}`.trim();
  if (combined) {
    console.log(`[info] gh release view returned non-zero: ${combined}`);
  }
  return false;
}

function uploadToGitHubRelease(tag, repo, stagedPath, options) {
  if (options.skipGh) {
    return;
  }

  logSection('GitHub Release Upload');

  const releaseExists = ghReleaseExists(tag, repo, options);
  if (!releaseExists) {
    const createArgs = ['release', 'create', tag, stagedPath, '--repo', repo, '--title', tag];
    if (options.draft) {
      createArgs.push('--draft');
    }
    if (options.notesFile) {
      createArgs.push('--notes-file', path.resolve(options.notesFile));
    } else if (options.generateNotes) {
      createArgs.push('--generate-notes');
    } else {
      createArgs.push('--notes', `Release ${tag}`);
    }
    runCommand('gh', createArgs, { dryRun: options.dryRun });
    return;
  }

  runCommand(
    'gh',
    ['release', 'upload', tag, stagedPath, '--repo', repo, '--clobber'],
    { dryRun: options.dryRun }
  );
}

function replaceRequired(content, pattern, replacer, label) {
  if (!pattern.test(content)) {
    throw new Error(`Failed to find ${label} in cask file`);
  }
  return content.replace(pattern, replacer);
}

function updateCaskFile(version, digest, options) {
  if (options.skipCask) {
    console.log('[skip] cask update');
    return;
  }

  const caskPath = path.resolve(options.caskPath);
  ensureFileExists(caskPath, 'Cask file');

  logSection('Update Homebrew Cask');
  const original = fs.readFileSync(caskPath, 'utf8');

  let updated = original;
  updated = replaceRequired(
    updated,
    /^(\s*version\s+")([^"]+)(")/m,
    `$1${version}$3`,
    'version'
  );
  updated = replaceRequired(
    updated,
    /^(\s*sha256\s+")([0-9a-fA-F]+)(")/m,
    `$1${digest}$3`,
    'sha256'
  );

  const urlLineMatch = updated.match(/^\s*url\s+"([^"]+)"/m);
  if (!urlLineMatch) {
    throw new Error('Failed to find url in cask file');
  }
  if (!urlLineMatch[1].includes('_universal.dmg')) {
    throw new Error('Cask url does not point to a _universal.dmg asset');
  }

  const versionLine = updated.match(/^\s*version\s+"([^"]+)"/m)?.[0] || '';
  const shaLine = updated.match(/^\s*sha256\s+"([^"]+)"/m)?.[0] || '';

  if (updated === original) {
    console.log('[ok] cask file already matches target version/sha256');
    return;
  }

  if (options.dryRun) {
    const oldVersionLine = original.match(/^\s*version\s+"([^"]+)"/m)?.[0] || '';
    const oldShaLine = original.match(/^\s*sha256\s+"([^"]+)"/m)?.[0] || '';
    console.log('[dry-run] cask changes preview:');
    console.log(`- ${oldVersionLine}`);
    console.log(`+ ${versionLine}`);
    console.log(`- ${oldShaLine}`);
    console.log(`+ ${shaLine}`);
    return;
  }

  fs.writeFileSync(caskPath, updated, 'utf8');
  console.log(`[ok] updated cask: ${path.relative(process.cwd(), caskPath)}`);
  console.log(`[ok] ${versionLine}`);
  console.log(`[ok] ${shaLine}`);
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const version = readPackageVersion();
  const tag = options.tag || `v${version}`;

  console.log('Switchme GitHub Release + Homebrew cask publisher');
  console.log(`version: ${version}`);
  console.log(`tag: ${tag}`);
  console.log(`repo: ${options.repo}`);
  console.log(`dry-run: ${options.dryRun ? 'yes' : 'no'}`);

  buildUniversalIfNeeded(version, options);

  logSection('Resolve Universal DMG');
  const sourceDmgPath = resolveSourceDmgPath(version, options);
  console.log(`asset: ${sourceDmgPath}`);

  const { stagedPath } = stageReleaseAsset(sourceDmgPath, version, options);

  logSection('Compute SHA256');
  const digest = sha256(options.dryRun ? sourceDmgPath : stagedPath);
  console.log(`sha256: ${digest}`);

  ensureGhAvailable(options);
  uploadToGitHubRelease(tag, options.repo, stagedPath, options);
  updateCaskFile(version, digest, options);

  logSection('Done');
  console.log('Next steps (if not using --dry-run):');
  console.log('- Check Casks/switchme.rb diff');
  console.log('- Commit and push cask update (after Release assets are uploaded successfully)');
}

try {
  main();
} catch (error) {
  console.error(`\n[ERROR] ${error.message}`);
  process.exit(1);
}
