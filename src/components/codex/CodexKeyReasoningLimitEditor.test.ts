import assert from "node:assert/strict";
import test from "node:test";
import { loadHookModule } from "../../../tests/helpers/reactHookHarness";

test("the Key editor sends a selected ceiling and explicitly clears it on Unlimited", async () => {
  const calls: Array<[string, string | null]> = [];
  const service = loadHookModule(new URL("../../services/codexLocalAccessService.ts", import.meta.url), {
    "@tauri-apps/api/core": {
      invoke: async (_command: string, args: { apiKeyId: string; maxReasoningEffort: string | null }) => {
        calls.push([args.apiKeyId, args.maxReasoningEffort]);
      },
    },
    "../stores/useCodexPathRecoveryStore": {},
  }).exports;
  const Dropdown = () => null;
  const editor = loadHookModule(new URL("./CodexKeyReasoningLimitEditor.tsx", import.meta.url), {
    "../SingleSelectDropdown": { SingleSelectDropdown: Dropdown },
  });
  const actions: Promise<unknown>[] = [];
  const view = editor.render(() => editor.exports.CodexKeyReasoningLimitEditor({
    value: "", disabled: false, t: (key: string) => key,
    onChange: (maxReasoningEffort: string) => {
      actions.push(service.updateCodexLocalAccessApiKey("shared", { maxReasoningEffort }));
    },
  }));
  const select = view.props.children[1];
  assert.equal(select.type, Dropdown);
  assert.equal(select.props.value, "unlimited");
  assert.ok(select.props.options.some((option: { value: string }) => option.value === "medium"));
  select.props.onChange("medium");
  select.props.onChange("unlimited");
  actions.push(service.updateCodexLocalAccessApiKey("shared", { label: "Updated" }));
  await Promise.all(actions);
  assert.deepEqual(calls, [["shared", "medium"], ["shared", ""], ["shared", null]]);
});
