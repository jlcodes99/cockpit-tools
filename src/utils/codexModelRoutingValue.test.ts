import assert from "node:assert/strict";
import test from "node:test";
import { areCodexModelRoutingsEqual, buildCodexModelRoutingValue, resolveRoutingCatalog } from "./codexModelRoutingValue.ts";

const routes = [{
  id: "cpa", namespace: "cpa", providerAccountId: "account",
  enabled: true, selectedModels: ["gpt-6-astra"], extraModels: ["custom"],
}];

test("disable persists an explicit value and preserves channel selections", () => {
  const disabled = JSON.parse(JSON.stringify(buildCodexModelRoutingValue(false, routes)));
  assert.equal(disabled.enabled, false);
  assert.deepEqual(disabled.routes, routes);
  assert.deepEqual(buildCodexModelRoutingValue(true, disabled.routes).routes, routes);
});
test("legacy empty state is not dirty, but disabled route edits are", () => {
  assert.equal(areCodexModelRoutingsEqual(null, buildCodexModelRoutingValue(false, [])), true);
  assert.equal(areCodexModelRoutingsEqual(buildCodexModelRoutingValue(false, routes), null), false);
});
test("payload does not mutate the form", () => {
  const saved = buildCodexModelRoutingValue(false, routes);
  saved.routes[0].selectedModels?.push("another");
  assert.deepEqual(routes[0].selectedModels, ["gpt-6-astra"]);
});
test("empty official catalog returns control to Codex", () => {
  assert.deepEqual(resolveRoutingCatalog([], true, "cpa/gpt-6-astra"), {
    enabled: false, models: [], defaultModelId: null,
  });
});
test("removed default resets while official model settings remain", () => {
  const models = [{ model_id: "gpt-6-astra", context_window: 1050000 }];
  assert.deepEqual(resolveRoutingCatalog(models, true, "cpa/gpt-6-astra"), {
    enabled: true, models, defaultModelId: null,
  });
  assert.equal(resolveRoutingCatalog(models, true, "gpt-6-astra").defaultModelId, "gpt-6-astra");
});
