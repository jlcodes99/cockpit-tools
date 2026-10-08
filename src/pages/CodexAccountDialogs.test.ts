import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import postcss from 'postcss';
import ts from 'typescript';

// Structural/CSS regressions only: these assertions inspect the real sources
// without opening a browser. They do not replace visual scroll verification.
const readSource = (name: string) => readFileSync(new URL(name, import.meta.url), 'utf8');
const parseTsx = (name: string) =>
  ts.createSourceFile(name, readSource(name), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const addSource = parseTsx('./CodexAddAccountDialog.tsx');
const overviewSource = parseTsx('./CodexAccountsOverviewPanel.tsx');
const styles = postcss.parse(readSource('./CodexAccountDialogs.css'));

function nodes<T extends ts.Node>(root: ts.Node, predicate: (node: ts.Node) => node is T): T[] {
  const result: T[] = [];
  const visit = (node: ts.Node) => {
    if (predicate(node)) result.push(node);
    ts.forEachChild(node, visit);
  };
  visit(root);
  return result;
}

function unwrap(node: ts.Node): ts.Node {
  return ts.isParenthesizedExpression(node) ? unwrap(node.expression) : node;
}

// Follow only rendered immediate children, treating fragments and conditional
// expressions as transparent, without descending through an actual DOM node.
function renderedElements(node: ts.Node): ts.JsxElement[] {
  const current = unwrap(node);
  if (ts.isJsxElement(current)) return [current];
  if (ts.isJsxFragment(current)) return current.children.flatMap(renderedElements);
  if (ts.isJsxExpression(current) && current.expression) return renderedElements(current.expression);
  if (ts.isBinaryExpression(current) && current.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken) {
    return renderedElements(current.right);
  }
  if (ts.isConditionalExpression(current)) {
    return [...renderedElements(current.whenTrue), ...renderedElements(current.whenFalse)];
  }
  return [];
}

function classes(element: ts.JsxElement): string[] {
  const attribute = element.openingElement.attributes.properties.find(
    (candidate): candidate is ts.JsxAttribute =>
      ts.isJsxAttribute(candidate) && candidate.name.getText() === 'className',
  );
  return attribute?.initializer && ts.isStringLiteral(attribute.initializer)
    ? attribute.initializer.text.split(/\s+/)
    : [];
}

function oneElement(elements: ts.JsxElement[], className: string): ts.JsxElement {
  const matches = elements.filter((element) => classes(element).includes(className));
  assert.equal(matches.length, 1, `Expected one immediate .${className}`);
  return matches[0];
}

function attributeExpression(element: ts.JsxElement, name: string): ts.Expression {
  const attribute = element.openingElement.attributes.properties.find(
    (candidate): candidate is ts.JsxAttribute => ts.isJsxAttribute(candidate) && candidate.name.getText() === name,
  );
  assert.ok(attribute?.initializer && ts.isJsxExpression(attribute.initializer) && attribute.initializer.expression,
    `Expected a ${name} expression`);
  return attribute.initializer.expression;
}

function actionButton(footer: ts.JsxElement, handler: string): ts.JsxElement {
  const matches = nodes(footer, ts.isJsxElement).filter((element) =>
    element.openingElement.tagName.getText() === 'button' &&
    nodes(attributeExpression(element, 'onClick'), ts.isIdentifier).some((identifier) => identifier.text === handler),
  );
  assert.equal(matches.length, 1, `Expected one button for ${handler}`);
  return matches[0];
}

function dialogFor(source: ts.SourceFile, visibility: string) {
  const branches = nodes(source, ts.isBinaryExpression).filter((node) =>
    node.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken &&
    ts.isIdentifier(unwrap(node.left)) && node.left.getText(source) === visibility,
  );
  assert.equal(branches.length, 1, `Expected one ${visibility} visibility branch`);
  const portal = unwrap(branches[0].right);
  assert.ok(ts.isCallExpression(portal), `${visibility} must render through a portal`);
  assert.equal(portal.expression.getText(source), 'createPortal');
  assert.equal(portal.arguments[1]?.getText(source), 'document.body');
  const overlay = oneElement(renderedElements(portal.arguments[0]), 'codex-account-dialog-overlay');
  assert.ok(classes(overlay).includes('modal-overlay'));
  assert.ok(!overlay.openingElement.attributes.properties.some((attribute) =>
    ts.isJsxAttribute(attribute) && attribute.name.getText(source) === 'onClick',
  ), 'The backdrop must not acquire a close-on-click handler');
  const dialog = oneElement(overlay.children.flatMap(renderedElements), 'codex-account-dialog');
  assert.ok(classes(dialog).includes('modal-content'));
  const children = dialog.children.flatMap(renderedElements);
  return {
    dialog,
    header: oneElement(children, 'modal-header'),
    body: oneElement(children, 'modal-body'),
    footer: oneElement(children, 'modal-footer'),
  };
}

const dialogs = [
  {
    name: 'add API Key', source: addSource, visibility: 'showAddModal', submit: 'handleApiKeyLogin',
    ready: { importing: false, addStatus: 'idle', apiModelCatalogFetching: false, apiKeyInput: 'test-key' },
    blocked: [{ importing: true }, { addStatus: 'loading' }, { apiModelCatalogFetching: true }, { apiKeyInput: '  ' }],
  },
  {
    name: 'quick switch', source: overviewSource, visibility: 'quickSwitchAccountId', submit: 'handleSubmitQuickSwitch',
    ready: { quickSwitchSubmitting: false, managedProvidersLoading: false, selectedQuickSwitchProvider: {}, selectedQuickSwitchApiKey: {} },
    blocked: [{ quickSwitchSubmitting: true }, { managedProvidersLoading: true }, { selectedQuickSwitchProvider: null }, { selectedQuickSwitchApiKey: null }],
  },
  {
    name: 'edit API Key', source: overviewSource, visibility: 'editingApiKeyCredentialsId', submit: 'handleSubmitApiKeyCredentials',
    ready: { savingApiKeyCredentials: false, editingApiModelCatalogFetching: false, editingApiKeyCredentialsValue: 'test-key' },
    blocked: [{ savingApiKeyCredentials: true }, { editingApiModelCatalogFetching: true }, { editingApiKeyCredentialsValue: '  ' }],
  },
];

for (const { name, source, visibility, submit, ready, blocked } of dialogs) {
  test(`structure: ${name} uses its own body portal with a fixed-shell action area`, () => {
    const { header, body, footer } = dialogFor(source, visibility);
    assert.ok(nodes(header, ts.isJsxElement).some((element) => classes(element).includes('modal-close')),
      'The close button must stay outside the scrolling form');
    const submitReferences = (root: ts.Node) => nodes(root, ts.isIdentifier).filter((node) => node.text === submit);
    assert.equal(submitReferences(footer).length, 1, `${submit} belongs to the root footer`);
    assert.equal(submitReferences(body).length, 0, `${submit} must not remain inside the scrolling body`);
    assert.ok(source.statements.some((statement) =>
      ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier) &&
      statement.moduleSpecifier.text === './CodexAccountDialogs.css',
    ), 'The shared dialog stylesheet must be loaded by this entry');
  });

  test(`code expression: ${name} retains its submit handler and disabled states after relocation`, () => {
    const { footer } = dialogFor(source, visibility);
    const button = actionButton(footer, submit);
    const disabledSource = attributeExpression(button, 'disabled').getText(source);
    assert.equal(vm.runInNewContext(`(${disabledSource})`, ready), false);
    for (const state of blocked) {
      assert.equal(vm.runInNewContext(`(${disabledSource})`, { ...ready, ...state }), true,
        `The action must remain disabled for ${JSON.stringify(state)}`);
    }
    let calls = 0;
    const onClick = vm.runInNewContext(`(${attributeExpression(button, 'onClick').getText(source)})`, {
      [submit]: () => { calls += 1; },
    });
    onClick();
    assert.equal(calls, 1, 'The relocated button must invoke the original submit handler once');
  });
}

