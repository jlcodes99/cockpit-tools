export type QuotaHistoryRange = "today" | "thisWeek" | "thisMonth";

export interface QuotaHistoryPoint {
  observedAt: number;
  bucketStart: number;
  shortRemaining: number | null;
  weeklyRemaining: number | null;
}

export function quotaHistoryDateRange(period: QuotaHistoryRange, offset: number, now = new Date()) {
  const from = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const to = new Date(from);
  if (period === "today") {
    from.setDate(from.getDate() + offset);
    to.setTime(from.getTime());
    to.setDate(to.getDate() + 1);
  } else if (period === "thisWeek") {
    from.setDate(from.getDate() - ((from.getDay() + 6) % 7) + offset * 7);
    to.setTime(from.getTime());
    to.setDate(to.getDate() + 7);
  } else {
    from.setDate(1);
    from.setMonth(from.getMonth() + offset);
    to.setTime(from.getTime());
    to.setMonth(to.getMonth() + 1);
  }
  return { from: from.getTime(), to: to.getTime() };
}

export const QUOTA_PLOT = { width: 920, height: 252, left: 49, right: 44, top: 20, bottom: 44 };

export function quotaHistoryTicks(first: number, last: number, period: QuotaHistoryRange, count: number, lang: string) {
  const sameDay = new Date(first).toDateString() === new Date(last).toDateString();
  const format = new Intl.DateTimeFormat(lang, period === "today" || sameDay
    ? { hour: "2-digit", minute: "2-digit" } : { month: "short", day: "numeric" });
  const firstLabel = format.format(first);
  const lastLabel = format.format(last);
  if (firstLabel === lastLabel) return [{ value: first, label: firstLabel }];

  const ticks = new Map([[firstLabel, first], [lastLabel, last]]);
  for (let index = 1; index < count - 1; index++) {
    const value = first + (last - first) * index / (count - 1);
    const label = format.format(value);
    if (!ticks.has(label)) ticks.set(label, value);
  }
  return [...ticks].map(([label, value]) => ({ label, value })).sort((a, b) => a.value - b.value);
}

export function quotaHistorySeries(
  points: QuotaHistoryPoint[],
  key: "shortRemaining" | "weeklyRemaining",
  x: (time: number) => number,
  y: (remaining: number) => number,
  interval: number,
) {
  const path: string[] = [];
  const singles: Array<{ x: number; y: number }> = [];
  let previous: QuotaHistoryPoint | undefined;
  let single: { x: number; y: number } | undefined;
  for (const point of points) {
    const remaining = point[key];
    if (remaining === null) {
      previous = undefined;
      if (single) singles.push(single);
      single = undefined;
      continue;
    }
    const position = { x: x(point.observedAt), y: y(remaining) };
    const connected = previous && point.bucketStart - previous.bucketStart <= interval;
    path.push(`${connected ? "L" : "M"}${position.x.toFixed(2)},${position.y.toFixed(2)}`);
    if (!connected && single) singles.push(single);
    single = connected ? undefined : position;
    previous = point;
  }
  if (single) singles.push(single);
  return { path: path.join(" "), singles };
}

export function nearestQuotaHistoryPoint(points: QuotaHistoryPoint[], time: number): number {
  let start = 0;
  let end = points.length - 1;
  while (start < end) {
    const middle = Math.floor((start + end) / 2);
    if (points[middle].observedAt < time) start = middle + 1;
    else end = middle;
  }
  return start > 0 && time - points[start - 1].observedAt <= points[start].observedAt - time
    ? start - 1 : start;
}
