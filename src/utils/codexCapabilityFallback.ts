import { CODEX_CAPABILITY_IDS, type CodexCapabilityFallbackConfig, type CodexCapabilityId } from '../types/codexLocalAccess';
import { isCodexApiKeyAccount, isCodexChatgptPlanAccount, type CodexAccount } from '../types/codex';

/** Mirror native_capability_support's hard exclusions; absence is never a positive claim. */
export function isCapabilityProtocolUnsupported(account: CodexAccount | undefined, capability: CodexCapabilityId): boolean {
  if (!account) return false;
  const hosted = ['tool_search', 'web_search', 'image_generation', 'file_search', 'code_interpreter', 'computer_use', 'mcp', 'programmatic_tool_calling'].includes(capability);
  if (isCodexChatgptPlanAccount(account)) {
    return (hosted && capability !== 'web_search') || ['audio_input', 'video_input', 'compact'].includes(capability);
  }
  // Native routing uses the persisted protocol, not host-name inference.
  const chat = isCodexApiKeyAccount(account) && account.api_wire_api === 'chat_completions' && !account.upstream_grok_account_id?.trim();
  return chat && (hosted || capability === 'compact');
}

export function copyCapabilityFallback(value?: CodexCapabilityFallbackConfig): CodexCapabilityFallbackConfig {
  return {
    enabled: true,
    accountIds: [...(value?.accountIds ?? [])],
    overrides: (value?.overrides ?? []).map((item) => ({ ...item, capabilities: { ...item.capabilities } })),
  };
}

export function capabilityFallbackFingerprint(value?: CodexCapabilityFallbackConfig): string {
  const copy = copyCapabilityFallback(value);
  return JSON.stringify({ ...copy, overrides: copy.overrides.map((item) => ({ ...item,
    capabilities: Object.fromEntries(Object.entries(item.capabilities).sort(([a], [b]) => a.localeCompare(b))),
  })) });
}

export interface CapabilityFallbackErrors {
  backups?: string;
  overrides: Record<number, { account?: string; model?: string }>;
}
export function validateCapabilityFallback(value: CodexCapabilityFallbackConfig, members: ReadonlySet<string>): CapabilityFallbackErrors {
  const errors: CapabilityFallbackErrors = { overrides: {} };
  if (value.accountIds.some((id) => !members.has(id))) errors.backups = 'capabilityFallback.errorMembers';
  const seen = new Set<string>();
  value.overrides.forEach((item, index) => {
    const row: { account?: string; model?: string } = {};
    if (!members.has(item.accountId)) row.account = 'capabilityFallback.errorMembers';
    const model = item.model.trim();
    if (!model || new TextEncoder().encode(model).length > 256 || /[\u0000-\u001f\u007f-\u009f]/.test(model) || (model.includes('*') && model !== '*')) row.model = 'capabilityFallback.errorModel';
    const key = JSON.stringify([item.accountId, model]);
    if (seen.has(key)) row.model = 'capabilityFallback.errorDuplicate';
    seen.add(key);
    if (Object.keys(row).length) errors.overrides[index] = row;
  });
  return errors;
}

export function hasCapabilityFallbackErrors(errors: CapabilityFallbackErrors): boolean {
  return Boolean(errors.backups || Object.keys(errors.overrides).length);
}

/** User declarations are not proof that a protocol's hard restrictions can be bypassed. */
export function declaredCapability(value: CodexCapabilityFallbackConfig, accountId: string, model: string, capability: CodexCapabilityId) {
  if (!(CODEX_CAPABILITY_IDS as readonly string[]).includes(capability)) return 'unknown';
  const wildcard = value.overrides.find((item) => item.accountId === accountId && item.model === '*')?.capabilities[capability];
  return value.overrides.find((item) => item.accountId === accountId && item.model === model)?.capabilities[capability] ?? wildcard ?? 'unknown';
}

export function capabilityLabelKey(id: string): string {
  return (CODEX_CAPABILITY_IDS as readonly string[]).includes(id) ? `capabilityFallback.capability.${id}` : 'capabilityFallback.unknown';
}

export function capabilityFallbackErrorKey(error: unknown): string {
  const code = String(error).replace(/^Error:\s*/, '').trim();
  const keys: Record<string, string> = {
    ACCOUNT_REQUIRED: 'errorBackups', ACCOUNT_NOT_IN_POOL: 'errorMembers', INVALID_MODEL: 'errorModel',
    DUPLICATE_OVERRIDE: 'errorDuplicate', NATIVE_REQUIRED: 'nativeOnly',
  };
  const key = code.startsWith('CAPABILITY_FALLBACK_') ? keys[code.slice('CAPABILITY_FALLBACK_'.length)] : undefined;
  return `capabilityFallback.${key ?? 'errorGeneric'}`;
}
