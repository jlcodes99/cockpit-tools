import assert from "node:assert/strict";
import test from "node:test";
import type { CodexQuickConfig } from "../types/codex";
import {
  CODEX_LAUNCH_PREVIEW_CONFIG_TIMEOUT,
  createCodexLaunchPreviewConfigService,
} from "./codexLaunchPreviewConfigService";

function config(modelId: string): CodexQuickConfig {
  return {
    context_window_1m: false,
    auto_compact_token_limit: 180_000,
    experimental_model_catalog_enabled: true,
    experimental_model_catalog_available: true,
    experimental_model_catalog_models: [
      { model_id: modelId, display_name: modelId },
    ],
    experimental_model_catalog_default_model_id: modelId,
    experimental_model_catalog_reset_models: [],
    context_management_experimental_mode: false,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, resolve, reject };
}

const flushPromises = () => new Promise<void>((resolve) => setImmediate(resolve));

test("reopening exposes the cached snapshot but each settled load refreshes it", async () => {
  const first = config("first");
  const second = config("second");
  let calls = 0;
  const service = createCodexLaunchPreviewConfigService({
    loader: async () => (++calls === 1 ? first : second),
  });

  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), null);
  assert.equal(await service.loadCodexLaunchPreviewConfig("a"), first);
  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), first);
  assert.equal(await service.loadCodexLaunchPreviewConfig("a"), second);
  assert.equal(calls, 2);
  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), second);
});

test("concurrent reads are single-flight for one instance and isolated across instances", async () => {
  const reads = new Map([
    ["a", deferred<CodexQuickConfig>()],
    ["b", deferred<CodexQuickConfig>()],
  ]);
  const calls: string[] = [];
  const service = createCodexLaunchPreviewConfigService({
    loader: (id) => {
      calls.push(id);
      return reads.get(id)!.promise;
    },
  });
  const first = service.loadCodexLaunchPreviewConfig("a");
  const shared = service.loadCodexLaunchPreviewConfig("a");
  const other = service.loadCodexLaunchPreviewConfig("b");
  assert.equal(first, shared);
  assert.notEqual(first, other);
  await flushPromises();
  assert.deepEqual(calls, ["a", "b"]);

  const a = config("model-a");
  const b = config("model-b");
  reads.get("a")!.resolve(a);
  reads.get("b")!.resolve(b);
  assert.deepEqual(await Promise.all([first, shared, other]), [a, a, b]);
  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), a);
  assert.equal(service.getCachedCodexLaunchPreviewConfig("b"), b);
  assert.equal(service.getCachedCodexLaunchPreviewConfig("c"), null);
});

test("failed reads preserve cached data and release single-flight for retry", async () => {
  const saved = config("saved");
  const refreshed = config("refreshed");
  const failure = new Error("read failed");
  let calls = 0;
  const service = createCodexLaunchPreviewConfigService({
    loader: async () => {
      if (++calls === 1) throw failure;
      return refreshed;
    },
  });
  service.rememberCodexLaunchPreviewConfig("a", saved);
  await assert.rejects(service.loadCodexLaunchPreviewConfig("a"), failure);
  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), saved);
  assert.equal(await service.loadCodexLaunchPreviewConfig("a"), refreshed);
  assert.equal(calls, 2);
});

test("a timed out IPC read can be retried and its late result cannot overwrite the retry", async () => {
  const stale = deferred<CodexQuickConfig>();
  const initial = config("initial");
  const fresh = config("fresh");
  let calls = 0;
  const service = createCodexLaunchPreviewConfigService({
    loader: () => (++calls === 1 ? stale.promise : Promise.resolve(fresh)),
    timeoutMs: 5,
  });
  service.rememberCodexLaunchPreviewConfig("a", initial);
  await assert.rejects(service.loadCodexLaunchPreviewConfig("a"), {
    message: CODEX_LAUNCH_PREVIEW_CONFIG_TIMEOUT,
  });
  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), initial);
  assert.equal(await service.loadCodexLaunchPreviewConfig("a"), fresh);
  stale.resolve(config("stale"));
  await flushPromises();
  assert.equal(calls, 2);
  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), fresh);
});

test("a late timed-out response cannot release a newer read's single-flight", async () => {
  const oldRead = deferred<CodexQuickConfig>();
  const newRead = deferred<CodexQuickConfig>();
  let calls = 0;
  const service = createCodexLaunchPreviewConfigService({
    loader: () => (++calls === 1 ? oldRead.promise : newRead.promise),
    timeoutMs: 20,
  });
  await assert.rejects(service.loadCodexLaunchPreviewConfig("a"), {
    message: CODEX_LAUNCH_PREVIEW_CONFIG_TIMEOUT,
  });
  const retry = service.loadCodexLaunchPreviewConfig("a");
  oldRead.resolve(config("old"));
  await flushPromises();
  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), null);
  assert.equal(service.loadCodexLaunchPreviewConfig("a"), retry);
  const fresh = config("new");
  newRead.resolve(fresh);
  assert.equal(await retry, fresh);
  assert.equal(calls, 2);
});

test("remembering a saved config supersedes an in-flight read for cache and callers", async () => {
  const oldRead = deferred<CodexQuickConfig>();
  const later = config("later");
  let calls = 0;
  const service = createCodexLaunchPreviewConfigService({
    loader: () => (++calls === 1 ? oldRead.promise : Promise.resolve(later)),
  });
  const pending = service.loadCodexLaunchPreviewConfig("a");
  await flushPromises();
  const saved = config("saved");
  service.rememberCodexLaunchPreviewConfig("a", saved);
  assert.equal(await pending, saved);
  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), saved);
  oldRead.resolve(config("stale"));
  await flushPromises();
  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), saved);
  assert.equal(await service.loadCodexLaunchPreviewConfig("a"), later);
  assert.equal(calls, 2);
});

test("late rejection after a save cannot erase saved data or reject settled callers", async () => {
  const oldRead = deferred<CodexQuickConfig>();
  const service = createCodexLaunchPreviewConfigService({
    loader: () => oldRead.promise,
  });
  const pending = service.loadCodexLaunchPreviewConfig("a");
  await flushPromises();
  const saved = config("saved");
  service.rememberCodexLaunchPreviewConfig("a", saved);
  assert.equal(await pending, saved);
  oldRead.reject(new Error("late read failure"));
  await flushPromises();
  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), saved);
});

test("the snapshot cache evicts least-recently-used instances at its limit", () => {
  const service = createCodexLaunchPreviewConfigService({
    loader: async () => config("unused"),
    maxEntries: 2,
  });
  const a = config("a");
  service.rememberCodexLaunchPreviewConfig("a", a);
  service.rememberCodexLaunchPreviewConfig("b", config("b"));
  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), a);
  service.rememberCodexLaunchPreviewConfig("c", config("c"));
  assert.equal(service.getCachedCodexLaunchPreviewConfig("b"), null);
  assert.equal(service.getCachedCodexLaunchPreviewConfig("a"), a);
  assert.notEqual(service.getCachedCodexLaunchPreviewConfig("c"), null);
});
