import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import postcss from "postcss";
import type { ComponentProps } from "react";
import type { CodexAccountPoolHealthModal } from "./CodexAccountPoolHealthModal";
import type { CodexAccount } from "../types/codex";
import type {
  CodexLocalAccessAccountHealth,
  CodexLocalAccessAccountPoolHealth,
  CodexLocalAccessAccountPoolMemberHealth,
} from "../types/codexLocalAccess";
import { isBlockingCodexAccountQuotaError } from "../utils/codexQuotaError";
import { resolveCodexHealthIssueDisplayName } from "../utils/codexAccountDisplayName";
import en from "../locales/en.json";
import * as poolDiagnostic from "../utils/codexAccountPoolDiagnostic";
import type { CodexApiKeyInspectionRequest } from "../utils/codexApiKeyInspection";
const { codexAccountPoolDiagnosticReason } = poolDiagnostic;

type Props = ComponentProps<typeof CodexAccountPoolHealthModal>;
type Element = { type: unknown; props: Record<string, any> };
const labels = en.codex.localAccess.accountPoolHealth.dialog;
const compiled = ts.transpileModule(
  readFileSync(new URL("./CodexAccountPoolHealthModal.tsx", import.meta.url), "utf8"),
  { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.ReactJSX } },
).outputText;

function account(id: string, name = id): CodexAccount {
  return {
    id, email: id, account_name: name, auth_mode: "apikey",
    api_model_catalog: ["deepseek-flash"], tokens: { id_token: "", access_token: "" },
    created_at: 0, last_used: 0,
  };
}
function health(accountId: string, patch: Partial<CodexLocalAccessAccountHealth> = {}): CodexLocalAccessAccountHealth {
  return {
    accountId, email: accountId, available: false, consecutiveFailures: 1,
    lastSuccessAt: null, lastFailureAt: 1, lastFailureStatus: 503,
    lastFailureCategory: null, lastFailureMessage: null,
    imageGenerationStatus: "unknown", imageGenerationCheckedAt: null,
    schedulerAvailable: false, schedulerReason: "transient_upstream", schedulerNextRetryAt: null,
    cooldowns: [], ...patch,
  };
}
function member(accountId: string, reasonCode = "model_cooldown"): CodexLocalAccessAccountPoolMemberHealth {
  return { accountId, accountEmail: accountId, available: false, reasonCode, reasonMessage: reasonCode };
}
function pool(patch: Partial<CodexLocalAccessAccountPoolHealth> = {}): CodexLocalAccessAccountPoolHealth {
  return {
    apiKeyId: "key-1", apiKeyLabel: "Client A", provider: "codex", model: "gpt-6-astra",
    requestKind: "text", errorCode: "auth_not_found", errorMessage: "no auth available",
    diagnosticAvailable: false, candidateAuths: 0, scopedAuths: 0, availableAuths: 0,
    unavailableAuths: 0, modelExcludedAuths: 0, quotaReservedAuths: 0, imagePolicyBlockedAuths: 0,
    accountStatuses: [], lastFailureAt: 1, ...patch,
  };
}

