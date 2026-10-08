export type CodexStatsRangeKey =
  | "daily"
  | "rolling7d"
  | "weekly"
  | "monthly"
  | "custom";

export interface CodexStatsTimeRange {
  startAt: number;
  endAt: number;
  startInput: string;
  endInput: string;
}

function formatDateInput(date: Date): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

function buildRange(start: Date, end: Date): CodexStatsTimeRange {
  return {
    startAt: start.getTime(),
    endAt: end.getTime(),
    startInput: formatDateInput(start),
    endInput: formatDateInput(end),
  };
}

export function buildCodexStatsTimeRange(
  key: CodexStatsRangeKey,
  now = new Date(),
): CodexStatsTimeRange {
  const todayStart = new Date(now);
  todayStart.setHours(0, 0, 0, 0);
  const todayEnd = new Date(todayStart);
  todayEnd.setHours(23, 59, 59, 999);
  if (key === "daily" || key === "custom") return buildRange(todayStart, todayEnd);
  if (key === "rolling7d") {
    const rangeStart = new Date(todayStart);
    rangeStart.setDate(rangeStart.getDate() - 6);
    return buildRange(rangeStart, todayEnd);
  }
  if (key === "weekly") {
    const weekStart = new Date(todayStart);
    const mondayOffset = (weekStart.getDay() + 6) % 7;
    weekStart.setDate(weekStart.getDate() - mondayOffset);
    return buildRange(weekStart, todayEnd);
  }
  const monthStart = new Date(todayStart.getFullYear(), todayStart.getMonth(), 1);
  return buildRange(monthStart, todayEnd);
}

export function parseCodexStatsTimeRange(
  startInput: string,
  endInput: string,
): CodexStatsTimeRange | null {
  const datePattern = /^\d{4}-\d{2}-\d{2}$/;
  if (!datePattern.test(startInput) || !datePattern.test(endInput)) return null;
  const startDate = new Date(`${startInput}T00:00:00`);
  const endDate = new Date(`${endInput}T00:00:00`);
  if (formatDateInput(startDate) !== startInput || formatDateInput(endDate) !== endInput) {
    return null;
  }
  const startAt = startDate.getTime();
  endDate.setHours(23, 59, 59, 999);
  const endAt = endDate.getTime();
  if (!Number.isFinite(startAt) || !Number.isFinite(endAt) || endAt < startAt) {
    return null;
  }
  return { startAt, endAt, startInput, endInput };
}
