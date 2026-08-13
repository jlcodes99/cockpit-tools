#!/usr/bin/env node
'use strict';

const childProcess = require('node:child_process');
const path = require('node:path');

const { EXPORTER_VERSION } = require('./exporter.cjs');
const { createExporterWebServer } = require('./web-server.cjs');

function usage() {
  return `Cockpit Account Exporter Web ${EXPORTER_VERSION}

Usage:
  node cockpit-account-exporter-web.cjs [options]

Options:
  --port <0-65535>       Localhost port; 0 selects a free port (default: 0).
  --output-root <path>   Default export root shown in the page.
  --no-open              Do not open the default browser automatically.
  --help                 Show this help.
  --version              Print exporter version.
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
  const options = { port: 0, openBrowser: true };
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    switch (argument) {
      case '--port':
        options.port = Number(requireValue(argv, index, argument));
        index += 1;
        break;
      case '--output-root':
        options.outputRoot = path.resolve(requireValue(argv, index, argument));
        index += 1;
        break;
      case '--no-open':
        options.openBrowser = false;
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
  if (!Number.isInteger(options.port) || options.port < 0 || options.port > 65535) {
    throw new Error('--port must be an integer between 0 and 65535');
  }
  return options;
}

function openUrl(url) {
  let command;
  let args;
  if (process.platform === 'win32') {
    command = 'cmd.exe';
    args = ['/d', '/s', '/c', `start "" "${url}"`];
  } else if (process.platform === 'darwin') {
    command = 'open';
    args = [url];
  } else {
    command = 'xdg-open';
    args = [url];
  }
  const child = childProcess.spawn(command, args, {
    detached: true,
    stdio: 'ignore',
    windowsHide: true,
  });
  child.unref();
}

async function main() {
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

  const repositoryRoot = path.resolve(__dirname, '..', '..');
  const application = createExporterWebServer({
    defaultOutputRoot:
      options.outputRoot || path.join(repositoryRoot, 'cockpit-account-exports'),
  });
  const address = await application.listen(options.port);
  const url = `${address.origin}/`;
  process.stdout.write(`Cockpit Account Exporter Web ${EXPORTER_VERSION}\n`);
  process.stdout.write(`Local page: ${url}\n`);
  process.stdout.write('Listening on 127.0.0.1 only. Press Ctrl+C to stop.\n');
  if (options.openBrowser) {
    openUrl(url);
  }
}

if (require.main === module) {
  main().catch((error) => {
    process.stderr.write(`Unable to start local export page: ${error.message}\n`);
    process.exitCode = 1;
  });
}

module.exports = {
  openUrl,
  parseArguments,
  usage,
};