// Run the actual render branches and action handlers, without browser or Tauri I/O.
function render(
  patch: Partial<Props> = {},
  clear: (id: string, timestamp: number) => Promise<boolean> = async () => true,
  copy: (value: string) => Promise<void> = async () => {},
) {
  const recovered: string[][] = [];
  const cleared: Array<[string, number]> = [];
  const events: string[] = [];
  const copied: string[] = [];
  const slots: any[] = [];
  let hookIndex = 0;
  let errorMessage: string | null = null;
  const reauthorized: string[] = [];
  const inspections: CodexApiKeyInspectionRequest[] = [];
  const navigationOrder: string[] = [];
  const exports: Record<string, any> = {};
  const jsx = (type: unknown, props: Element["props"]): Element => ({ type, props });
  vm.runInNewContext(compiled, {
    exports,
    window: { dispatchEvent(event: Event) { events.push(event.type); } },
    Event,
    navigator: { clipboard: { async writeText(value: string) { copied.push(value); await copy(value); } } },
    require(name: string) {
      if (name === "react") return {
        useMemo: (factory: () => unknown) => factory(),
        useRef: (current: unknown) => {
          const index = hookIndex++;
          if (!(index in slots)) slots[index] = { current };
          return slots[index];
        },
        useState: (initial: any) => {
          const index = hookIndex++;
          if (!(index in slots)) slots[index] = typeof initial === "function" ? initial() : initial;
          return [slots[index], (value: any) => {
            slots[index] = typeof value === "function" ? value(slots[index]) : value;
          }];
        },
        useEffect() {},
      };
      if (name === "react/jsx-runtime") return { jsx, jsxs: jsx };
      if (name === "lucide-react") return new Proxy({}, { get: (_, key) => key });
      if (name === "@tauri-apps/api/event") return { listen() { throw new Error("Unexpected Tauri call"); } };
      if (name === "react-i18next") return { useTranslation: () => ({
        t(key: string, options: any) {
          let label: any = key.split(".").reduce((value: any, part) => value?.[part], en);
          label = label ?? (typeof options === "string" ? options : options?.defaultValue) ?? key;
          return label.replace(/{{(\w+)}}/g, (_: string, name: string) => String(options?.[name] ?? ""));
        },
      }) };
      if (name === "../presentation/platformAccountPresentation") return {
        buildCodexAccountPresentation: () => ({ planLabel: "API", planClass: "api", quotaItems: [] }),
      };
      if (name === "../utils/codexAccountPoolDiagnostic") return poolDiagnostic;
      if (name === "../utils/codexApiKeyInspection") return {
        requestCodexApiKeyInspection(request: CodexApiKeyInspectionRequest) {
          navigationOrder.push("inspection");
          inspections.push({ ...request });
        },
      };
      if (name === "../services/codexLocalAccessService") return {
        clearCodexLocalAccessPoolFailure: async (id: string, timestamp: number) => {
          cleared.push([id, timestamp]);
          return clear(id, timestamp);
        },
      };
      if (name === "../utils/codexQuotaError") return { isBlockingCodexAccountQuotaError };
      if (name === "../utils/codexAccountDisplayName") return { resolveCodexHealthIssueDisplayName };
      if (name === "./ModalErrorMessage") return {
        ModalErrorMessage: "modal-error",
        useModalErrorState: () => ({ message: errorMessage, scrollKey: 0, set(value: string | null) { errorMessage = value; } }),
      };
      if (name.endsWith(".css")) return {};
      throw new Error(`Unexpected import: ${name}`);
    },
  });
  let props: Props = {
    isOpen: true, accountIds: ["deepseek"], accounts: [account("deepseek", "DeepSeek")],
    accountHealth: [], accountPoolHealth: [], actionBusy: false,
    onClose() { navigationOrder.push("closed"); },
    onRecover: async (id) => { recovered.push([id]); },
    onRecoverAll: async (ids) => { recovered.push(Array.from(ids)); },
    onReauthorize: (id) => { reauthorized.push(id); },
    ...patch,
  };
  return {
    get tree() { hookIndex = 0; return exports.CodexAccountPoolHealthModal(props) as Element; },
    update(next: Partial<Props>) { props = { ...props, ...next }; },
    recovered, reauthorized, cleared, copied, events, inspections, navigationOrder,
  };
}
function elements(value: any): Element[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  if (!value || typeof value !== "object" || !value.props) return [];
  return [value, ...elements(value.props.children)];
}
function text(value: any): string {
  if (Array.isArray(value)) return value.map(text).join(" ");
  if (typeof value === "string" || typeof value === "number") return String(value);
  return value?.props ? text(value.props.children) : "";
}
function buttons(tree: Element, label: string): Element[] {
  return elements(tree).filter((element) => element.type === "button" && text(element).trim() === label);
}
function rows(tree: Element): Element[] {
  return elements(tree).filter((element) => element.props.className?.split(" ").includes("codex-account-pool-health-item"));
}
function assertNoRecovery(tree: Element) {
  assert.equal(buttons(tree, labels.recover).length, 0);
  assert.equal(buttons(tree, labels.recoverAll).length, 0);
}

