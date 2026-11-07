import { invoke } from '@tauri-apps/api/core';
import { isCodexLaunchCancelled } from './codexLaunchCancellation';
import {
  isCodexPathRecoveryManaged,
  requestCodexPathRecovery,
} from '../stores/useCodexPathRecoveryStore';

type LegacyRetry = {
  kind: 'default' | 'instance' | 'switchAccount';
  instanceId?: string;
  accountId?: string;
} | {
  kind: 'codexCommand';
  command: 'switch_codex_account' | 'codex_local_access_activate';
  args: Record<string, unknown>;
};

/** Preserve legacy events, but let managed callers retain their exact continuation. */
export function recoverLegacyCodexPath(retry?: LegacyRetry): void {
  if (isCodexPathRecoveryManaged()) return;
  if (retry?.kind === 'codexCommand') {
    if (retry.command !== 'switch_codex_account' && retry.command !== 'codex_local_access_activate') return;
    const command = retry.command;
    const args = { ...retry.args, pathRecoveryManaged: true };
    void requestCodexPathRecovery(() => invoke(command, args), {
      key: JSON.stringify(['legacy-codex-command', command, args]),
      withoutDesktop: () => invoke(command, { ...args, launchAfterSwitch: false }),
    }).catch((error: unknown) => {
      if (!isCodexLaunchCancelled(error)) console.error('Codex path recovery failed:', error);
    });
    return;
  }
  const switching = retry?.kind === 'switchAccount' && !!retry.accountId;
  const args = switching
    ? { accountId: retry.accountId, launchAfterSwitch: true, pathRecoveryManaged: true }
    : { instanceId: retry?.instanceId || '__default__' };
  const command = switching ? 'switch_codex_account' : 'codex_start_instance';
  // Invoke directly: nesting a managed service inside this recovery would wait
  // on its own open dialog instead of continuing the original operation.
  void requestCodexPathRecovery(() => invoke(command, args), {
    key: JSON.stringify(['legacy-codex-path', command, args]),
    withoutDesktop: switching
      ? () => invoke(command, { ...args, launchAfterSwitch: false })
      : undefined,
  }).catch((error: unknown) => {
    if (!isCodexLaunchCancelled(error)) console.error('Codex path recovery failed:', error);
  });
}
