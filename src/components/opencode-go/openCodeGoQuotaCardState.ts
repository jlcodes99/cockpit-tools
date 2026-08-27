export const OPEN_CODE_GO_QUOTA_WINDOW_IDS = [
  'rolling',
  'weekly',
  'monthly',
] as const;

export type OpenCodeGoQuotaWindowId =
  (typeof OPEN_CODE_GO_QUOTA_WINDOW_IDS)[number];

export interface OpenCodeGoQuotaWindowValue {
  usedPercent?: number | null;
  remainingPercent?: number | null;
  resetsAt?: number | null;
}

export type OpenCodeGoQuotaCardStatus =
  | 'loading'
  | 'ready'
  | 'error'
  | 'unavailable';

export interface OpenCodeGoQuotaCardState {
  id: OpenCodeGoQuotaWindowId;
  label: string;
  status: OpenCodeGoQuotaCardStatus;
  remainingPercent: number | null;
  percentageText: string;
  resetText: string;
  resetsAt: number | null;
}

const WINDOW_LABELS: Record<OpenCodeGoQuotaWindowId, string> = {
  rolling: 'Rolling 5h',
  weekly: 'Weekly',
  monthly: 'Monthly',
};

function finiteNumber(value: number | null | undefined): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function clampPercent(value: number): number {
  return Math.max(0, Math.min(100, value));
}

function formatPercent(value: number): string {
  const rounded = Math.round(value * 10) / 10;
  return `${rounded}%`;
}

export function formatOpenCodeGoResetCountdown(
  resetsAt: number | null | undefined,
  nowMs = Date.now(),
): string {
  const normalizedReset = finiteNumber(resetsAt);
  if (normalizedReset == null || normalizedReset <= 0) {
    return 'Reset unavailable';
  }

  const remainingSeconds = Math.floor(normalizedReset - nowMs / 1000);
  if (remainingSeconds <= 0) return 'Reset due';

  const totalMinutes = Math.floor(remainingSeconds / 60);
  if (totalMinutes < 1) return 'Resets in <1m';

  const days = Math.floor(totalMinutes / (24 * 60));
  const hours = Math.floor((totalMinutes % (24 * 60)) / 60);
  const minutes = totalMinutes % 60;
  const parts: string[] = [];

  if (days > 0) parts.push(`${days}d`);
  if (hours > 0) parts.push(`${hours}h`);
  if (days === 0 && minutes > 0) parts.push(`${minutes}m`);

  return `Resets in ${parts.join(' ')}`;
}

export function buildOpenCodeGoQuotaCardStates(input: {
  windows?: Partial<
    Record<OpenCodeGoQuotaWindowId, OpenCodeGoQuotaWindowValue | null>
  >;
  loadingWindows?: readonly OpenCodeGoQuotaWindowId[];
  errors?: Partial<Record<OpenCodeGoQuotaWindowId, string | null>>;
  nowMs?: number;
}): OpenCodeGoQuotaCardState[] {
  const loadingWindows = new Set(input.loadingWindows ?? []);
  const nowMs = input.nowMs ?? Date.now();

  return OPEN_CODE_GO_QUOTA_WINDOW_IDS.map((id) => {
    const window = input.windows?.[id];
    const error = input.errors?.[id]?.trim();
    const remainingValue = finiteNumber(window?.remainingPercent);
    const usedValue = finiteNumber(window?.usedPercent);
    const computedRemaining =
      remainingValue ?? (usedValue == null ? null : 100 - usedValue);
    const remainingPercent =
      computedRemaining == null ? null : clampPercent(computedRemaining);
    const resetsAt = finiteNumber(window?.resetsAt);

    if (loadingWindows.has(id)) {
      return {
        id,
        label: WINDOW_LABELS[id],
        status: 'loading',
        remainingPercent: null,
        percentageText: 'Loading…',
        resetText: '',
        resetsAt,
      };
    }

    if (error) {
      return {
        id,
        label: WINDOW_LABELS[id],
        status: 'error',
        remainingPercent: null,
        percentageText: error,
        resetText: '',
        resetsAt,
      };
    }

    if (remainingPercent == null) {
      return {
        id,
        label: WINDOW_LABELS[id],
        status: 'unavailable',
        remainingPercent: null,
        percentageText: 'Unavailable',
        resetText: '',
        resetsAt,
      };
    }

    return {
      id,
      label: WINDOW_LABELS[id],
      status: 'ready',
      remainingPercent,
      percentageText: formatPercent(remainingPercent),
      resetText: formatOpenCodeGoResetCountdown(resetsAt, nowMs),
      resetsAt,
    };
  });
}
