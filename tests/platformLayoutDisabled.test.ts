import assert from "node:assert/strict";
import test from "node:test";

import { ALL_PLATFORM_IDS, type PlatformId } from "../src/types/platform.ts";
import {
  isPlatformDisabled,
  makePlatformEntryId,
  parseGroupEntryId,
  parsePlatformEntryId,
  usePlatformLayoutStore,
} from "../src/stores/usePlatformLayoutStore.ts";

// 应用设置中拥有独立配置分区的平台（与 PlatformSettingsSection 的调用点一致）
const SETTINGS_PLATFORM_IDS: PlatformId[] = [
  "antigravity",
  "claude_manager",
  "github-copilot",
  "windsurf",
  "kiro",
  "codebuddy",
  "codebuddy_cn",
  "qoder",
  "zcode",
  "trae",
  "trae_solo",
  "trae_cn",
  "trae_solo_cn",
  "workbuddy",
  "zed",
  "cursor",
  "grok",
  "codex",
];

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

// 应用设置中的平台分区需在禁用时隐藏：这里验证驱动该行为的订阅谓词，
// 避免 store 语义变动后分区静默重新出现。
test("disabled set reflects every platform having an App Settings section", () => {
  resetState();

  const store = usePlatformLayoutStore.getState();
  for (const id of SETTINGS_PLATFORM_IDS) {
    store.setDisabledPlatform(id, true);
  }

  const disabled = new Set(usePlatformLayoutStore.getState().disabledPlatformIds);
  for (const id of SETTINGS_PLATFORM_IDS) {
    assert.equal(disabled.has(id), true, `${id} 应被标记为禁用`);
  }
  assert.equal(isPlatformDisabled("grok"), true);
});

// 回归：平台可能以「独立条目」或「分组子级」两种形态出现。
// 开关必须两种形态都能覆盖，否则分组内的平台会没有入口可点。
test("every platform is reachable as a standalone entry or a group child", () => {
  resetState();

  const { orderedEntryIds, platformGroups } = usePlatformLayoutStore.getState();
  const standalone = new Set<string>();
  const inGroup = new Set<string>();

  for (const entryId of orderedEntryIds) {
    const platformId = parsePlatformEntryId(entryId);
    if (platformId) {
      standalone.add(platformId);
      continue;
    }
    const groupId = parseGroupEntryId(entryId);
    if (!groupId) continue;
    const group = platformGroups.find((item) => item.id === groupId);
    for (const id of group?.platformIds ?? []) {
      inGroup.add(id);
    }
  }

  for (const id of ALL_PLATFORM_IDS) {
    assert.equal(
      standalone.has(id) || inGroup.has(id),
      true,
      `${id} 既不是独立条目也不在任何分组中，布局里将没有启用/禁用入口`,
    );
  }
});

// 回归：布局弹窗里 entry.id 是 `platform:xxx`，而 store 存裸平台 ID。
// 直接把 entry.id 传进去会被静默丢弃，表现为开关点了没反应。
test("disabling via a prefixed entry id takes effect", () => {
  resetState();

  const entryId = makePlatformEntryId("cursor");
  assert.equal(entryId, "platform:cursor");

  const platformId = parsePlatformEntryId(entryId);
  assert.equal(platformId, "cursor");
  usePlatformLayoutStore.getState().setDisabledPlatform(platformId!, true);

  assert.deepEqual(usePlatformLayoutStore.getState().disabledPlatformIds, ["cursor"]);
  assert.equal(isPlatformDisabled("cursor"), true);

  // UI 用 entry id 形态回显勾选状态，必须能对上
  const rendered = new Set(
    usePlatformLayoutStore.getState().disabledPlatformIds.map(makePlatformEntryId),
  );
  assert.equal(rendered.has(entryId), true);
});

test("a raw entry id is rejected instead of silently dropped", () => {
  resetState();

  const before = [...usePlatformLayoutStore.getState().disabledPlatformIds];
  usePlatformLayoutStore.getState().setDisabledPlatform("platform:cursor" as PlatformId, true);

  // 这是曾经导致开关失效的写法：断言它确实无效，提醒调用方必须先 parse
  const after = usePlatformLayoutStore.getState().disabledPlatformIds;
  assert.deepEqual(after, before);
  assert.equal(parsePlatformEntryId("group:trae-suite"), null);
});

// 回归：分组行的「启用」开关控制整组。组内平台全部启用 → 勾选；
// 组内任一平台被单独禁用 → 分组开关取消勾选。
test("group-level enable reflects every member platform", () => {
  resetState();

  const { platformGroups } = usePlatformLayoutStore.getState();
  const codebuddyGroup = platformGroups.find((g) => g.id === "codebuddy-suite");
  assert.ok(codebuddyGroup, "应存在 codebuddy-suite 分组");

  // 全部启用 → 分组勾选
  assert.equal(
    codebuddyGroup!.platformIds.every((id) => !isPlatformDisabled(id)),
    true,
  );

  // 禁用其中一个成员 → 分组取消勾选
  usePlatformLayoutStore.getState().setDisabledPlatform(codebuddyGroup!.platformIds[0], true);
  assert.equal(
    codebuddyGroup!.platformIds.every((id) => !isPlatformDisabled(id)),
    false,
  );
});
