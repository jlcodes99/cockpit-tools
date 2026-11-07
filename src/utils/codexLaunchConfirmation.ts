import type { InstanceProfile } from '../types/instance';

export function isCodexLaunchUnconfirmed(error: unknown): boolean {
  const message = String(error ?? '').replace(/^Error:\s*/, '');
  return message.startsWith('CODEX_START_CONFIRMATION_UNCERTAIN:')
    || message === 'Codex startup confirmation failed: no stable new default process';
}

function normalizeDirectory(path: string): string {
  const normalized = path.replace(/\\/g, '/').replace(/\/+$/, '');
  return /^[a-z]:\//i.test(normalized) || normalized.startsWith('//')
    ? normalized.toLowerCase() : normalized;
}

export function codexLaunchExcludedPids(error: unknown): number[] {
  const message = String(error ?? '').replace(/^Error:\s*/, '');
  if (!message.startsWith('CODEX_START_CONFIRMATION_UNCERTAIN:')) return [];
  try {
    const value = JSON.parse(message.slice('CODEX_START_CONFIRMATION_UNCERTAIN:'.length));
    return Array.isArray(value.beforePids)
      ? value.beforePids.filter((pid: unknown) => typeof pid === 'number' && Number.isInteger(pid) && pid > 0) : [];
  } catch { return []; }
}

export function confirmedCodexLaunch(
  instances: InstanceProfile[], instanceId: string, userDataDir?: string, excludedPids: number[] = [],
): InstanceProfile | undefined {
  return instances.find((instance) => instance.id === instanceId
    && instance.running && !!instance.lastPid
    && !excludedPids.includes(instance.lastPid)
    && (!userDataDir || normalizeDirectory(instance.userDataDir) === normalizeDirectory(userDataDir)));
}
