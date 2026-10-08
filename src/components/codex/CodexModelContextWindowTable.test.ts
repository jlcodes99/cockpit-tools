import assert from "node:assert/strict";
import test from "node:test";
import { loadHookModule } from "../../../tests/helpers/reactHookHarness.ts";
import { resolveProviderModelVisionState } from "../../utils/codexModelProviderVision.ts";

function elements(value: any): any[] {
  if (!value || typeof value !== "object") return [];
  if (Array.isArray(value)) return value.flatMap(elements);
  return [value, ...elements(value.props?.children)];
}

test("vision rows render saved mixed-case overrides and inherited defaults", () => {
  const h = loadHookModule(new URL("./CodexModelContextWindowTable.tsx", import.meta.url), {
    "react-i18next": { useTranslation: () => ({ t: (key: string) => key }) },
    "../SingleSelectDropdown": { SingleSelectDropdown: () => null },
    "../../utils/codexModelProviderVision": { resolveProviderModelVisionState },
  });
  const changes: [string, boolean][] = [];
  const tree = h.render(() => h.exports.CodexModelContextWindowTable({
    models: ["MiniMax-M3", "Qwen-VL", "Other-Model"],
    drafts: {},
    onChange: () => {},
    showContextWindow: false,
    visionStates: { "minimax-m3": true, "qwen-vl": false },
    visionDefault: true,
    onVisionChange: (model: string, enabled: boolean) => changes.push([model, enabled]),
  }));
  const checkboxes = elements(tree).filter((node) => node.type === "input" && node.props.type === "checkbox");
  assert.equal(checkboxes.length, 3);
  assert.equal(checkboxes[0].props.checked, true);
  assert.equal(checkboxes[1].props.checked, false);
  assert.equal(checkboxes[2].props.checked, true);
  checkboxes[0].props.onChange({ target: { checked: false } });
  assert.deepEqual(changes, [["minimax-m3", false]]);
});
