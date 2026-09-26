import type { ProxyActivitySummaryEntry } from '../services/codexProxyActivityService';

/** One traffic sample. Rates are bytes per second. */
export interface CodexProxyTrafficSample { at: number; up: number; down: number }

export interface CodexProxyTraffic {
  /** False until the first summary arrives, so the UI shows a placeholder instead of 0. */
  ready: boolean;
  upPerSecond: number;
  downPerSecond: number;
  sessionUpload: number;
  sessionDownload: number;
  samples: CodexProxyTrafficSample[];
  activeConnections: number;
  unavailable: boolean;
}

/** Every proxy-bindable account in one poll. */
export interface ProxyTrafficTotals {
  upload: number;
  download: number;
  activeConnections: number;
  supported: boolean;
}

export interface ProxyTrafficReading {
  at: number;
  /** `null` means the summary command failed; the previous totals stay untouched. */
  totals: ProxyTrafficTotals | null;
}

/** 200 samples at a 3 second interval cover roughly the last 10 minutes. */
export const PROXY_TRAFFIC_SAMPLE_LIMIT = 200;

function nonNegative(value: number): number {
  return Number.isFinite(value) && value > 0 ? value : 0;
}

export function createProxyTraffic(): CodexProxyTraffic {
  return {
    ready: false,
    upPerSecond: 0,
    downPerSecond: 0,
    sessionUpload: 0,
    sessionDownload: 0,
    samples: [],
    activeConnections: 0,
    unavailable: false,
  };
}

/**
 * The traffic bar always reports the whole workspace, so a failing or unsupported account
 * only lowers `supported`; it never removes the counters of the accounts that still report.
 */
export function sumProxyActivitySummary(
  entries: readonly ProxyActivitySummaryEntry[],
): ProxyTrafficTotals {
  let upload = 0;
  let download = 0;
  let activeConnections = 0;
  let supported = entries.length === 0;
  for (const entry of entries) {
    upload += nonNegative(entry.upload);
    download += nonNegative(entry.download);
    activeConnections += nonNegative(entry.connectionCount);
    supported = supported || entry.supported;
  }
  return { upload, download, activeConnections, supported };
}

function ratePerSecond(before: number, after: number, elapsedMs: number): number {
  const delta = after - before;
  // A counter that decreased means the engine restarted; report no rate instead of a negative one.
  if (!Number.isFinite(delta) || delta <= 0) return 0;
  const rate = (delta * 1000) / elapsedMs;
  return Number.isFinite(rate) && rate > 0 ? rate : 0;
}

export function computeProxyTrafficRates(
  previous: ProxyTrafficTotals | null,
  next: ProxyTrafficTotals,
  elapsedMs: number,
): { upPerSecond: number; downPerSecond: number } {
  if (!previous || !Number.isFinite(elapsedMs) || elapsedMs <= 0) {
    return { upPerSecond: 0, downPerSecond: 0 };
  }
  return {
    upPerSecond: ratePerSecond(previous.upload, next.upload, elapsedMs),
    downPerSecond: ratePerSecond(previous.download, next.download, elapsedMs),
  };
}

function sessionDelta(before: number | null, after: number): number {
  if (before === null) return 0;
  const delta = after - before;
  return Number.isFinite(delta) && delta > 0 ? delta : 0;
}

export function appendProxyTrafficSample(
  samples: readonly CodexProxyTrafficSample[],
  sample: CodexProxyTrafficSample,
  limit = PROXY_TRAFFIC_SAMPLE_LIMIT,
): CodexProxyTrafficSample[] {
  const next = [...samples, sample];
  const keep = Number.isFinite(limit) && limit > 0 ? Math.floor(limit) : next.length;
  return next.length > keep ? next.slice(next.length - keep) : next;
}

/**
 * Pure assembly for the traffic bar: the hook only owns the timer and the React state.
 * The session total accumulates observed deltas since mount, never the engine's own lifetime.
 */
export function applyProxyTrafficReading(
  previous: CodexProxyTraffic,
  previousReading: ProxyTrafficReading | null,
  reading: ProxyTrafficReading,
): CodexProxyTraffic {
  if (!reading.totals || !reading.totals.supported) {
    return { ...previous, upPerSecond: 0, downPerSecond: 0, unavailable: true };
  }
  const totals = reading.totals;
  const baseline = previousReading?.totals ?? null;
  const elapsedMs = previousReading ? reading.at - previousReading.at : 0;
  const rates = computeProxyTrafficRates(baseline, totals, elapsedMs);
  return {
    ready: true,
    upPerSecond: rates.upPerSecond,
    downPerSecond: rates.downPerSecond,
    sessionUpload: previous.sessionUpload + sessionDelta(baseline?.upload ?? null, totals.upload),
    sessionDownload: previous.sessionDownload + sessionDelta(baseline?.download ?? null, totals.download),
    samples: appendProxyTrafficSample(previous.samples, {
      at: reading.at,
      up: rates.upPerSecond,
      down: rates.downPerSecond,
    }),
    activeConnections: totals.activeConnections,
    unavailable: false,
  };
}

const TRAFFIC_UNITS = ['B', 'KB', 'MB', 'GB'] as const;

/** Byte formatting for the traffic bar: B, KB, MB, GB (values above GB stay in GB). */
export function formatTrafficBytes(bytes: number): string {
  const value = nonNegative(bytes);
  if (value < 1024) return `${Math.round(value)} B`;
  let scaled = value;
  let unit = 0;
  while (scaled >= 1024 && unit < TRAFFIC_UNITS.length - 1) {
    scaled /= 1024;
    unit += 1;
  }
  const digits = scaled >= 100 ? 0 : scaled >= 10 ? 1 : 2;
  return `${scaled.toFixed(digits)} ${TRAFFIC_UNITS[unit]}`;
}
