import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';

export interface CodexRecycledAccount {
  id: string;
  account_id: string;
  email: string;
  account_name?: string | null;
  plan_type?: string | null;
  deleted_at: number;
}

export async function listCodexRecycledAccounts(): Promise<CodexRecycledAccount[]> {
  let timeout: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      invoke<CodexRecycledAccount[]>('list_codex_recycled_accounts'),
      new Promise<never>((_, reject) => {
        timeout = setTimeout(() => reject(new Error('CODEX_RECYCLE_BIN_READ_TIMEOUT')), 15_000);
      }),
    ]);
  } finally {
    clearTimeout(timeout);
  }
}
export const restoreCodexRecycledAccount = (recycleId: string) =>
  invoke<void>('restore_codex_recycled_account', { recycleId });
export const deleteCodexRecycledAccount = (recycleId: string) =>
  invoke<void>('delete_codex_recycled_account', { recycleId });
// Use the confirmed snapshot: accounts recycled later must not be removed.
export const emptyCodexRecycleBin = (recycleIds: string[]) =>
  invoke<void>('empty_codex_recycle_bin', { recycleIds });

// Resolve only after the complete snapshot has been saved. Cancellation must
// never advance an export-and-delete operation to its destructive step.
export async function saveCodexRecycledAccounts(recycleIds: string[]): Promise<boolean> {
  if (recycleIds.length === 0) return false;
  const ids = [...recycleIds];
  const path = await save({
    defaultPath: `codex-recycle-bin-${new Date().toISOString().replace(/[:.]/g, '-')}.json`,
    filters: [{ name: 'JSON', extensions: ['json'] }],
  });
  if (!path) return false;
  await invoke<void>('export_codex_recycled_accounts', { recycleIds: ids, path });
  return true;
}
