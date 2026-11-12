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
      t: (key: string, args?: { value: string }) => args ? `${args.value} pp` : key,
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
    points, onlyChanges, onOnlyChangesChange: (value: boolean) => { onlyChanges = value; },
  }));
  const pagination = () => elements(h.flush()).find((element) => element.type === Pagination)!.props;
  const rowAt = (index: number) => elements(h.flush()).filter((element) => element.type === "tr").find((row) =>
    elements(row).some((element) => element.type === "time" && element.props.dateTime === new Date(points[index].observedAt).toISOString()));
  const deltas = (index: number) => elements(rowAt(index)).filter((element) =>
    element.props.className === "codex-quota-history__delta").map((element) => element.props.children);
  const timeCount = () => elements(h.flush()).filter((element) => element.type === "time").length;

  assert.equal(timeCount(), 20);
  assert.ok(rowAt(21));
  assert.deepEqual(deltas(21), ["-1 pp", "0 pp"]);
  assert.deepEqual(deltas(3), ["—", "0 pp"]);
  assert.equal(rowAt(1), undefined);
  pagination().onNextPage();
  assert.equal(timeCount(), 2);
  assert.deepEqual(deltas(1), ["-2 pp", "-40 pp"]);
  assert.ok(elements(rowAt(1)).some((element) => element.type === "strong" && element.props.children === "0%"));
  assert.deepEqual(deltas(0), ["—", "—"]);

  elements(h.flush()).find((element) => element.props.role === "switch")!.props.onClick();
  assert.equal(pagination().currentPage, 1);
  assert.equal(timeCount(), 4);
  assert.deepEqual(deltas(2), ["—", "+41 pp"]);
  assert.deepEqual(deltas(20), ["+1 pp", "0 pp"]);
  assert.equal(rowAt(3), undefined);
  h.unmount();
});
