import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { isStandardCodexOAuthAccount } from "../../types/codex.ts";

const source = ts.createSourceFile("CodexLaunchPreviewModal.tsx", readFileSync(
  new URL("./CodexLaunchPreviewModal.tsx", import.meta.url), "utf8",
), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const callbacks: string[] = [];
let footerAction = "";
function visit(node: ts.Node) {
  if (ts.isVariableDeclaration(node)
    && ["handleReauthorize", "handleManualRefreshReauthorize"].includes(node.name.getText(source))) {
    assert.ok(node.initializer && ts.isCallExpression(node.initializer));
    callbacks.push(`const ${node.name.getText(source)} = ${node.initializer.arguments[0].getText(source)};`);
  }
  if (ts.isJsxExpression(node) && node.expression
    && node.expression.getText(source).includes("onClick={() => handleReauthorize()}")) {
    footerAction = node.expression.getText(source);
  }
  ts.forEachChild(node, visit);
}
visit(source);
assert.equal(callbacks.length, 2);
assert.ok(footerAction, "the preview footer exposes a direct reauthorization action");
const compiled = ts.transpileModule(`${callbacks.join("\n")}
globalThis.handlers = { handleReauthorize, handleManualRefreshReauthorize };
globalThis.renderAction = () => (${footerAction});`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
}).outputText;

function harness(mode = "account") {
  const events: unknown[] = [];
  const context: Record<string, any> = {
    account: {
      id: "existing-account", auth_mode: "oauth", requires_reauth: false,
      tokens: { access_token: "synthetic-access", refresh_token: "synthetic-refresh" },
    },
    mode, instanceId: "target-instance", busy: false, manualRefreshResult: null,
    isStandardCodexOAuthAccount,
    setManualRefreshResult: (value: unknown) => events.push(["result", value]),
    onClose: () => events.push("close"),
    window: { dispatchEvent: (event: unknown) => events.push(event) },
    CustomEvent: class { constructor(public type: string, public options: unknown) {} },
    requestCodexOpenAddAccount: (detail: unknown) => events.push(["authorize", detail]),
    KeyRound: () => null, t: (key: string) => key,
    React: { createElement: (type: unknown, props: unknown, ...children: unknown[]) => ({ type, props, children }) },
  };
  vm.runInNewContext(compiled, context);
  return { context, events, requests: () => events.filter((event: any) => event?.[0] === "authorize") as any[] };
}

for (const mode of ["account", "instance"]) {
  test(`${mode}: a healthy account can reauthorize without retrying a launch`, () => {
    const h = harness(mode);
    const button = h.context.renderAction();
    assert.equal(button.type, "button");
    assert.equal(button.props.disabled, false);
    assert.equal(button.children.includes("common.reauthorize"), true);
    button.props.onClick();
    assert.equal(h.events.includes("close"), true);
    assert.deepEqual(JSON.parse(JSON.stringify(h.requests())), [["authorize", {
      tab: "oauth", targetAccountId: "existing-account",
    }]]);
  });
}

test("busy previews cannot reauthorize, and API-key/service previews have no direct OAuth action", () => {
  const h = harness();
  h.context.busy = true;
  assert.equal(h.context.renderAction().props.disabled, true);
  h.context.handlers.handleReauthorize();
  assert.deepEqual(h.events, []);
  h.context.busy = false;
  h.context.account = { id: "api-key", auth_mode: "apikey" };
  assert.equal(h.context.renderAction(), false);
  h.context.handlers.handleReauthorize();
  h.context.account = null;
  assert.equal(h.context.renderAction(), null);
  h.context.handlers.handleReauthorize();
  assert.deepEqual(h.events, []);
});

test("refresh-failure recovery retains its existing instance-launch retry", () => {
  const h = harness("instance");
  h.context.handlers.handleManualRefreshReauthorize();
  assert.deepEqual(h.events, []);
  h.context.manualRefreshResult = { status: "error", requiresReauth: true };
  h.context.handlers.handleManualRefreshReauthorize();
  assert.deepEqual(JSON.parse(JSON.stringify(h.requests())), [["authorize", {
    tab: "oauth", targetAccountId: "existing-account",
    retryInstanceLaunchAfterOAuth: true, retryInstanceId: "target-instance",
  }]]);
});
