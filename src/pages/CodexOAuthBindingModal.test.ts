import assert from "node:assert/strict";
import test from "node:test";
import { loadHookModule } from "../../tests/helpers/reactHookHarness";
import { resolveOAuthBindingQuotaReserve } from "./codexAccountsControllerModel";

function nodes(value: any): any[] {
  if (!value || typeof value !== "object") return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}

function text(value: any): string {
  if (value == null || typeof value === "boolean") return "";
  if (Array.isArray(value)) return value.map(text).join("");
  return typeof value === "object" ? text(value.props?.children) : String(value);
}

test("new OAuth bindings start with both reserves at 10%", () => {
  for (const collection of [undefined, null, {}, { boundOauthAccountId: null, boundOauthQuotaReserve: null }]) {
    assert.deepEqual(resolveOAuthBindingQuotaReserve(collection), {
      hourlyPercent: 10, weeklyPercent: 10,
    });
  }
});

test("opening an existing binding preserves custom reserves and a saved opt-out", () => {
  assert.deepEqual(resolveOAuthBindingQuotaReserve({
    boundOauthAccountId: "oauth-1",
    boundOauthQuotaReserve: { hourlyPercent: 15, weeklyPercent: 25 },
  }), { hourlyPercent: 15, weeklyPercent: 25 });
  assert.equal(resolveOAuthBindingQuotaReserve({
    boundOauthAccountId: "oauth-1", boundOauthQuotaReserve: null,
  }), null);
  assert.equal(resolveOAuthBindingQuotaReserve({ boundOauthAccountId: "oauth-1" }), null);
});

test("invalid saved reserves are not displayed as an enabled valid policy", () => {
  for (const hourlyPercent of [0, 101, 10.5, NaN]) {
    assert.equal(resolveOAuthBindingQuotaReserve({
      boundOauthAccountId: "oauth-1",
      boundOauthQuotaReserve: { hourlyPercent, weeklyPercent: 10 },
    }), null);
  }
  assert.equal(resolveOAuthBindingQuotaReserve({
    boundOauthAccountId: "oauth-1",
    boundOauthQuotaReserve: { hourlyPercent: 10, weeklyPercent: 0 },
  }), null);
});

function modalHarness(focusCalls: any[] = [], escapeCalls: any[] = []) {
  const imports: Record<string, unknown> = {};
  imports["../hooks/useModalFocusTrap"] = {
    useModalFocusTrap: (ref: any, open: boolean, restore = true, active = true) =>
      focusCalls.push({ ref, open, restore, active }),
  };
  imports["../hooks/useEscClose"] = {
    useEscCloseTopmost: (open: boolean, close: () => void) => escapeCalls.push({ open, close }),
  };
  for (const component of ["AccountTagFilterDropdown", "ModalErrorMessage", "MultiSelectFilterDropdown", "PaginationControls", "SingleSelectFilterDropdown"]) {
    imports[`../components/${component}`] = { [component]: component };
  }
  return loadHookModule(new URL("./CodexOAuthBindingModal.tsx", import.meta.url), imports);
}

test("binding dialog shows saved percentages directly and opens the shared editor", () => {
  const h = modalHarness();
  let editCount = 0;
  const props: any = {
    t: (key: string, fallback: any) => typeof fallback === "string" ? fallback : fallback?.defaultValue ?? key,
    oauthBindingTargetActive: true,
    oauthBindingTargetKind: "local_access",
    isLocalAccessOAuthBinding: true,
    oauthAccounts: [],
    oauthBindingQuotaReserve: { hourlyPercent: 15, weeklyPercent: 25 },
    oauthBindingSaving: false,
    openOAuthBindingQuotaReserveEditor: () => { editCount++; },
  };
  h.render(() => h.exports.CodexOAuthBindingModal(props));
  let tree = nodes(h.flush());
  const summary = tree.find((node) => node.props?.className?.includes("codex-oauth-binding-quota-summary"));
  assert.ok(summary);
  assert.match(text(summary), /5小时 15%.*周 25%/);
  assert.equal(tree.find((node) => node.props?.type === "checkbox").props.checked, true);
  summary.props.onClick();
  assert.equal(editCount, 1);
  props.oauthBindingSaving = true;
  tree = nodes(h.flush());
  assert.equal(tree.find((node) => node.props?.className?.includes("codex-oauth-binding-quota-summary")).props.disabled, true);
  props.oauthBindingQuotaReserve = null;
  tree = nodes(h.flush());
  assert.equal(tree.find((node) => node.props?.type === "checkbox").props.checked, false);
  assert.equal(tree.some((node) => node.props?.className?.includes("codex-oauth-binding-quota-summary")), false);
  props.isLocalAccessOAuthBinding = false;
  props.oauthBindingTargetKind = "api_key_account";
  props.oauthBindingQuotaReserve = { hourlyPercent: 10, weeklyPercent: 10 };
  tree = nodes(h.flush());
  assert.equal(tree.some((node) => node.props?.type === "checkbox"), false);
  h.unmount();
});

test("binding and reserve dialogs keep focus and Escape with only the active layer", () => {
  const focusCalls: any[] = [];
  const escapeCalls: any[] = [];
  const h = modalHarness(focusCalls, escapeCalls);
  let parentCloses = 0;
  let childCloses = 0;
  const props: any = {
    t: (key: string, fallback: any) => typeof fallback === "string" ? fallback : key,
    oauthBindingTargetActive: true,
    isLocalAccessOAuthBinding: true,
    oauthAccounts: [],
    oauthBindingQuotaReserve: { hourlyPercent: 10, weeklyPercent: 10 },
    oauthBindingQuotaReserveFieldErrors: {},
    closeOAuthBindingModal: () => { parentCloses++; },
    closeOAuthBindingQuotaReserveEditor: () => { childCloses++; },
  };
  h.render(() => {
    focusCalls.length = 0; escapeCalls.length = 0;
    return h.exports.CodexOAuthBindingModal(props);
  });
  const dialogs = () => nodes(h.flush()).filter((node) => node.props?.role === "dialog");
  assert.equal(dialogs().length, 1);
  assert.equal(dialogs()[0].props.ref, focusCalls[0].ref);
  assert.equal(dialogs()[0].props.tabIndex, -1);
  assert.equal(focusCalls.filter((entry) => entry.open && entry.active).length, 1);
  props.oauthBindingQuotaReserveEditorOpen = true;
  assert.equal(dialogs().length, 2);
  assert.equal(focusCalls[0].open, true);
  assert.equal(focusCalls[0].active, false);
  assert.equal(focusCalls[0].restore, true);
  assert.equal(dialogs()[1].props.ref, focusCalls[1].ref);
  assert.equal(dialogs()[0].props["aria-hidden"], true);
  const activeEscape = escapeCalls.filter((entry) => entry.open);
  assert.equal(activeEscape.length, 1);
  activeEscape[0].close();
  assert.equal(childCloses, 1);
  assert.equal(parentCloses, 0);
  props.oauthBindingQuotaReserveEditorOpen = false;
  assert.equal(dialogs().length, 1);
  assert.equal(focusCalls[0].active, true);
  escapeCalls.filter((entry) => entry.open)[0].close();
  assert.equal(parentCloses, 1);
  props.oauthBindingSaving = true;
  dialogs();
  assert.equal(escapeCalls.filter((entry) => entry.open).length, 0);
  h.unmount();
});