test("GPT pool failures do not attribute failures to a DeepSeek-only member", () => {
  const { tree } = render({ accountPoolHealth: [pool(), pool({ model: "gpt-5.6-luna", apiKeyId: "key-2" })] });
  assert.equal(rows(tree).length, 2);
  assert.ok(text(tree).includes("gpt-6-astra"));
  assert.ok(text(tree).includes("gpt-5.6-luna"));
  assert.ok(text(tree).includes("Client A"));
  assert.ok(text(tree).includes(labels.poolUnattributedDetail));
  assert.ok(!text(tree).includes("DeepSeek"));
  assert.ok(!text(tree).includes(labels.noIssues));
  assertNoRecovery(tree);
});

test("legacy or empty member diagnostics stay at pool level even before accounts load", () => {
  for (const accountStatuses of [undefined, [], [member(" ")]]) {
    const { tree } = render({ accountIds: [], accounts: [], accountPoolHealth: [pool({ accountStatuses })] });
    assert.equal(rows(tree).length, 1);
    assert.ok(text(tree).includes(labels.poolUnavailable));
    assertNoRecovery(tree);
  }
});

test("pool context honors masking and does not treat zero unknown counters as model incompatibility", () => {
  const { tree } = render({ accountPoolHealth: [pool()], maskAccountText: () => "masked key" });
  assert.ok(text(tree).includes("masked key"));
  assert.ok(!text(tree).includes("Client A"));
  const diagnosticPrefix = labels.poolDiagnosticDetail.split("{{candidate}}")[0].replace("{{model}}", "gpt-6-astra");
  assert.ok(!text(tree).includes(diagnosticPrefix));
  const diagnostic = render({ accountPoolHealth: [pool({ diagnosticAvailable: true, candidateAuths: 2 })] });
  assert.ok(text(diagnostic.tree).includes(`${diagnosticPrefix}2`));
  assertNoRecovery(diagnostic.tree);
});

test("real account diagnostics remain attributed and recoverable beside an unscoped pool failure", () => {
  const app = render({ accountHealth: [health("deepseek")], accountPoolHealth: [pool()] });
  assert.equal(rows(app.tree).length, 2);
  assert.ok(text(app.tree).includes("DeepSeek"));
  buttons(app.tree, labels.recover)[0].props.onClick();
  assert.deepEqual(app.recovered, [["deepseek"]]);
});

test("reported members use backend attribution even with a client model alias", () => {
  const app = render({ accountPoolHealth: [pool({ accountStatuses: [member("deepseek")] })] });
  assert.equal(rows(app.tree).length, 2);
  assert.ok(text(app.tree).includes("DeepSeek"));
  assert.ok(!text(app.tree).includes(labels.poolUnattributedDetail));
  buttons(app.tree, labels.recover)[0].props.onClick();
  assert.deepEqual(app.recovered, [["deepseek"]]);
});

test("model and policy failures offer no recovery, including bulk recovery via account health", () => {
  for (const reason of ["model_not_supported", "model_not_available", "not_found", "model_excluded", "model_disabled", "quota_reserved", "image_policy_blocked", "disabled", "pool_unavailable"]) {
    assertNoRecovery(render({ accountHealth: [health("deepseek", { schedulerReason: reason })] }).tree);
    const { tree } = render({
      accountHealth: [health("deepseek")],
      accountPoolHealth: [pool({ accountStatuses: [member("deepseek", reason)] })],
    });
    assertNoRecovery(tree);
  }
});

test("bulk recovery includes only visible recoverable members and deduplicates accounts", () => {
  const ids = ["first", "second", "suppressed", "excluded", "reauth"];
  const app = render({
    accountIds: ids, accounts: ids.map((id) => account(id)), accountHealth: ids.map((id) => health(id)),
    recoverySuppressedAccountIds: ["suppressed"],
    accountPoolHealth: [pool({ accountStatuses: [member("first"), member("suppressed"), member("excluded", "model_excluded"), member("reauth", "unauthorized")] }), pool({ accountStatuses: [member("first")] })],
  });
  buttons(app.tree, labels.recoverAll)[0].props.onClick();
  assert.deepEqual(app.recovered[0].sort(), ["first", "second"]);
});

test("suppressed failures leave an empty state without recovery actions", () => {
  const { tree } = render({
    accountHealth: [health("deepseek")], recoverySuppressedAccountIds: ["deepseek"],
    accountPoolHealth: [pool({ accountStatuses: [member("deepseek")] })],
  });
  assert.equal(rows(tree).length, 0);
  assert.ok(text(tree).includes(labels.noIssues));
  assertNoRecovery(tree);
});

