import assert from "node:assert/strict";
import test from "node:test";
import { loadHookModule } from "../../../tests/helpers/reactHookHarness";
import * as rules from "../../utils/codexApiKeyModelRules";

test("the allowlist picker offers uncovered catalog models and applies only the chosen draft change", () => {
  const Dropdown = () => null;
  const editor = loadHookModule(new URL("./CodexKeyModelAllowlistEditor.tsx", import.meta.url), {
    "../SingleSelectDropdown": { SingleSelectDropdown: Dropdown },
    "../../utils/codexApiKeyModelRules": rules,
  });
  let value = "gpt-6-astra, Vendor/*-vision\ngpt-6-astra";
  const modelIds = ["gpt-6-astra", "gpt-6-sol", "gpt-6-luna", "vendor/fast-vision", "vendor/plain", "gpt-6-astra(high)"];
  editor.render(() => editor.exports.CodexKeyModelAllowlistEditor({
    value, modelIds, disabled: false, t: (key: string) => key,
    onChange: (next: string) => { value = next; },
  }));
  const select = () => editor.flush().props.children[1];
  assert.equal(select().type, Dropdown);
  assert.deepEqual(Array.from(select().props.options, (option: { value: string }) => option.value),
    ["all", "model:gpt-6-sol", "model:gpt-6-luna", "model:vendor/plain"]);
  select().props.onChange("model:gpt-6-sol");
  assert.deepEqual(rules.parseModelRuleText(value), ["gpt-6-astra", "Vendor/*-vision", "gpt-6-sol"]);
  select().props.onChange("all");
  assert.equal(value, "");
  assert.equal(select(), false);
  editor.unmount();
});
