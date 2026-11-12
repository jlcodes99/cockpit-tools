import assert from "node:assert/strict";
import test from "node:test";
import { loadHookModule, settlePromises } from "../../../tests/helpers/reactHookHarness";

type Element = { type: unknown; props: Record<string, any> };
function elements(value: unknown): Element[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  if (!value || typeof value !== "object" || !("props" in value)) return [];
  const element = value as Element;
  return [element, ...elements(element.props.children)];
}

test("history cleanup requires a fresh preview and uses its exact range after confirmation", async () => {
  const scope = { accountId: "a", identity: "identity", from: 0, to: 60_000 };
  const requests: { command: string; args: any }[] = [];
  const settings = {
    enabled: true, intervalMinutes: 1, retentionDays: 90, autoCleanup: true,
    cleanupEveryDays: 1, accountIds: null, onlyChanges: false,
  };
  const store = { accounts: [{ id: "a" }], accountsLoaded: true };
  const h = loadHookModule(new URL("./CodexQuotaHistorySettings.tsx", import.meta.url), {
    "react-i18next": { useTranslation: () => ({ t: (key: string) => key, i18n: { language: "en" } }) },
    "@tauri-apps/api/core": { invoke: async (command: string, args: unknown) => {
      requests.push({ command, args });
      if (command === "codex_quota_history_settings") return settings;
      if (command === "codex_quota_history_stats") return { count: 12, bytes: 1024 };
      if (command === "codex_quota_history_cleanup_preview") return { count: 4, range: scope };
      if (command === "codex_quota_history_cleanup") return 4;
      throw new Error(command);
    } },
    "../../stores/useCodexAccountStore": { useCodexAccountStore: Object.assign(
      (selector: (state: typeof store) => unknown) => selector(store), { getState: () => store },
    ) },
    "../../types/codex": { isCodexApiKeyAccount: () => false },
    "../../utils/codexAccountDisplayName": { resolveCodexAccountDisplayName: () => "Account A" },
    "../../utils/privacy": { isPrivacyModeEnabledByDefault: () => false, maskSensitiveValue: (value: string) => value },
    "../../utils/tauriEventListener": { subscribeTauriEvent: () => () => {} },
    "../SingleSelectDropdown": { SingleSelectDropdown: "dropdown" },
  });
  h.render(() => {
    const wrapper = h.exports.CodexQuotaHistorySettings();
    const body = wrapper.props.children[1];
    return body ? body.type(body.props) : wrapper;
  });
  const byLabel = (label: string) => elements(h.flush()).find((node) =>
    node.props.children === `codex.quotaHistory.storage.${label}` ||
    node.props["ariaLabel"] === `codex.quotaHistory.storage.${label}`);
  elements(h.flush()).find((node) => node.props["aria-expanded"] === false)!.props.onClick();
  await settlePromises();
  byLabel("cleanupAccounts")!.props.onChange("a");
  byLabel("preview")!.props.onClick();
  await settlePromises();
  assert.ok(byLabel("confirmCleanup"));
  assert.equal(requests.some(({ command }) => command === "codex_quota_history_cleanup"), false);

  byLabel("cleanupRange")!.props.onChange("all");
  assert.equal(byLabel("confirmCleanup"), undefined);
  byLabel("preview")!.props.onClick();
  await settlePromises();
  const preview = requests[requests.length - 1];
  assert.equal(preview.command, "codex_quota_history_cleanup_preview");
  assert.equal(preview.args.accountId, "a");
  assert.equal(preview.args.selection.kind, "all");
  byLabel("confirmCleanup")!.props.onClick();
  await settlePromises();
  const deletion = requests.find(({ command }) => command === "codex_quota_history_cleanup")!;
  assert.equal(deletion.args.range, scope);
  assert.ok(byLabel("deleted"));
  h.unmount();
});
