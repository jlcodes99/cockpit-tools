#!/usr/bin/env node

// Build a Windows portable archive from the files emitted by `tauri build`.
// The archive mirrors the layout used by the installed Tauri application:
// the executable and native DLLs are at the root, with Tauri resources in
// the sibling `resources` directory.

const fs = require('node:fs');
const path = require('node:path');
const zlib = require('node:zlib');

function parseArgs(argv) {
  const args = {};
  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (!token.startsWith('--')) continue;
    const key = token.slice(2);
    const value = argv[index + 1];
    if (!value || value.startsWith('--')) {
      args[key] = 'true';
      continue;
    }
    args[key] = value;
    index += 1;
  }
  return args;
}

function requiredArg(args, key) {
  const value = args[key];
  if (!value) throw new Error(`Missing required argument --${key}`);
  return value;
}

function findExecutable(releaseDir, explicitPath) {
  const candidates = explicitPath
    ? [explicitPath]
    : [
        path.join(releaseDir, 'cockpit_tools.exe'),
        path.join(releaseDir, 'cockpit-tools.exe'),
        path.join(releaseDir, 'Cockpit.Tools.exe'),
        path.join(releaseDir, 'Cockpit Tools.exe'),
      ];
  const executable = candidates.find((candidate) => fs.existsSync(candidate));
  if (!executable) {
    throw new Error(`Windows application executable not found in ${releaseDir}`);
  }
  return path.resolve(executable);
}

function listFilesRecursive(rootDir) {
  const result = [];
  const entries = fs.readdirSync(rootDir, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = path.join(rootDir, entry.name);
    if (entry.isDirectory()) {
      result.push(...listFilesRecursive(fullPath));
    } else if (entry.isFile()) {
      result.push(fullPath);
    }
  }
  return result;
}

function collectPortableFiles({ releaseDir, executablePath }) {
  const files = [{ sourcePath: executablePath, archivePath: path.basename(executablePath) }];

  // Tauri places externalBin and bundle resources in this directory. Keep the
  // directory name because the runtime resolves it relative to the executable.
  const resourcesDir = path.join(releaseDir, 'resources');
  if (fs.existsSync(resourcesDir)) {
    for (const sourcePath of listFilesRecursive(resourcesDir)) {
      files.push({
        sourcePath,
        archivePath: path.join('resources', path.relative(resourcesDir, sourcePath)),
      });
    }
  }

  // Some Windows toolchains emit a loader/runtime DLL beside the executable.
  // Include those DLLs when present, without copying unrelated build outputs.
  for (const entry of fs.readdirSync(path.dirname(executablePath), { withFileTypes: true })) {
    if (
      entry.isFile() &&
      (entry.name.toLowerCase().endsWith('.dll') ||
        /^cockpit-cliproxy(?:[-_].*)?\.exe$/i.test(entry.name))
    ) {
      files.push({
        sourcePath: path.join(path.dirname(executablePath), entry.name),
        archivePath: entry.name,
      });
    }
  }

  return files.sort((left, right) => left.archivePath.localeCompare(right.archivePath));
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function dosDateTime(date = new Date()) {
  const year = Math.max(1980, date.getFullYear());
  return {
    date: ((year - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate(),
    time: (date.getHours() << 11) | (date.getMinutes() << 5) | Math.floor(date.getSeconds() / 2),
  };
}

function u16(value) {
  const buffer = Buffer.alloc(2);
  buffer.writeUInt16LE(value & 0xffff);
  return buffer;
}

function u32(value) {
  const buffer = Buffer.alloc(4);
  buffer.writeUInt32LE(value >>> 0);
  return buffer;
}

function createZip(entries) {
  const localRecords = [];
  const centralRecords = [];
  let offset = 0;
  const timestamp = dosDateTime();

  for (const entry of entries) {
    const name = Buffer.from(entry.archivePath.replaceAll(path.sep, '/'));
    const source = Buffer.isBuffer(entry.content) ? entry.content : fs.readFileSync(entry.sourcePath);
    const compressed = zlib.deflateRawSync(source, { level: 9 });
    const checksum = crc32(source);
    const local = Buffer.concat([
      u32(0x04034b50),
      u16(20),
      u16(0),
      u16(8),
      u16(timestamp.time),
      u16(timestamp.date),
      u32(checksum),
      u32(compressed.length),
      u32(source.length),
      u16(name.length),
      u16(0),
      name,
      compressed,
    ]);
    localRecords.push(local);

    centralRecords.push(Buffer.concat([
      u32(0x02014b50),
      u16(20),
      u16(20),
      u16(0),
      u16(8),
      u16(timestamp.time),
      u16(timestamp.date),
      u32(checksum),
      u32(compressed.length),
      u32(source.length),
      u16(name.length),
      u16(0),
      u16(0),
      u16(0),
      u16(0),
      u32(0),
      u32(offset),
      name,
    ]));
    offset += local.length;
  }

  const centralDirectory = Buffer.concat(centralRecords);
  const localDirectory = Buffer.concat(localRecords);
  const end = Buffer.concat([
    u32(0x06054b50),
    u16(0),
    u16(0),
    u16(entries.length),
    u16(entries.length),
    u32(centralDirectory.length),
    u32(localDirectory.length),
    u16(0),
  ]);
  return Buffer.concat([localDirectory, centralDirectory, end]);
}

function buildWindowsPortableArchive({ releaseDir, outputDir, version, executable }) {
  const releaseRoot = path.resolve(releaseDir);
  const executablePath = findExecutable(releaseRoot, executable && path.resolve(executable));
  const files = collectPortableFiles({ releaseDir: releaseRoot, executablePath });
  const archiveRoot = `Cockpit.Tools_${version}_x64-portable`;
  const entries = files.map((file) => ({
    sourcePath: file.sourcePath,
    archivePath: path.join(archiveRoot, file.archivePath),
  }));
  entries.push({
    archivePath: path.join(archiveRoot, 'README.txt'),
    content: Buffer.from(
      'Cockpit Tools portable edition\r\n\r\n' +
      `Extract this folder and run ${path.basename(executablePath)}.\r\n` +
        'Windows 10/11 with Microsoft Edge WebView2 Runtime is required.\r\n' +
        '账号和配置仍按当前 Windows 用户目录保存，不会随 ZIP 文件夹自动迁移。\r\n',
      'utf8',
    ),
  });

  fs.mkdirSync(outputDir, { recursive: true });
  const outputPath = path.join(outputDir, `${archiveRoot}.zip`);
  fs.writeFileSync(outputPath, createZip(entries));
  return outputPath;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const outputPath = buildWindowsPortableArchive({
    releaseDir: requiredArg(args, 'release-dir'),
    outputDir: requiredArg(args, 'output-dir'),
    version: requiredArg(args, 'version'),
    executable: args.executable,
  });
  console.log(`Windows portable archive created at ${outputPath}`);
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`[build_windows_portable] ${error.message}`);
    process.exit(1);
  }
}

module.exports = {
  buildWindowsPortableArchive,
  collectPortableFiles,
  createZip,
  findExecutable,
};