test("credential failures retain reauthorization in both account and member diagnostics", () => {
  for (const patch of [
    { accountHealth: [health("deepseek", { schedulerReason: "unauthorized" })] },
    { accountPoolHealth: [pool({ accountStatuses: [member("deepseek", "unauthorized")] })] },
  ]) {
    const app = render(patch);
    assertNoRecovery(app.tree);
    buttons(app.tree, en.common.reauthorize)[0].props.onClick();
    assert.deepEqual(app.reauthorized, ["deepseek"]);
  }
});

test("pool-only errors keep explicit close controls and never close on the overlay", () => {
  let closed = 0;
  const { tree } = render({ accountPoolHealth: [pool()], onClose: () => { closed++; } });
  assert.equal(tree.props.onClick, undefined);
  buttons(tree, en.common.close)[0].props.onClick();
  assert.equal(closed, 1);
});

test("long pool lists keep the existing viewport bound and internally scrollable body", () => {
  const style = postcss.parse(readFileSync(new URL("./CodexAccountPoolHealthModal.css", import.meta.url), "utf8"));
  const declarations = (selector: string) => {
    const result: Record<string, string> = {};
    style.walkRules(selector, (rule) => { rule.walkDecls((decl) => { result[decl.prop] = decl.value; }); });
    return result;
  };
  assert.match(declarations(".modal.codex-account-pool-health-modal")["max-height"], /100vh/);
  const body = declarations(".codex-account-pool-health-body");
  assert.equal(body["overflow-y"], "auto");
  assert.equal(body["min-height"], "0");
});

test("selection-stage reasons distinguish unknown diagnostics, missing candidates and scope mismatch", () => {
  const reason = (patch: Partial<CodexLocalAccessAccountPoolHealth>) =>
    codexAccountPoolDiagnosticReason(pool(patch)).key.split('.').pop();
  assert.equal(reason({ candidateAuths: 3 }), "poolUnknownDetail");
  assert.equal(reason({ diagnosticAvailable: true }), "poolNoCandidatesDetail");
  assert.equal(reason({ diagnosticAvailable: true, candidateAuths: 1 }), "poolScopeMismatchDetail");
  assert.equal(reason({ diagnosticAvailable: true, candidateAuths: 1, scopedAuths: 1 }), "poolUnknownDetail");
  for (const counter of ["unavailableAuths", "modelExcludedAuths", "quotaReservedAuths", "imagePolicyBlockedAuths"] as const) {
    const result = codexAccountPoolDiagnosticReason(pool({ diagnosticAvailable: true, candidateAuths: 2, scopedAuths: 2, [counter]: 1 }));
    assert.ok(result.key.endsWith("poolBlockedDetail"));
    assert.equal(Object.values(result.values).filter((count) => count === 1).length, 1);
  }
  const scoped = render({ accountPoolHealth: [pool({ diagnosticAvailable: true, candidateAuths: 1 })] });
  assert.ok(text(scoped.tree).includes(labels.poolScopeMismatchDetail));
  assert.ok(!text(scoped.tree).includes(labels.authDetail));
});

const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

test("clearing removes only the exact pool record, preserves account health and allows newer failures", async () => {
  const app = render({ accountHealth: [health("deepseek")], accountPoolHealth: [pool(), pool({ apiKeyId: "key-2" })] });
  buttons(app.tree, en.common.clearRecord)[0].props.onClick();
  await settle();
  assert.deepEqual(app.cleared, [["key-1", 1]]);
  assert.deepEqual(app.events, ["codex-local-access-state-updated"]);
  assert.equal(buttons(app.tree, en.common.clearRecord).length, 1);
  assert.ok(text(app.tree).includes("DeepSeek"));
  assert.ok(text(app.tree).includes(labels.clearSuccess));
  app.update({ accountPoolHealth: [pool({ lastFailureAt: 2 })] });
  assert.equal(buttons(app.tree, en.common.clearRecord).length, 1);
});

test("pool record can also be cleared when member diagnostics exist", async () => {
  const app = render({ accountPoolHealth: [pool({ accountStatuses: [member("deepseek")] })] });
  buttons(app.tree, en.common.clearRecord)[0].props.onClick();
  await settle();
  assert.equal(rows(app.tree).length, 0);
  assert.ok(text(app.tree).includes(labels.clearSuccess));
  assert.ok(text(app.tree).includes(labels.noIssues));
  assert.equal(app.recovered.length, 0);
});

