import assert from "node:assert/strict";
import { beforeEach, describe, it } from "node:test";
import { useCodexBatchImportTaskStore } from "../src/stores/useCodexBatchImportTaskStore.ts";

const snapshot = {
  taskId: " task-1 ",
  sessionId: " session-1 ",
  busy: true,
  current: 2,
  total: 5,
  phase: "scanning",
  checkQuota: true,
  hasPreview: false,
  hasResult: false,
  open: false,
};

describe("codex batch import task store", () => {
  beforeEach(() => {
    useCodexBatchImportTaskStore.setState({
      jobs: {},
      activeTaskId: null,
      reopenTaskId: null,
      reopenNonce: 0,
    });
  });

  it("publishes the active background job as normalized task state", () => {
    useCodexBatchImportTaskStore.getState().publish(snapshot);

    const state = useCodexBatchImportTaskStore.getState();
    assert.equal(state.activeTaskId, "task-1");
    assert.deepEqual(
      {
        taskId: state.jobs["task-1"]?.taskId,
        sessionId: state.jobs["task-1"]?.sessionId,
        busy: state.jobs["task-1"]?.busy,
        current: state.jobs["task-1"]?.current,
        total: state.jobs["task-1"]?.total,
        phase: state.jobs["task-1"]?.phase,
        open: state.jobs["task-1"]?.open,
      },
      {
        taskId: "task-1",
        sessionId: "session-1",
        busy: true,
        current: 2,
        total: 5,
        phase: "scanning",
        open: false,
      },
    );
  });

  it("reopens a background job once, then clears the reopen request", () => {
    const store = useCodexBatchImportTaskStore.getState();
    store.publish(snapshot);
    store.requestReopen("task-1");

    let state = useCodexBatchImportTaskStore.getState();
    assert.equal(state.activeTaskId, "task-1");
    assert.equal(state.reopenTaskId, "task-1");
    assert.equal(state.reopenNonce, 1);
    assert.equal(state.jobs["task-1"]?.open, true);

    state.consumeReopen();
    state = useCodexBatchImportTaskStore.getState();
    assert.equal(state.reopenTaskId, null);
    assert.equal(state.reopenNonce, 1);
  });

  it("removes a completed background job without disturbing unrelated jobs", () => {
    const store = useCodexBatchImportTaskStore.getState();
    store.publish(snapshot);
    store.publish({ ...snapshot, taskId: "task-2", sessionId: "task-2" });
    store.clear("task-1");

    const state = useCodexBatchImportTaskStore.getState();
    assert.equal(state.jobs["task-1"], undefined);
    assert.ok(state.jobs["task-2"]);
    assert.equal(state.activeTaskId, "task-2");
  });
});
