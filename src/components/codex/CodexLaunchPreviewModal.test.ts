import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { deferred, settlePromises } from "../../../tests/helpers/reactHookHarness";

// Exercise the production callbacks with controlled IPC completion order. This
// deliberately excludes rendering, native dialogs and real account/config data.
const source = ts.createSourceFile("CodexLaunchPreviewModal.tsx", readFileSync(
  new URL("./CodexLaunchPreviewModal.tsx", import.meta.url), "utf8",
), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const names = ["requestClose", "persistDraft", "handleExecute", "handleInstanceChange", "applyContextConfig"];
const callbacks: string[] = [];
const closeButtons: (string | undefined)[] = [];
function visit(node: ts.Node) {
  if (ts.isVariableDeclaration(node) && names.includes(node.name.getText(source))) {
    assert.ok(node.initializer && ts.isCallExpression(node.initializer));
    callbacks.push(`const ${node.name.getText(source)} = ${node.initializer.arguments[0].getText(source)};`);
  }
  if (ts.isJsxOpeningElement(node) && node.tagName.getText(source) === "button") {
    const attributes = node.attributes.properties.filter(ts.isJsxAttribute);
    const click = attributes.find((attribute) => attribute.name.getText(source) === "onClick");
    if (click?.initializer?.getText(source) === "{requestClose}") {
      closeButtons.push(attributes.find((attribute) => attribute.name.getText(source) === "disabled")?.initializer?.getText(source));
    }
  }
  ts.forEachChild(node, visit);
}
visit(source);
assert.equal(callbacks.length, names.length);
const compiled = ts.transpileModule(callbacks.join("\n") + `\nglobalThis.handlers = {${names.join(",")}};`, {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

function harness(routing = false, overlay = "global-progress-overlay") {
  const read = deferred<any>();
  const write = deferred<any>();
  const events: string[] = [];
  const lateUpdates: string[] = [];
  const cached: unknown[] = [];
  let closed = false;
  let store = { instances: [{ id: "synthetic", saved: false }, { id: "unrelated", saved: false }] };
  const saved = { model: "saved" };
  const savedInstance = { id: "synthetic", saved: true };
  const c: Record<string, any> = {
    configSession: { current: 1 }, configWritePending: { current: false },
    busy: false, checkingConfig: false, configReady: true, configBusy: false, contextConfigSaving: false,
    loadedConfig: {}, loadedInstanceKey: null, configLoadInputs: { current: { selectedInstance: store.instances[0] } },
    catalogEnabled: false, modelsError: null, models: [], defaultModelId: null,
    routingEnabled: false, routingEnabledForSave: false, routingDirty: routing,
    nextModelRouting: { enabled: false, routes: [] }, mixedRoutingBindAccountId: undefined,
    normalizedRoutingRoutes: [], dirty: true, contextWindowInput: "", compactLimitInput: "",
    contextOverrideEnabled: false, mode: "account", instanceId: "synthetic", account: null,
    loadedTarget: "synthetic", previewTargetKey: "synthetic",
    document: { querySelectorAll: () => ["codex-launch-preview-overlay", overlay].map((className) => ({ classList: [className] })) },
    onClose: () => { assert.equal(c.configSession.current, 2); closed = true; events.push("close"); },
    loadCodexLaunchPreviewConfig: () => { events.push("read"); return read.promise; },
    saveCodexInstanceQuickConfig: () => { events.push("quick-write"); return write.promise; },
    saveCodexInstanceConfiguration: () => { events.push("routing-write"); return write.promise; },
    rememberCodexLaunchPreviewConfig: (_id: string, config: unknown) => { cached.push(config); },
    useCodexInstanceStore: { getState: () => store, setState: (next: typeof store) => { store = next; } },
    resolveRoutingCatalog: (models: unknown[]) => ({ enabled: false, models, defaultModelId: null }),
    codexLaunchPreviewQuickConfigKey: () => "same",
    codexLaunchPreviewInstanceConfigKey: (instance: { id: string }) => instance.id,
    onExecute: async () => { events.push("launch"); },
    onInstanceChange: async () => { events.push("switch"); },
    CODEX_LAUNCH_PREVIEW_CONFIG_TIMEOUT: "timeout",
    getCodexExperimentalModelErrorMessage: () => "synthetic failure", t: (key: string) => key,
  };
  for (const name of ["setCheckingConfig", "setNotice", "setError", "setSaving", "setExecuting",
    "setChangingInstance", "setContextConfigError", "setContextConfigSaving", "setConfigLoadError",
    "setLoadedInstanceKey", "setRoutingRoutes", "applyLoadedConfig", "setContextConfigSnapshot", "setContextConfigOpen"]) {
    c[name] = () => { if (closed) lateUpdates.push(name); else events.push(name); };
  }
  vm.runInNewContext(compiled, c);
  return { c, read, write, events, lateUpdates, cached, saved, savedInstance, getStore: () => store,
    completeWrite(context: boolean) { write.resolve(routing && !context ? { quickConfig: saved, instance: savedInstance } : saved); },
  };
}

test("upstream close policy allows the busy footer and global overlays, but retains child protection", () => {
  assert.deepEqual(closeButtons, ["{busy}", undefined]);
  const global = harness(); global.c.busy = true; global.c.handlers.requestClose();
  assert.deepEqual(global.events, ["close"]);
  const child = harness(false, "codex-launch-preview-model-config-overlay");
  child.c.handlers.requestClose();
  assert.deepEqual(child.events, []);
  assert.equal(child.c.configSession.current, 1);
});

const actions = ["persistDraft", "handleExecute", "handleInstanceChange", "applyContextConfig"] as const;
for (const action of ["handleExecute", "handleInstanceChange"]) {
  test(`${action}: a clean draft cannot continue if Close wins the await boundary`, async () => {
    const h = harness(); h.c.dirty = false;
    const pending = h.c.handlers[action](action === "handleInstanceChange" ? "next-synthetic" : true);
    h.c.handlers.requestClose(); await pending;
    assert.deepEqual(h.events, ["close"]);
    assert.deepEqual(h.lateUpdates, []);
  });
}
for (const action of actions) {
  const argument = action === "handleInstanceChange" ? "next-synthetic" : true;
  for (const failed of [false, true]) {
    test(`${action}: closing before config read ${failed ? "fails" : "succeeds"} prevents all later work`, async () => {
      const h = harness(); const pending = h.c.handlers[action](argument);
      h.c.handlers.requestClose();
      if (failed) h.read.reject(new Error("synthetic read failure")); else h.read.resolve({});
      await pending;
      assert.equal(h.events.some((event) => ["quick-write", "routing-write", "launch", "switch"].includes(event)), false);
      assert.deepEqual(h.lateUpdates, []);
      assert.deepEqual(h.cached, []);
    });
  }
  for (const routing of action === "applyContextConfig" ? [false] : [false, true]) {
    for (const close of [false, true]) {
      for (const failed of [false, true]) {
        test(`${action}: ${routing ? "routing" : "quick"} write ${failed ? "failure" : "success"}, ${close ? "closed" : "still open"}`, async () => {
          const h = harness(routing); const pending = h.c.handlers[action](argument);
          h.read.resolve({}); await settlePromises();
          assert.equal(h.events.filter((event) => event.endsWith("-write")).length, 1);
          if (close) h.c.handlers.requestClose();
          if (failed) h.write.reject(new Error("synthetic write failure"));
          else h.completeWrite(action === "applyContextConfig");
          const result = await pending;
          assert.deepEqual(h.lateUpdates, []);
          assert.equal(h.events.includes("launch"), !failed && !close && action === "handleExecute");
          assert.equal(h.events.includes("switch"), !failed && !close && action === "handleInstanceChange");
          assert.equal(h.cached.length, failed ? 0 : 1);
          if (!failed) assert.equal(h.cached[0], h.saved, "completed writes still refresh the shared cache");
          assert.equal(h.getStore().instances[0].saved, !failed && routing, "routing writes still refresh the shared instance store");
          assert.equal(h.getStore().instances[1].id, "unrelated");
          if (!close) assert.equal(h.c.configWritePending.current, false);
          if (action === "persistDraft") assert.equal(result, !failed && !close);
        });
      }
    }
  }
}
