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
function render(patch: Partial<Props> = {}) {
  const recovered: string[][] = [];
  const reauthorized: string[] = [];
  const exports: Record<string, any> = {};
  const jsx = (type: unknown, props: Element["props"]): Element => ({ type, props });
  vm.runInNewContext(compiled, {
    exports,
    require(name: string) {
      if (name === "react") return {
        useMemo: (factory: () => unknown) => factory(), useRef: (current: unknown) => ({ current }),
        useState: (initial: any) => [typeof initial === "function" ? initial() : initial, () => {}],
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
      if (name === "../utils/codexQuotaError") return { isBlockingCodexAccountQuotaError };
      if (name === "../utils/codexAccountDisplayName") return { resolveCodexHealthIssueDisplayName };
      if (name === "./ModalErrorMessage") return {
        ModalErrorMessage: "modal-error",
        useModalErrorState: () => ({ message: null, scrollKey: 0, set() {} }),
      };
      if (name.endsWith(".css")) return {};
      throw new Error(`Unexpected import: ${name}`);
    },
  });
  const props: Props = {
    isOpen: true, accountIds: ["deepseek"], accounts: [account("deepseek", "DeepSeek")],
    accountHealth: [], accountPoolHealth: [], actionBusy: false,
    onClose() {},
    onRecover: async (id) => { recovered.push([id]); },
    onRecoverAll: async (ids) => { recovered.push(Array.from(ids)); },
    onReauthorize: (id) => { reauthorized.push(id); },
    ...patch,
  };
  return { tree: exports.CodexAccountPoolHealthModal(props) as Element, recovered, reauthorized };
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
  assert.equal(rows(app.tree).length, 1);
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