test('structure: the relocated add footer only appears on the API Key tab', () => {
  const { footer, dialog } = dialogFor(addSource, 'showAddModal');
  let ancestor: ts.Node | undefined = footer.parent;
  let apiKeyOnly = false;
  while (ancestor && ancestor !== dialog) {
    if (ts.isBinaryExpression(ancestor) &&
      ancestor.operatorToken.kind === ts.SyntaxKind.AmpersandAmpersandToken) {
      const condition = unwrap(ancestor.left);
      apiKeyOnly ||= ts.isBinaryExpression(condition) &&
        condition.operatorToken.kind === ts.SyntaxKind.EqualsEqualsEqualsToken &&
        ts.isIdentifier(condition.left) && condition.left.text === 'addTab' &&
        ts.isStringLiteral(condition.right) && condition.right.text === 'apikey';
    }
    ancestor = ancestor.parent;
  }
  assert.ok(apiKeyOnly, 'The API Key action must not leak onto OAuth/import/token tabs');
});

test('structure: background scroll locks follow add and both edit visibility states', () => {
  for (const [source, expected] of [
    [addSource, 'Boolean(showAddModal)'],
    [overviewSource, 'Boolean(quickSwitchAccountId||editingApiKeyCredentialsId)'],
  ] as const) {
    assert.ok(source.statements.some((statement) =>
      ts.isImportDeclaration(statement) && ts.isStringLiteral(statement.moduleSpecifier) &&
      statement.moduleSpecifier.text === '../hooks/useModalScrollLock',
    ), 'Each entry must import the shared reference-counted scroll lock');
    const hooks = nodes(source, ts.isCallExpression).filter((node) =>
      ts.isIdentifier(node.expression) && node.expression.text === 'useModalScrollLock',
    );
    assert.equal(hooks.length, 1);
    assert.equal(hooks[0].arguments[0]?.getText(source).replace(/\s+/g, ''), expected);
  }
});

