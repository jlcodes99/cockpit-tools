import type { SshCodexSyncStatus } from '../types/sshServer';

// verified describes the remote files. A refresh warning must remain visible.
export function sshSyncSucceeded(status: Pick<SshCodexSyncStatus, 'verified' | 'error'>): boolean {
  return status.verified && !status.error;
}
