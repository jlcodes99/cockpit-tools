export type RemoteKeyRange = 'today' | '7d' | 'month' | 'custom';
export function remoteKeyRange(preset: Exclude<RemoteKeyRange, 'custom'>, now = Date.now()) {
  const date = new Date(now);
  const midnight = Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate());
  return {
    startAt: preset === 'today' ? midnight : preset === 'month' ? Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), 1) : midnight - 6 * 86_400_000,
    endAt: midnight + 86_400_000,
  };
}
export function customRemoteKeyRange(start: string, end: string, now = Date.now()) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(start) || !/^\d{4}-\d{2}-\d{2}$/.test(end)) return null;
  const startAt = Date.parse(`${start}T00:00:00Z`);
  const endAt = Date.parse(`${end}T00:00:00Z`) + 86_400_000;
  if (!Number.isFinite(startAt) || !Number.isFinite(endAt) || startAt < 0 || endAt <= startAt || endAt - startAt > 93 * 86_400_000 || endAt > now + 86_400_000) return null;
  if (new Date(startAt).toISOString().slice(0, 10) !== start || new Date(endAt - 86_400_000).toISOString().slice(0, 10) !== end) return null;
  return { startAt, endAt };
}
