import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const file = "useProviderAccountsPage.ts";
const source = ts.createSourceFile(file, readFileSync(new URL(file, import.meta.url), "utf8"), ts.ScriptTarget.Latest, true);
let cancelCallback = "";
ts.forEachChild(source, function visit(node) {
  if (ts.isVariableDeclaration(node) && node.name.getText(source) === "cancelPendingOauthLogin"
    && node.initializer && ts.isCallExpression(node.initializer)) {
    cancelCallback = node.initializer.arguments[0].getText(source);
  }
  ts.forEachChild(node, visit);
});

test("closing or changing an OAuth page cannot send an unscoped cancellation during startup", async () => {
  assert.ok(cancelCallback);
  const calls: string[] = [];
  const loginId = { current: null as string | null };
  const cancel = vm.runInNewContext(ts.transpileModule(cancelCallback, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText, {
    oauthLoginIdRef: loginId,
    oauthActiveRef: { current: true },
    oauthCompletingRef: { current: false },
    oauthServiceRef: { current: { cancelLogin: async (id: string) => { calls.push(id); } } },
  });
  cancel();
  assert.deepEqual(calls, []);
  loginId.current = "previous-social";
  cancel();
  loginId.current = "new-enterprise";
  await Promise.resolve();
  assert.deepEqual(calls, ["previous-social"]);
});
