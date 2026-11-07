export function isCodexLaunchCancelled(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return message.replace(/^(?:Error:\s*)+/, '').trim() === 'CODEX_START_CANCELLED';
}
