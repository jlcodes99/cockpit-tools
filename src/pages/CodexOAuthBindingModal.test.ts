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

test("new OAuth bindings leave quota reservation disabled by default", () => {
  for (const collection of [undefined, null, {}, { boundOauthAccountId: null, boundOauthQuotaReserve: null }]) {
    assert.equal(resolveOAuthBindingQuotaReserve(collection), null);
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

test("saved strict protection and its safety margin survive reopening", () => {
  assert.deepEqual(resolveOAuthBindingQuotaReserve({
    boundOauthAccountId: "oauth-1",
    boundOauthQuotaReserve: { hourlyPercent: 10, weeklyPercent: 10, strict: true, safetyMarginPercent: 5 },
  }), { hourlyPercent: 10, weeklyPercent: 10, strict: true, safetyMarginPercent: 5 });
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
  for (const overlay of nodes(h.flush()).filter((node) => node.props?.className?.includes("modal-overlay"))) {
    assert.equal(overlay.props.onClick, undefined, "background clicks must keep both dialogs open");
  }
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

test("quota protection is built in and validates its margin for both engines", () => {
  const h = modalHarness();
  const saved: unknown[] = [];
  const props: any = {
    t: (key: string, fallback: any) => typeof fallback === "string" ? fallback : key,
    oauthBindingTargetActive: false, isLocalAccessOAuthBinding: true,
    oauthBindingQuotaReserveEditorOpen: true,
    oauthBindingQuotaReserve: { hourlyPercent: 10, weeklyPercent: 10 },
    oauthBindingQuotaReserveFieldErrors: {}, oauthBindingHourlyReserveDraft: "10",
    oauthBindingWeeklyReserveDraft: "10", oauthBindingHourlyReserveInputRef: { current: null },
    oauthBindingWeeklyReserveInputRef: { current: null },
    confirmOAuthBindingQuotaReserveEditor: (value: unknown) => saved.push(value),
    setOauthBindingQuotaReserveFieldErrors: () => {},
    localAccessState: { collection: { gatewayMode: "native" } },
  };
  h.render(() => h.exports.CodexOAuthBindingModal(props));
  let tree = nodes(h.flush());
  const confirm = () => nodes(h.flush()).find((node) => node.type === "button" && text(node) === "确认")!;
  assert.equal(tree.some((node) => node.props?.type === "checkbox"), false);
  const margin = tree.find((node) => node.type === "input" && node.props.maxLength === 2)!;
  let focused = 0;
  margin.props.ref.current = { scrollIntoView() {}, focus() { focused++; } };
  margin.props.onChange({ target: { value: "0" } });
  confirm().props.onClick();
  assert.equal(saved.length, 0);
  assert.equal(focused, 1);
  assert.match(text(h.flush()), /safetyMarginInvalid/);
  nodes(h.flush()).find((node) => node.type === "input" && node.props.maxLength === 2)!.props.onChange({ target: { value: "5" } });
  confirm().props.onClick();
  assert.equal(saved.length, 1);
  assert.equal((saved[0] as any).strict, undefined);
  assert.equal((saved[0] as any).safetyMarginPercent, 5);
  assert.doesNotMatch(text(h.flush()), /safetyMarginInvalid/);
  props.localAccessState = { collection: { gatewayMode: "sidecar" } };
  tree = nodes(h.flush());
  assert.equal(tree.some((node) => node.props?.type === "checkbox"), false);
  assert.equal(tree.some((node) => node.type === "input" && node.props.maxLength === 2), true);
  assert.doesNotMatch(text(h.flush()), /strictNativeOnly/);
  h.unmount();
});