test("duplicate clear clicks submit once, keep the modal usable and show busy state", async () => {
  let finish!: (value: boolean) => void;
  const app = render({ accountPoolHealth: [pool()] }, () => new Promise((resolve) => { finish = resolve; }));
  const button = buttons(app.tree, en.common.clearRecord)[0];
  button.props.onClick();
  button.props.onClick();
  assert.equal(app.cleared.length, 1);
  assert.equal(buttons(app.tree, en.common.clearingRecord)[0].props.disabled, true);
  assert.equal(buttons(app.tree, en.common.close)[0].props.disabled, undefined);
  finish(true);
  await settle();
  assert.equal(rows(app.tree).length, 0);
});

test("clear failures remain inside the open modal, retain the record and clear stale errors on retry", async () => {
  let attempt = 0;
  const app = render({ accountPoolHealth: [pool()] }, async () => {
    attempt++;
    if (attempt === 1) throw new Error("pool_diagnostic_clear_timeout");
    return attempt !== 2;
  });
  const message = () => elements(app.tree).find((element) => element.type === "modal-error")?.props.message;
  buttons(app.tree, en.common.clearRecord)[0].props.onClick();
  await settle();
  assert.equal(message(), labels.clearFailed.replace("{{error}}", "pool_diagnostic_clear_timeout"));
  assert.equal(rows(app.tree).length, 1);
  buttons(app.tree, en.common.clearRecord)[0].props.onClick();
  assert.equal(message(), null);
  await settle();
  assert.equal(message(), labels.clearStale);
  assert.equal(rows(app.tree).length, 1);
  buttons(app.tree, en.common.clearRecord)[0].props.onClick();
  await settle();
  assert.equal(message(), null);
  assert.equal(rows(app.tree).length, 0);
});

test("scope diagnostics describe each precise selection reason without account recovery", () => {
  const reasons = ["scope_mismatch", "account_mapping_missing", "bound_account_not_loaded", "bound_account_not_candidate"];
  const app = render({
    accountIds: [],
    accountPoolHealth: [pool({
      diagnosticAvailable: true, candidateAuths: 1,
      scopeDiagnostics: reasons.map((reasonCode) => ({ accountId: "deepseek", accountEmail: "fallback@example.com", reasonCode })),
    })],
  });
  for (const reason of reasons) assert.ok(text(app.tree).includes(
    labels[poolDiagnostic.codexAccountPoolScopeReasonKey(reason).split('.').pop() as keyof typeof labels],
  ));
  assert.ok(text(app.tree).includes("DeepSeek"));
  assert.ok(text(app.tree).includes(labels.unmappedAccount));
  assert.ok(!text(app.tree).includes("fallback@example.com"));
  assert.ok(!text(app.tree).includes(labels.poolUnattributedDetail));
  assertNoRecovery(app.tree);
  assert.equal(buttons(app.tree, en.common.reauthorize).length, 0);
});

test("scope account names honor masking and never expose unknown credential IDs", () => {
  const diagnostics = [
    { accountId: "deepseek", accountEmail: "other@example.com", reasonCode: "scope_mismatch" },
    { accountId: "secret-credential-id", accountEmail: "", reasonCode: "bound_account_not_loaded" },
    { accountId: "unmapped-secret-id", accountEmail: "secret@example.com", reasonCode: "account_mapping_missing" },
    { accountId: "unknown", accountEmail: "bound@example.com", reasonCode: "bound_account_not_candidate" },
  ];
  assert.equal(poolDiagnostic.codexAccountPoolScopeDisplayName(diagnostics[0], [account("deepseek", "Name")], "Unknown"), "Name");
  assert.equal(poolDiagnostic.codexAccountPoolScopeDisplayName(diagnostics[3], [], "Unknown"), "bound@example.com");
  const { tree } = render({ accountIds: [], accountPoolHealth: [pool({ scopeDiagnostics: diagnostics })], maskAccountText: () => "masked" });
  assert.ok(text(tree).includes("masked"));
  for (const secret of ["DeepSeek", "other@example.com", "secret-credential-id", "unmapped-secret-id", "secret@example.com", "bound@example.com"]) {
    assert.ok(!text(tree).includes(secret), secret);
  }
  assertNoRecovery(tree);
});

