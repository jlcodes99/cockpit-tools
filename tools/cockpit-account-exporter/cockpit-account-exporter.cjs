#!/usr/bin/env node
'use strict';

const os = require('node:os');
const path = require('node:path');

const {
  DATASET_NAMES,
  DEFAULT_STALE_AFTER_MINUTES,
  EXPORTER_VERSION,
  exportCockpitAccounts,
  normalizeDatasets,
} = require('./exporter.cjs');

function usage() {
  return `Cockpit Account Exporter ${EXPORTER_VERSION}

Offline, read-only export of Cockpit Codex account identities, quota windows,
reset times, and local API usage. No network calls or token refreshes are made.

Usage:
  node cockpit-account-exporter.cjs [options]

Options:
  --profile <production|development>  Select default Cockpit data directory.
  --data-dir <path>                   Override the Cockpit data directory.
  --output-dir <path>                 Empty/new destination directory.
  --plan-family <team|pro|plus|free|all>
                                      Account plan family filter (default: team).
  --format <both|json|csv>            Output format (default: both).
  --datasets <accounts,quota,gateway> Comma-separated datasets (default: all).
  --stale-after-minutes <number>      Snapshot stale threshold (default: 15).
  --skip-invalid                      Report and skip unreadable account details.
  --validate-only                     Read, decrypt, and validate without writing.
  --help                              Show this help.
  --version                           Print exporter version.

Default data directories:
  production:  %USERPROFILE%\\.antigravity_cockpit
  development: %USERPROFILE%\\.antigravity_cockpit_dev
`;
}

function requireValue(argv, index, optionName) {
  const value = argv[index + 1];
  if (!value || value.startsWith('--')) {
    throw new Error(`${optionName} requires a value`);
  }
  return value;
}

function parseArguments(argv) {
  const envProfile = String(process.env.COCKPIT_TOOLS_PROFILE || '').trim().toLowerCase();
  const options = {
    profile: envProfile === 'dev' ? 'development' : 'production',
    planFamily: 'team',
    format: 'both',
    datasets: [...DATASET_NAMES],
    staleAfterMinutes: DEFAULT_STALE_AFTER_MINUTES,
    validateOnly: false,
    skipInvalid: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    switch (argument) {
      case '--profile':
        options.profile = requireValue(argv, index, argument).trim().toLowerCase();
        index += 1;
        break;
      case '--data-dir':
        options.dataDirectory = requireValue(argv, index, argument);
        index += 1;
        break;
      case '--output-dir':
        options.outputDirectory = requireValue(argv, index, argument);
        index += 1;
        break;
      case '--plan-family':
        options.planFamily = requireValue(argv, index, argument).trim().toLowerCase();
        index += 1;
        break;
      case '--format':
        options.format = requireValue(argv, index, argument).trim().toLowerCase();
        index += 1;
        break;
      case '--datasets':
        options.datasets = normalizeDatasets(requireValue(argv, index, argument));
        index += 1;
        break;
      case '--stale-after-minutes':
        options.staleAfterMinutes = Number(requireValue(argv, index, argument));
        index += 1;
        break;
      case '--skip-invalid':
        options.skipInvalid = true;
        break;
      case '--validate-only':
        options.validateOnly = true;
        break;
      case '--help':
        options.help = true;
        break;
      case '--version':
        options.version = true;
        break;
      default:
        throw new Error(`Unknown option: ${argument}`);
    }
  }

  if (!['production', 'development'].includes(options.profile)) {
    throw new Error(`Unsupported profile: ${options.profile}`);
  }
  if (!['team', 'pro', 'plus', 'free', 'all'].includes(options.planFamily)) {
    throw new Error(`Unsupported plan family: ${options.planFamily}`);
  }
  if (!['both', 'json', 'csv'].includes(options.format)) {
    throw new Error(`Unsupported format: ${options.format}`);
  }
  if (
    !Number.isInteger(options.staleAfterMinutes) ||
    options.staleAfterMinutes < 1 ||
    options.staleAfterMinutes > 10080
  ) {
    throw new Error('--stale-after-minutes must be an integer between 1 and 10080');
  }
  if (!options.dataDirectory) {
    const environmentDirectory = String(process.env.COCKPIT_TOOLS_DATA_DIR || '').trim();
    options.dataDirectory =
      environmentDirectory ||
      path.join(
        os.homedir(),
        options.profile === 'development'
          ? '.antigravity_cockpit_dev'
          : '.antigravity_cockpit',
      );
  }
  if (!options.validateOnly && !options.outputDirectory) {
    const timestamp = new Date().toISOString().replaceAll(':', '').replaceAll('-', '').replace(/\.\d{3}Z$/u, 'Z');
    options.outputDirectory = path.join(
      process.cwd(),
      `cockpit-account-export-${options.profile}-${timestamp}`,
    );
  }
  return options;
}

function publicResult(result, validateOnly) {
  return {
    ok: true,
    validateOnly: Boolean(validateOnly),
    outputDirectory: result.outputDirectory,
    summary: result.summary,
  };
}

function main() {
  let options;
  try {
    options = parseArguments(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`Argument error: ${error.message}\n\n${usage()}`);
    process.exitCode = 2;
    return;
  }

  if (options.help) {
    process.stdout.write(usage());
    return;
  }
  if (options.version) {
    process.stdout.write(`${EXPORTER_VERSION}\n`);
    return;
  }

  try {
    const result = exportCockpitAccounts(options);
    process.stdout.write(`${JSON.stringify(publicResult(result, options.validateOnly), null, 2)}\n`);
  } catch (error) {
    process.stderr.write(`Export failed: ${error.message}\n`);
    process.exitCode = 1;
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  parseArguments,
  publicResult,
  usage,
};
