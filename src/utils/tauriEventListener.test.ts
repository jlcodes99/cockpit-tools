import assert from "node:assert/strict";
import test from "node:test";
import {
  createSafeTauriUnlisten,
  installTauriEventCleanupGuard,
  isStaleTauriListenerError,
} from "./tauriEventListener";

test("safe Tauri cleanup runs only once", () => {
  let calls = 0;
  const cleanup = createSafeTauriUnlisten(
    () => {
      calls += 1;
    },
    "test:event",
  );

  cleanup();
  cleanup();

  assert.equal(calls, 1);
});

test("safe Tauri cleanup consumes stale async rejection", async () => {
  const errors: unknown[] = [];
  const cleanup = createSafeTauriUnlisten(
    () => Promise.reject(new TypeError(
      "undefined is not an object (evaluating 'listeners[eventId].handlerId')",
    )),
    "test:event",
    (error) => errors.push(error),
  );

  cleanup();
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(errors.length, 1);
  assert.equal(isStaleTauriListenerError(errors[0]), true);
});

test("global Tauri cleanup guard ignores only missing listener ids", () => {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  let stale = true;
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: {
      __TAURI_EVENT_PLUGIN_INTERNALS__: {
        unregisterListener: () => {
          if (stale) {
            throw new TypeError(
              "undefined is not an object (evaluating 'listeners[eventId].handlerId')",
            );
          }
          throw new Error("unexpected cleanup failure");
        },
      },
    },
  });

  try {
    assert.equal(installTauriEventCleanupGuard(), true);
    assert.equal(installTauriEventCleanupGuard(), false);
    assert.doesNotThrow(() => {
      window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener("test:event", 7);
    });
    stale = false;
    assert.throws(
      () => window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener("test:event", 8),
      /unexpected cleanup failure/,
    );
  } finally {
    if (previousWindow) {
      Object.defineProperty(globalThis, "window", previousWindow);
    } else {
      delete (globalThis as { window?: unknown }).window;
    }
  }
});