test("known unloaded bound account IDs remain identifiable without claiming deletion", () => {
  const diagnostic = { accountId: "bound-business-id", accountEmail: "", reasonCode: "bound_account_not_loaded" };
  const app = render({ accountIds: [], accounts: [], accountPoolHealth: [pool({ scopeDiagnostics: [diagnostic] })] });
  assert.ok(text(app.tree).includes(labels.accountReference.replace("{{id}}", "bound-business-id")));
  assert.ok(text(app.tree).includes(labels.boundAccountNotLoadedDetail));
  assert.ok(!text(app.tree).includes(labels.missingDetail));
  assertNoRecovery(app.tree);
  app.update({ maskAccountText: () => "masked bound account" });
  assert.ok(!text(app.tree).includes("bound-business-id"));
  assert.ok(text(app.tree).includes("masked bound account"));
  app.update({ maskAccountText: undefined, accountPoolHealth: [pool({ scopeDiagnostics: [{ ...diagnostic, reasonCode: "account_mapping_missing" }] })] });
  assert.ok(!text(app.tree).includes("bound-business-id"));
  assert.ok(text(app.tree).includes(labels.unmappedAccount));
});

test("binding inspection closes the modal first and preserves internal key and request context", () => {
  const app = render({ accountIds: [], accountPoolHealth: [pool({
    apiKeyId: "__internal-instance-key__", requestKind: "image_edit",
    scopeDiagnostics: [{ accountId: "", accountEmail: "", reasonCode: "account_mapping_missing" }],
  })] });
  buttons(app.tree, labels.inspectBinding)[0].props.onClick();
  assert.deepEqual(app.navigationOrder, ["closed", "inspection"]);
  assert.deepEqual(app.inspections, [{ apiKeyId: "__internal-instance-key__", requestKind: "image_edit" }]);
  assert.equal(app.cleared.length, 0);
  assert.equal(app.recovered.length, 0);
  assert.equal(app.reauthorized.length, 0);
});

test("legacy unknown and missing candidate diagnostics offer configuration inspection", () => {
  for (const diagnosticAvailable of [false, true]) {
    const app = render({ accountIds: [], accountPoolHealth: [pool({ diagnosticAvailable })] });
    buttons(app.tree, labels.inspectConfig)[0].props.onClick();
    assert.deepEqual(app.inspections, [{ apiKeyId: "key-1", requestKind: "text" }]);
    assertNoRecovery(app.tree);
  }
  assert.ok(poolDiagnostic.codexAccountPoolScopeReasonKey("unexpected_diagnostic").endsWith("scopeUnknownDetail"));
});

test("request diagnostics expose a timestamp and copy only the request ID", async () => {
  const app = render({ accountPoolHealth: [pool({ requestId: " request-from-log ", lastFailureAt: 1_700_000_000_000 })] });
  assert.ok(text(app.tree).includes(en.common.requestId));
  assert.ok(text(app.tree).includes(en.common.failureTime));
  assert.ok(text(app.tree).includes("request-from-log"));
  assert.equal(elements(app.tree).find((element) => element.type === "time")?.props.dateTime, "2023-11-14T22:13:20.000Z");
  await buttons(app.tree, en.common.copy)[0].props.onClick();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(app.copied, ["request-from-log"]);
  assert.ok(text(app.tree).includes(en.common.copied));
  assert.equal(app.cleared.length, 0);
  assert.equal(app.recovered.length, 0);
});

test("copy errors remain inside the diagnostic dialog and legacy IDs are optional", async () => {
  const app = render({ accountPoolHealth: [pool({ requestId: "request-1" })] }, undefined, async () => {
    throw new Error("clipboard denied");
  });
  buttons(app.tree, en.common.copy)[0].props.onClick();
  await new Promise((resolve) => setImmediate(resolve));
  assert.ok(elements(app.tree).find((element) => element.type === "modal-error")?.props.message.includes("clipboard denied"));
  assert.deepEqual(app.navigationOrder, []);
  app.update({ accountPoolHealth: [pool()] });
  assert.equal(buttons(app.tree, en.common.copy).length, 0);
});
