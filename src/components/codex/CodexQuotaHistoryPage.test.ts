import assert from "node:assert/strict";
import test from "node:test";
import { deferred, loadHookModule, settlePromises } from "../../../tests/helpers/reactHookHarness";
import type { CodexAccount } from "../../types/codex";
import zhCN from "../../locales/zh-CN.json";
import * as chart from "./codexQuotaHistoryChart";

type Element = { type: unknown; props: Record<string, any> };
function elements(value: unknown): Element[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  if (!value || typeof value !== "object" || !("props" in value)) return [];
  const element = value as Element;
  return [element, ...elements(element.props.children)];
}

test("the quota page queries its selected account, date and minute interval and discards stale responses", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const Dropdown = () => null;
  const Details = () => null;
  const jobs: { args: { accountId: string; from: number; to: number; intervalMinutes: number }; result: ReturnType<typeof deferred<chart.QuotaHistoryPoint[]>> }[] = [];
  let recorded: ((event: { payload: string[] }) => void) | undefined;
  const refreshed: string[] = [];
  const account = (id: string) => ({ id, email: id, created_at: 1 } as CodexAccount);
  const currentAccount = account("b");
  const h = loadHookModule(new URL("./CodexQuotaHistoryPage.tsx", import.meta.url), {
    "@tauri-apps/api/core": { invoke(name: string, args: typeof jobs[number]["args"]) {
      assert.equal(name, "get_codex_quota_history");
      const result = deferred<chart.QuotaHistoryPoint[]>();
      jobs.push({ args, result });
      return result.promise;
    } },
    "react-i18next": { useTranslation: () => ({
      t: (key: string) => key.split(".").reduce<unknown>((value, part) =>
        (value as Record<string, unknown> | undefined)?.[part], zhCN) ?? key,
      i18n: { language: "zh-CN" },
    }) },
    "../SingleSelectDropdown": { SingleSelectDropdown: Dropdown },
    "./CodexQuotaHistoryDetails": { CodexQuotaHistoryDetails: Details },
    "./CodexQuotaHistorySettings": { CodexQuotaHistorySettings: () => null },
    "../../stores/useCodexAccountStore": { useCodexAccountStore: (select: (state: unknown) => unknown) =>
      select({ currentAccount, refreshQuota: async (id: string) => { refreshed.push(id); } }) },
    "../../types/codex": { isCodexApiKeyAccount: () => false },
    "../../utils/tauriEventListener": { subscribeTauriEvent(name: string, listener: typeof recorded) {
      if (name === "codex:quota-history-cleaned") return () => {};
      assert.equal(name, "codex:quota-history-recorded");
      recorded = listener;
      return () => { recorded = undefined; };
    } },
    "./codexQuotaHistoryChart": chart,
  });
  h.render(() => h.exports.CodexQuotaHistoryPage({ accounts: [account("a"), currentAccount], resolveDisplayName: (a: CodexAccount) => a.email }));
  const node = (selector: (node: Element) => boolean) => {
    const result = elements(h.flush()).find(selector);
    assert.ok(result);
    return result;
  };
  context.mock.timers.tick(100);
  assert.equal(jobs[0].args.accountId, "b");
  assert.deepEqual(elements(h.flush()).filter((el) => el.type === "button" && "aria-pressed" in el.props)
    .map((el) => el.props.children), ["图表", "明细", "日", "周", "月"]);
  node((el) => el.type === Dropdown).props.onChange("a");
  node((el) => el.type === "input").props.onChange({ target: { value: "7" } });
  node((el) => el.type === "button" && el.props.children === "周").props.onClick();
  h.flush();
  context.mock.timers.tick(100);
  const week = chart.quotaHistoryDateRange("thisWeek", 0);
  assert.deepEqual({ ...jobs[1].args }, { accountId: "a", intervalMinutes: 7, ...week });
  jobs[0].result.resolve([{ observedAt: 8, bucketStart: 0, shortRemaining: 100, weeklyRemaining: 100 }]);
  await settlePromises();
  node((el) => el.props.role === "status");
  const points = [{ observedAt: 8, bucketStart: 0, shortRemaining: 90, weeklyRemaining: 80 }];
  jobs[1].result.resolve(points);
  await settlePromises();
  node((el) => el.type === "button" && el.props.children === "明细").props.onClick();
  const details = node((el) => el.type === Details);
  assert.equal(details.props.points, points);
  assert.equal(node((el) => el.type === Dropdown).props.value, "a");
  assert.equal(node((el) => el.type === "input").props.value, "7");
  assert.equal(node((el) => el.type === "button" && el.props.children === "周").props["aria-pressed"], true);
  assert.equal(jobs.length, 2);
  recorded?.({ payload: ["b"] });
  context.mock.timers.tick(250);
  assert.equal(jobs.length, 2);
  recorded?.({ payload: ["a"] });
  context.mock.timers.tick(250);
  assert.equal(jobs.length, 3);
  node((el) => el.props["aria-label"] === zhCN.common.refresh).props.onClick();
  await settlePromises();
  assert.deepEqual(refreshed, ["a"]);
  h.unmount();
});
