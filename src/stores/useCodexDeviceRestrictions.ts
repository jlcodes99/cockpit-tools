import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';

export type DeviceRestrictionScope = 'desktop' | 'quota' | 'api' | 'oauth' | 'token';
export interface CodexDeviceRestriction {
  subject: string;
  scope: DeviceRestrictionScope;
  source: string;
  instanceId: string | null;
  status: 'restricted' | 'recovered';
  suspended: boolean;
  firstSeenAt: number;
  lastSeenAt: number;
  lastSuccessAt: number | null;
  occurrences: number;
  revision: number;
}

interface State {
  records: CodexDeviceRestriction[];
  loading: boolean;
  failed: boolean;
  refresh: () => Promise<void>;
}
let flight: Promise<void> | undefined;
let pending = false;

export const useCodexDeviceRestrictions = create<State>((set) => ({
  records: [], loading: false, failed: false,
  refresh: () => {
    pending = true;
    if (flight) return flight;
    flight = (async () => {
      do {
        pending = false;
        set({ loading: true, failed: false });
        try {
          const records: CodexDeviceRestriction[] = [];
          let after: string | null = null;
          for (;;) {
            const page: CodexDeviceRestriction[] = await invoke<CodexDeviceRestriction[]>('codex_list_device_restrictions', { after });
            if (!page.length) break;
            records.push(...page);
            after = page[page.length - 1].subject;
          }
          set({ records, loading: false });
        } catch {
          // Preserve the last known evidence; a read failure is not an empty/healthy state.
          set({ failed: true, loading: false });
        }
      } while (pending);
    })().finally(() => { flight = undefined; });
    return flight;
  },
}));

export async function allowCodexDeviceRestrictionRetry(record: CodexDeviceRestriction) {
  await invoke('codex_allow_device_restriction_retry', {
    subject: record.subject, scope: record.scope, revision: record.revision,
  });
  await useCodexDeviceRestrictions.getState().refresh();
}
