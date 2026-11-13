import assert from "node:assert/strict";
import test from "node:test";
import { loadHookModule } from "../../../tests/helpers/reactHookHarness";
import type { QuotaHistoryPoint } from "./codexQuotaHistoryChart";

type Element = { type: unknown; props: Record<string, any> };

function elements(value: unknown): Element[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  if (!value || typeof value !== "object" || !("props" in value)) return [];
  const element = value as Element;
  return [element, ...elements(element.props.children)];
}

test("quota details page newest first and compare to the original preceding sample", () => {
  const Pagination = () => null;
  const h = loadHookModule(new URL("./CodexQuotaHistoryDetails.tsx", import.meta.url), {
    "../PaginationControls": { PaginationControls: Pagination },
    "react-i18next": { useTranslation: () => ({
      t: (key: string) => key,
      i18n: { language: "en-US" },
    }) },
  });
  const points: QuotaHistoryPoint[] = Array.from({ length: 22 }, (_, i) => ({
    observedAt: i * 60_000,
    bucketStart: i * 60_000,
    shortRemaining: i === 2 ? null : i === 0 ? 100 : i === 20 ? 99 : 98,
    weeklyRemaining: i === 0 ? 40 : i === 1 ? 0 : 41,
  }));
  let onlyChanges = false;
  h.render(() => h.exports.CodexQuotaHistoryDetails({
    history: { points, shortConsumed: 3, weeklyConsumed: 40, intervalMinutes: 1 },
    onlyChanges, onOnlyChangesChange: (value: boolean) => { onlyChanges = value; },
  }));
  const pagination = () => elements(h.flush()).find((element) => element.type === Pagination)!.props;
  const rowAt = (index: number) => elements(h.flush()).filter((element) => element.type === "tr").find((row) =>
    elements(row).some((element) => element.type === "time" && element.props.dateTime === new Date(points[index].observedAt).toISOString()));
  const deltas = (index: number) => elements(rowAt(index)).filter((element) =>
    element.props.className === "codex-quota-history__delta").map((element) => ({
      text: element.props.children,
      direction: element.props["data-direction"],
    }));
  const timeCount = () => elements(h.flush()).filter((element) => element.type === "time").length;
  const totals = () => elements(h.flush()).filter((element) => element.type === "dd").map((element) => element.props.children);

  assert.equal(timeCount(), 20);
  assert.deepEqual(totals(), ["-3%", "-40%"]);
  assert.ok(rowAt(21));
  assert.deepEqual(deltas(21), [
    { text: "-1%", direction: "down" },
    { text: "--", direction: undefined },
  ]);
  assert.deepEqual(deltas(3), [
    { text: "--", direction: undefined },
    { text: "--", direction: undefined },
  ]);
  assert.equal(rowAt(1), undefined);
  pagination().onNextPage();
  assert.equal(timeCount(), 2);
  assert.deepEqual(totals(), ["-3%", "-40%"]);
  assert.deepEqual(deltas(1), [
    { text: "-2%", direction: "down" },
    { text: "-40%", direction: "down" },
  ]);
  assert.ok(elements(rowAt(1)).some((element) => element.type === "strong" && element.props.children === "0%"));
  assert.deepEqual(deltas(0), [
    { text: "--", direction: undefined },
    { text: "--", direction: undefined },
  ]);

  elements(h.flush()).find((element) => element.props.role === "switch")!.props.onClick();
  assert.equal(pagination().currentPage, 1);
  // 4 changed rows (21, 20, 2, 1) + 1 baseline row (0)
  assert.equal(timeCount(), 5);
  assert.deepEqual(totals(), ["-3%", "-40%"]);
  assert.deepEqual(deltas(2), [
    { text: "--", direction: undefined },
    { text: "+41%", direction: "up" },
  ]);
  assert.deepEqual(deltas(20), [
    { text: "+1%", direction: "up" },
    { text: "--", direction: undefined },
  ]);
  assert.ok(rowAt(0));
  assert.equal(rowAt(3), undefined);
  h.unmount();
});

test("quota details formats decimal changes and retains single decimal precision", () => {
  const Pagination = () => null;
  const h = loadHookModule(new URL("./CodexQuotaHistoryDetails.tsx", import.meta.url), {
    "../PaginationControls": { PaginationControls: Pagination },
    "react-i18next": { useTranslation: () => ({
      t: (key: string) => key,
      i18n: { language: "en-US" },
    }) },
  });
  const points: QuotaHistoryPoint[] = [
    { observedAt: 0, bucketStart: 0, shortRemaining: 100, weeklyRemaining: 80 },
    { observedAt: 60_000, bucketStart: 60_000, shortRemaining: 99.9, weeklyRemaining: 79.5 },
    { observedAt: 120_000, bucketStart: 120_000, shortRemaining: 100, weeklyRemaining: 79.5 },
  ];
  h.render(() => h.exports.CodexQuotaHistoryDetails({
    history: { points, shortConsumed: 0, weeklyConsumed: null, intervalMinutes: 1 },
    onlyChanges: true, onOnlyChangesChange: () => {},
  }));
  const rowAt = (index: number) => elements(h.flush()).filter((element) => element.type === "tr").find((row) =>
    elements(row).some((element) => element.type === "time" && element.props.dateTime === new Date(points[index].observedAt).toISOString()));
  const remainings = (index: number) => elements(rowAt(index)).filter((element) =>
    element.props.className === "codex-quota-history__remaining").map((element) => element.props.children);
  const deltas = (index: number) => elements(rowAt(index)).filter((element) =>
    element.props.className === "codex-quota-history__delta").map((element) => ({
      text: element.props.children,
      direction: element.props["data-direction"],
    }));

  assert.deepEqual(remainings(1), ["99.9%", "79.5%"]);
  assert.deepEqual(deltas(1), [
    { text: "-0.1%", direction: "down" },
    { text: "-0.5%", direction: "down" },
  ]);
  assert.deepEqual(deltas(2), [
    { text: "+0.1%", direction: "up" },
    { text: "--", direction: undefined },
  ]);
  h.unmount();
});
