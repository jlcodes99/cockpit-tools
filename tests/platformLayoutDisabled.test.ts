import assert from "node:assert/strict";
import test from "node:test";

import { isPlatformDisabled, usePlatformLayoutStore } from "../src/stores/usePlatformLayoutStore.ts";

function resetState() {
  usePlatformLayoutStore.setState({
    hiddenPlatformIds: [],
    disabledPlatformIds: [],
  } as never);
}

test("hidden and disabled are independent", () => {
  resetState();

  // 只隐藏：不停止自动活动
  usePlatformLayoutStore.getState().setHiddenPlatform("codex", true);
  assert.equal(isPlatformDisabled("codex"), false, "隐藏不应等同于禁用");

  // 再禁用：此时才停止自动活动
  usePlatformLayoutStore.getState().setDisabledPlatform("codex", true);
  assert.equal(isPlatformDisabled("codex"), true);
});

test("disabling does not hide the platform entry", () => {
  resetState();

  usePlatformLayoutStore.getState().setDisabledPlatform("cursor", true);

  const state = usePlatformLayoutStore.getState();
  assert.equal(isPlatformDisabled("cursor"), true);
  assert.equal(
    state.hiddenPlatformIds.includes("cursor"),
    false,
    "禁用不应把平台从布局中隐藏",
  );
});

test("toggling disabled twice restores the enabled state", () => {
  resetState();

  const store = usePlatformLayoutStore.getState();
  store.toggleDisabledPlatform("grok");
  assert.equal(isPlatformDisabled("grok"), true);

  usePlatformLayoutStore.getState().toggleDisabledPlatform("grok");
  assert.equal(isPlatformDisabled("grok"), false);
});

test("re-enabling a disabled platform only clears that platform", () => {
  resetState();

  const store = usePlatformLayoutStore.getState();
  store.setDisabledPlatform("codex", true);
  usePlatformLayoutStore.getState().setDisabledPlatform("cursor", true);
  usePlatformLayoutStore.getState().setDisabledPlatform("codex", false);

  assert.equal(isPlatformDisabled("codex"), false);
  assert.equal(isPlatformDisabled("cursor"), true);
});
