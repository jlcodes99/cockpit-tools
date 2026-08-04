import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  CreateManagedCodexTaskInput,
  ManagedCodexEvidenceCursor,
  ManagedCodexEvidenceEventPayload,
  ManagedCodexTaskEvidencePage,
  ManagedCodexTaskResumeMode,
  ManagedCodexTaskRuntimeStatus,
  ManagedCodexTaskSnapshot,
} from '../types/codexManagedTask';

export const MANAGED_CODEX_TASK_UPDATED_EVENT = 'codex-managed-task://updated';
export const MANAGED_CODEX_TASK_EVIDENCE_EVENT = 'codex-managed-task://evidence';

export async function createManagedCodexTask(
  input: CreateManagedCodexTaskInput,
): Promise<ManagedCodexTaskSnapshot> {
  return await invoke('codex_managed_task_create', { input });
}

export async function listManagedCodexTasks(limit = 200): Promise<ManagedCodexTaskSnapshot[]> {
  return await invoke('codex_managed_task_list', { limit });
}

export async function getManagedCodexTask(taskId: string): Promise<ManagedCodexTaskSnapshot> {
  return await invoke('codex_managed_task_get', { taskId });
}

export async function cancelManagedCodexTask(taskId: string): Promise<ManagedCodexTaskSnapshot> {
  return await invoke('codex_managed_task_cancel', { taskId });
}

export async function resumeManagedCodexTask(
  taskId: string,
  mode: ManagedCodexTaskResumeMode,
): Promise<ManagedCodexTaskSnapshot> {
  return await invoke('codex_managed_task_resume', { taskId, mode });
}

export async function listManagedCodexTaskEvidence(
  taskId: string,
  cursor?: ManagedCodexEvidenceCursor,
  limit = 100,
): Promise<ManagedCodexTaskEvidencePage> {
  return await invoke('codex_managed_task_list_evidence', {
    taskId,
    cursor: cursor ?? null,
    limit,
  });
}

export async function getManagedCodexTaskRuntimeStatus(): Promise<ManagedCodexTaskRuntimeStatus> {
  return await invoke('codex_managed_task_runtime_status');
}

export async function listenManagedCodexTaskUpdated(
  handler: (task: ManagedCodexTaskSnapshot) => void,
): Promise<UnlistenFn> {
  return await listen<ManagedCodexTaskSnapshot>(MANAGED_CODEX_TASK_UPDATED_EVENT, (event) => {
    handler(event.payload);
  });
}

export async function listenManagedCodexTaskEvidence(
  handler: (payload: ManagedCodexEvidenceEventPayload) => void,
): Promise<UnlistenFn> {
  return await listen<ManagedCodexEvidenceEventPayload>(
    MANAGED_CODEX_TASK_EVIDENCE_EVENT,
    (event) => handler(event.payload),
  );
}
