import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { buildCodexModelRoutingValue, resolveRoutingCatalog } from "../../utils/codexModelRoutingValue.ts";
import { deferred, settlePromises } from "../../../tests/helpers/reactHookHarness";

// Exercise the production callbacks with controlled IPC completion order. This
// deliberately excludes rendering, native dialogs and real account/config data.
const source = ts.createSourceFile("CodexLaunchPreviewModal.tsx", readFileSync(
  new URL("./CodexLaunchPreviewModal.tsx", import.meta.url), "utf8",
), ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const names = ["requestClose", "persistDraft", "handleExecute", "handleInstanceChange", "applyContextConfig", "openModelConfig", "closeModelConfig"];
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
    normalizedRoutingRoutes: [], routingRoutes: [], dirty: true, contextWindowInput: "", compactLimitInput: "",
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

for (const accept of [false, true]) {
  test(`mixed routing requires explicit catalog opt-in before editing: ${accept}`, async () => {
    const h = harness();
    Object.assign(h.c, {
      unavailable: false, routingEnabled: true, catalogEnabled: false,
      models: [{ model_id: 'custom/model', display_name: 'Model', context_window: 272000, auto_compact_token_limit: 262000 }],
      confirmDialog: async () => accept,
      setModelConfigSnapshot: (value: any) => { h.c.modelConfigSnapshot = value; },
      setCatalogEnabled: (value: boolean) => { h.c.catalogEnabled = value; },
      setModelConfigOpen: (value: boolean) => { h.c.modelConfigOpen = value; },
      setModels: (value: any) => { h.c.models = value; },
      setDefaultModelId: (value: any) => { h.c.defaultModelId = value; },
      setModelsError: (value: any) => { h.c.modelsError = value; },
    });
    await h.c.handlers.openModelConfig();
    assert.equal(h.c.catalogEnabled, accept);
    assert.equal(!!h.c.modelConfigOpen, accept);
    if (!accept) { assert.equal(h.c.modelConfigSnapshot, undefined); return; }
    h.c.handlers.closeModelConfig(false);
    assert.equal(h.c.catalogEnabled, false, 'cancel restores the original opt-out');
    await h.c.handlers.openModelConfig();
    h.c.handlers.closeModelConfig(true);
    assert.equal(h.c.catalogEnabled, true);
    h.c.resolveRoutingCatalog = resolveRoutingCatalog;
    let payload: any[] = [];
    h.c.saveCodexInstanceQuickConfig = async (...args: any[]) => {
      payload = args;
      return { experimental_model_catalog_enabled: args[3], experimental_model_catalog_models: args[4] };
    };
    const pending = h.c.handlers.persistDraft(); h.read.resolve({});
    assert.equal(await pending, true);
    assert.equal(payload[3], true, 'backend must receive an enabled persistence policy');
    assert.equal(payload[4][0].context_window, 272000);
    assert.equal(payload[4][0].auto_compact_token_limit, 262000);
    assert.equal((h.cached[0] as any).experimental_model_catalog_models[0].context_window, 272000);
  });
}

test('late model-management confirmation cannot reopen a closed or replaced preview', async () => {
  for (const close of [false, true]) {
    const h = harness();
    const confirmation = deferred<boolean>();
    const draftUpdates: string[] = [];
    Object.assign(h.c, {
      unavailable: false,
      confirmDialog: () => confirmation.promise,
      setModelConfigSnapshot: () => draftUpdates.push('snapshot'),
      setCatalogEnabled: () => draftUpdates.push('enabled'),
      setModelConfigOpen: () => draftUpdates.push('open'),
    });
    const pending = h.c.handlers.openModelConfig();
    if (close) h.c.handlers.requestClose();
    else h.c.configSession.current += 1;
    confirmation.resolve(true);
    await pending;
    assert.deepEqual(draftUpdates, []);
    assert.deepEqual(h.lateUpdates, []);
  }
});

test('canceling per-model edits restores the mixed-routing selection as well as the model draft', async () => {
  const h = harness();
  const route = { id: 'route', namespace: 'relay', providerAccountId: 'provider', enabled: true,
    selectedModels: ['model'], extraModels: ['custom-model'] };
  const initialRouting = buildCodexModelRoutingValue(true, [route]);
  Object.assign(h.c, {
    unavailable: false, catalogEnabled: true, routingRoutes: [route],
    models: [{ model_id: 'relay/model', display_name: 'Model', context_window: 200000 }],
    setModelConfigSnapshot: (value: any) => { h.c.modelConfigSnapshot = value; },
    setCatalogEnabled: (value: boolean) => { h.c.catalogEnabled = value; },
    setModelConfigOpen: () => {}, setModelsError: () => {},
    setModels: (value: any) => { h.c.models = value; },
    setDefaultModelId: (value: any) => { h.c.defaultModelId = value; },
    setRoutingRoutes: (value: any) => { h.c.routingRoutes = value; },
  });
  await h.c.handlers.openModelConfig();
  h.c.routingRoutes[0].selectedModels = [];
  h.c.models = [];
  h.c.handlers.closeModelConfig(false);
  assert.deepEqual(Array.from(h.c.routingRoutes[0].selectedModels), ['model']);
  assert.deepEqual(Array.from(h.c.routingRoutes[0].extraModels), ['custom-model']);
  assert.equal(h.c.models[0].context_window, 200000);
  assert.equal(h.c.catalogEnabled, true);
  let savedPayload: any;
  Object.assign(h.c, {
    routingDirty: true, routingEnabled: true, routingEnabledForSave: true,
    normalizedRoutingRoutes: h.c.routingRoutes,
    nextModelRouting: buildCodexModelRoutingValue(true, h.c.routingRoutes),
    mixedRoutingOAuthAccount: { id: 'oauth' },
    accounts: [{ id: 'provider', auth_mode: 'apikey' }],
    eligibleCodexModelRoutingAccounts: (accounts: any[]) => accounts,
    syncExperimentalModelsWithRouting: (models: any[]) => models,
    resolveRoutingCatalog,
    saveCodexInstanceConfiguration: async (input: any) => {
      savedPayload = input;
      return { quickConfig: {}, instance: h.savedInstance };
    },
  });
  const pending = h.c.handlers.persistDraft();
  h.read.resolve({});
  assert.equal(await pending, true);
  assert.deepEqual(savedPayload.modelRouting, initialRouting);
  assert.equal(savedPayload.experimentalModelCatalogModels[0].context_window, 200000);
});
