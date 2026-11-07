import assert from 'node:assert/strict';
import test from 'node:test';
import { resolveCodexApiServiceLogModelPair } from './codexApiServiceLogModel';

test('routed requests show the client model with the actual upstream model', () => {
  assert.deepEqual(
    resolveCodexApiServiceLogModelPair({
      modelId: "glm-5.3",
      requestedModel: "cpa/gpt-5.5",
      upstreamModel: "glm-5.3",
    }),
    { requestedModel: "cpa/gpt-5.5", upstreamModel: "glm-5.3" },
  );
});

test('identical upstream models still render the second line', () => {
  assert.deepEqual(
    resolveCodexApiServiceLogModelPair({
      modelId: "gpt-5.4",
      requestedModel: "gpt-5.4",
      upstreamModel: "GPT-5.4",
    }),
    { requestedModel: "gpt-5.4", upstreamModel: "GPT-5.4" },
  );
});

test('rows without a recorded upstream model keep the single-line layout', () => {
  assert.deepEqual(resolveCodexApiServiceLogModelPair({ modelId: "gpt-5.4" }), {
    requestedModel: "gpt-5.4",
    upstreamModel: "",
  });
});

test('legacy logs without the new fields and empty rows stay readable', () => {
  assert.deepEqual(
    resolveCodexApiServiceLogModelPair({
      modelId: "gpt-5.4",
      requestedModel: "   ",
      upstreamModel: null,
    }),
    { requestedModel: "gpt-5.4", upstreamModel: "" },
  );
  assert.deepEqual(resolveCodexApiServiceLogModelPair({}), {
    requestedModel: "--",
    upstreamModel: "",
  });
});
