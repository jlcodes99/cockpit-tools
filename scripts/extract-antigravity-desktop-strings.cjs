#!/usr/bin/env node
// Read static UI copy from the installed, official macOS application. This tool
// does not execute any extracted JavaScript or read the Antigravity profile.
const fs = require('node:fs');
const path = require('node:path');
const zlib = require('node:zlib');
const ts = require('typescript');

const maxAssetBytes = 48 * 1024 * 1024;
const copyProperties = new Set([
  'actionLabel', 'aria-description', 'aria-label', 'buttonLabel', 'cancelLabel',
  'caption', 'confirmLabel', 'confirmText', 'confirmationText', 'description',
  'detail', 'displayName', 'emptyMessage', 'emptyTitle', 'errorMessage', 'heading',
  'helpText', 'label', 'message', 'noArchived', 'noConversations', 'noMatches',
  'placeholder', 'subtitle', 'tagDescription', 'tagTitle', 'title', 'tooltip', 'tooltipText',
]);

function readRenderer(binary) {
  let offset = 0;
  while ((offset = binary.indexOf('PK\x03\x04', offset, 'latin1')) !== -1) {
    const entry = offset++;
    if (entry + 37 > binary.length || binary.readUInt16LE(entry + 26) !== 7) continue;
    if (binary.toString('ascii', entry + 30, entry + 37) !== 'main.js') continue;
    const method = binary.readUInt16LE(entry + 8);
    if (method !== 0 && method !== 8) continue;
    const packedSize = binary.readUInt32LE(entry + 18);
    const size = binary.readUInt32LE(entry + 22);
    const start = entry + 37 + binary.readUInt16LE(entry + 28);
    if (!packedSize || !size || size > maxAssetBytes || start + packedSize > binary.length) continue;
    const packed = binary.subarray(start, start + packedSize);
    const source = method === 8 ? zlib.inflateRawSync(packed, { maxOutputLength: maxAssetBytes }) : packed;
    if (source.length === size && source.includes('settings-sidebar')) return source.toString('utf8');
  }
  throw new Error('Antigravity renderer bundle was not found');
}

function* readNativeScripts(archive) {
  const base = 8 + archive.readUInt32LE(4);
  const header = JSON.parse(archive.toString('utf8', 16, 16 + archive.readUInt32LE(12)));
  function* collect(entries, directory) {
    for (const [name, entry] of Object.entries(entries)) {
      const relative = directory ? `${directory}/${name}` : name;
      if (entry.files) {
        yield* collect(entry.files, relative);
      } else if (relative.endsWith('.js') && typeof entry.offset === 'string' && entry.size <= maxAssetBytes) {
        const offset = base + Number(entry.offset);
        if (Number.isSafeInteger(offset) && offset >= base && offset + entry.size <= archive.length) {
          yield [relative, archive.toString('utf8', offset, offset + entry.size)];
        }
      }
    }
  }
  if (header.files.dist?.files) yield* collect(header.files.dist.files, 'dist');
}

function owner(node) {
  for (let current = node; current.parent; current = current.parent) {
    const parent = current.parent;
    if (ts.isPropertyAssignment(parent)) return copyProperties.has(parent.name.text) ? parent.name.text : null;
    if (ts.isJsxAttribute(parent)) return copyProperties.has(parent.name.text) ? parent.name.text : null;
    if (ts.isCallExpression(parent)) {
      const call = parent.expression;
      const name = ts.isPropertyAccessExpression(call) ? call.name.text : ts.isIdentifier(call) ? call.text : '';
      if (name === 'createElement' && parent.arguments.indexOf(current) >= 2) return 'element';
      if (['alert', 'confirm', 'showErrorBox'].includes(name)) return 'dialog';
      return null;
    }
    if (!ts.isParenthesizedExpression(parent) && !ts.isConditionalExpression(parent) &&
        !ts.isArrayLiteralExpression(parent) && !ts.isBinaryExpression(parent)) return null;
  }
  return null;
}

function extract(sourceText, asset) {
  const tree = ts.createSourceFile(asset, sourceText, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);
  const unique = new Map();
  function record(node, text, kind) {
    const value = text.trim().replace(/\s+/gu, ' ');
    if (!/[A-Za-z]{2}/u.test(value) || value.length > 1000 || /(?:^(?:https?:|[.#/@{])|[<>]|\b(?:flex|bg|w|max|px|text)-\w)/iu.test(value)) return;
    if (!unique.has(value)) unique.set(value, { text: value, kind, asset, offset: node.getStart(tree) });
  }
  function visit(node) {
    if (ts.isStringLiteralLike(node)) {
      const kind = owner(node);
      if (kind) record(node, node.text, kind);
    } else if (ts.isTemplateExpression(node) && owner(node)) {
      const text = node.head.text + node.templateSpans.map((span, index) => `{${index}}${span.literal.text}`).join('');
      record(node, text, 'template');
    } else if (ts.isJsxText(node)) {
      record(node, node.text, 'element');
    }
    ts.forEachChild(node, visit);
  }
  visit(tree);
  return [...unique.values()];
}

const app = process.argv[2];
if (!app || process.argv.length !== 3) {
  throw new Error('Usage: node scripts/extract-antigravity-desktop-strings.cjs /Applications/Antigravity.app');
}
const resources = path.join(app, 'Contents', 'Resources');
const renderer = readRenderer(fs.readFileSync(path.join(resources, 'bin', 'language_server')));
const native = fs.readFileSync(path.join(resources, 'app.asar'));
const assets = [['renderer/main.js', renderer], ...readNativeScripts(native)];
const inventory = assets.flatMap(([file, source]) => extract(source, file));
process.stdout.write(`${JSON.stringify(inventory, null, 2)}\n`);
