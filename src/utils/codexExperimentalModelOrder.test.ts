import assert from "node:assert/strict";
import test from "node:test";
import {
  insertModelsBySource,
  modelSourceGroupKey,
  moveModel,
} from "./codexExperimentalModelOrder.ts";

const row = (model_id: string) => ({ model_id });

test("source key prefers resolver label, then namespace, then subscription", () => {
  assert.equal(modelSourceGroupKey("gpt-5.4"), "subscription");
  assert.equal(modelSourceGroupKey("cpa/gpt-5.6-sol"), "api:cpa");
  assert.equal(
    modelSourceGroupKey("cpa/gpt-5.6-sol", () => ({
      kind: "api",
      label: "CPA",
    })),
    "api:cpa",
  );
});

test("new models insert after the last row of the same source", () => {
  const models = [
    row("gpt-5.4"),
    row("cpa/gpt-5.6-sol"),
    row("1024/gpt-5.6-luna"),
  ];
  assert.deepEqual(
    insertModelsBySource(models, [row("cpa/gpt-5.6-terra"), row("cpa/grok-4.6")]).map(
      (item) => item.model_id,
    ),
    [
      "gpt-5.4",
      "cpa/gpt-5.6-sol",
      "cpa/gpt-5.6-terra",
      "cpa/grok-4.6",
      "1024/gpt-5.6-luna",
    ],
  );
});

test("unknown source appends to the end", () => {
  assert.deepEqual(
    insertModelsBySource([row("gpt-5.4")], [row("zhipu-glm/glm-5.3")]).map(
      (item) => item.model_id,
    ),
    ["gpt-5.4", "zhipu-glm/glm-5.3"],
  );
});

test("moveModel swaps neighbors and ignores out of range", () => {
  const models = [row("a"), row("b"), row("c")];
  assert.deepEqual(moveModel(models, 2, 0).map((item) => item.model_id), [
    "c",
    "a",
    "b",
  ]);
  assert.equal(moveModel(models, 0, -1), models);
});
