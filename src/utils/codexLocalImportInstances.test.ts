import assert from "node:assert/strict";
import test from "node:test";
import { buildCodexLocalImportInstanceOptions } from "./codexLocalImportInstances.ts";
import type { InstanceProfile } from "../types/instance.ts";

function instance(partial: Partial<InstanceProfile>): InstanceProfile {
  return {
    id: partial.id || "instance-1",
    name: partial.name || "工作账号",
    userDataDir: partial.userDataDir || "/tmp/codex-instances/work",
    extraArgs: "",
    createdAt: 0,
    running: false,
    ...partial,
  } as InstanceProfile;
}

test("默认实例排在候选列表最前", () => {
  const options = buildCodexLocalImportInstanceOptions([
    instance({ id: "__default__", name: "", isDefault: true }),
    instance({ id: "inst-b", name: "B 实例" }),
    instance({ id: "inst-a", name: "A 实例" }),
  ]);

  assert.deepEqual(
    options.map((option) => option.id),
    ["__default__", "inst-a", "inst-b"],
  );
  assert.equal(options[0].isDefault, true);
});

test("候选列表保留 profile 目录与运行态，供界面区分同名实例", () => {
  const options = buildCodexLocalImportInstanceOptions([
    instance({
      id: "inst-a",
      name: "工作账号",
      userDataDir: "/tmp/codex-instances/work-a",
      running: true,
    }),
  ]);

  assert.equal(options.length, 1);
  assert.equal(options[0].userDataDir, "/tmp/codex-instances/work-a");
  assert.equal(options[0].running, true);
});

test("缺少 id 的脏数据会被忽略", () => {
  const options = buildCodexLocalImportInstanceOptions([
    instance({ id: "" }),
    instance({ id: "inst-a", name: "A 实例" }),
  ]);

  assert.deepEqual(
    options.map((option) => option.id),
    ["inst-a"],
  );
});
