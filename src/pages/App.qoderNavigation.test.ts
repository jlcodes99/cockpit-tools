import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';

test('main-window navigation accepts both Qoder platform pages after startup and clicks', () => {
  const source = ts.createSourceFile('App.tsx', readFileSync(new URL('../App.tsx', import.meta.url), 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
  const names = new Set(['RENDERABLE_PAGE_VALUES', 'RENDERABLE_PAGE_SET', 'normalizeStoredActivePage']);
  const selected = source.statements.filter(node => {
    if (ts.isFunctionDeclaration(node)) return !!node.name && names.has(node.name.text);
    return ts.isVariableStatement(node) && node.declarationList.declarations.some(decl => ts.isIdentifier(decl.name) && names.has(decl.name.text));
  });
  const script = ts.transpileModule(selected.map(node => node.getText(source)).join('\n') + '\nnormalizeStoredActivePage;', { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const normalize = vm.runInNewContext(script) as (value: string) => string | null;
  assert.equal(normalize('qoder'), 'qoder');
  assert.equal(normalize(' qoder-cn '), 'qoder-cn');
  assert.equal(normalize('qoder_cn'), null);
});