function declarations(selector: string) {
  const values: Record<string, string> = {};
  let found = false;
  styles.walkRules((rule) => {
    if (rule.parent?.type !== 'root' || !rule.selectors.some((item) =>
      item.replace(/\s+/g, ' ').trim() === selector,
    )) return;
    found = true;
    rule.walkDecls((declaration) => { values[declaration.prop] = declaration.value; });
  });
  assert.ok(found, `Missing base CSS rule for ${selector}`);
  return values;
}

test('CSS: viewport bounds and one scrolling body keep the shell and action area reachable', () => {
  const shell = declarations('.modal-content.codex-account-dialog');
  assert.equal(shell.display, 'flex');
  assert.equal(shell['flex-direction'], 'column');
  assert.match(shell['max-height'], /calc\(100d?vh\s*-\s*\d+px\)/);
  assert.equal(shell.overflow, 'hidden');
  const body = declarations('.modal-content.codex-account-dialog > .modal-body');
  assert.equal(body['min-height'], '0');
  assert.equal(body['max-height'], 'inherit', 'Override the legacy independent 70vh body cap');
  assert.equal(body['overflow-y'], 'auto');
  assert.equal(body['overscroll-behavior'], 'contain');
  for (const child of ['modal-header', 'modal-tabs', 'modal-footer']) {
    assert.equal(declarations(`.modal-content.codex-account-dialog > .${child}`)['flex-shrink'], '0');
  }
  assert.equal(declarations('.modal-content.codex-account-dialog > .modal-footer')['flex-wrap'], 'wrap');
});

test('CSS: dialog scroll boundaries contain wheel/touch chaining', () => {
  assert.equal(declarations('.codex-account-dialog-overlay')['overscroll-behavior'], 'contain');
  assert.equal(declarations('.modal-content.codex-account-dialog > .modal-body')['overscroll-behavior'], 'contain');
});
