// Language-file equality alone cannot detect a key missing from every locale.
const fs = require('node:fs');
const path = require('node:path');
const ts = require('typescript');

function requiredTranslationReferences(source, filename) {
  const ast = ts.createSourceFile(filename, source, ts.ScriptTarget.Latest, true);
  const references = [];
  function literalKeys(node) {
    if (!node) return [];
    if (ts.isStringLiteralLike(node)) return node.text.includes('.') ? [node.text] : [];
    if (ts.isConditionalExpression(node)) return [...literalKeys(node.whenTrue), ...literalKeys(node.whenFalse)];
    if (ts.isParenthesizedExpression(node)) return literalKeys(node.expression);
    return [];
  }
  function visit(node) {
    if (ts.isCallExpression(node) && ['t', 'i18n.t', 'i18next.t'].includes(node.expression.getText(ast))) {
      const options = node.arguments[1];
      const hasDefault = options && (ts.isStringLiteralLike(options) ||
        (ts.isObjectLiteralExpression(options) && options.properties.some(property => property.name?.text === 'defaultValue')));
      if (!hasDefault) {
        for (const key of literalKeys(node.arguments[0])) references.push({
          key, file: filename, line: ast.getLineAndCharacterOfPosition(node.getStart()).line + 1,
        });
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(ast);
  return references;
}

function collectRequiredTranslationReferences(root) {
  const result = [];
  function walk(directory) {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const filename = path.join(directory, entry.name);
      if (entry.isDirectory()) walk(filename);
      else if (/\.tsx?$/.test(filename) && !/\.(test|spec)\.tsx?$/.test(filename)) {
        result.push(...requiredTranslationReferences(fs.readFileSync(filename, 'utf8'), filename));
      }
    }
  }
  walk(root);
  return result;
}

function findMissingTranslationReferences(references, locales) {
  const missing = [];
  for (const reference of references) {
    const languages = [];
    for (const [language, data] of locales) {
      const value = reference.key.split('.').reduce((current, key) => current?.[key], data);
      if (typeof value !== 'string') languages.push(language);
    }
    if (languages.length) missing.push({ ...reference, languages });
  }
  return missing;
}

// Stable backend error/message codes are translated dynamically at the UI boundary.
// Only complete literal keys count; format!("...summary.{}") is not a literal key.
function backendTranslationReferences(source, filename) {
  const result = [];
  const pattern = /"((?:codex\.(?:builtinPolicy|deviceIdentity|proxyQuality|localContentPrivacy|gatewayUsage|network|tokenMeter|modelQuarantine)|pelican\.error)\.[A-Za-z0-9_]+(?:\.[A-Za-z0-9_]+)*)(?="|:\s)/g;
  for (const match of source.matchAll(pattern)) {
    result.push({ key: match[1], file: filename, line: source.slice(0, match.index).split('\n').length });
  }
  return result;
}

function collectBackendTranslationReferences(root) {
  const directory = path.join(root, 'src-tauri/src');
  const references = [];
  function walk(dir) {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const filename = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(filename);
      else if (entry.name.endsWith('.rs')) references.push(...backendTranslationReferences(fs.readFileSync(filename, 'utf8'), filename));
    }
  }
  walk(directory);
  const meterPath = path.join(directory, 'modules/codex_app_token_rates.js');
  if (fs.existsSync(meterPath)) {
    for (const match of fs.readFileSync(meterPath, 'utf8').matchAll(/text\('([A-Za-z0-9_]+)'\)/g)) {
      references.push({ key: `codex.tokenMeter.${match[1]}`, file: meterPath, line: 1 });
    }
  }
  return references;
}

module.exports = { requiredTranslationReferences, collectRequiredTranslationReferences, findMissingTranslationReferences,
  backendTranslationReferences, collectBackendTranslationReferences };
