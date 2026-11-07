import i18n from '../i18n';

export async function withClaudeProfileErrors<T>(operation: () => Promise<T>): Promise<T> {
  try {
    return await operation();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    const keys = {
      CLAUDE_PROFILE_RESTORE_BUSY: 'common.profileRestoreBusy',
      CLAUDE_PROFILE_INVALID: 'common.profileRestoreInvalid',
      CLAUDE_PROFILE_RECOVERY_FAILED: 'common.profileRestoreRecoveryFailed',
    } as const;
    for (const [code, key] of Object.entries(keys)) {
      if (message.startsWith(code)) {
        throw new Error(i18n.t(key, { detail: message.slice(code.length).replace(/^:\s*/, '') }));
      }
    }
    throw error;
  }
}
