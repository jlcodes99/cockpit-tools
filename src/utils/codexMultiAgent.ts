import type { CodexMultiAgentConfig } from '../types/codexLocalAccess';

export function copyMultiAgent(_value?: CodexMultiAgentConfig): CodexMultiAgentConfig {
  return { optimize: true, orphanDelegation: true, compatModels: [] };
}
export function multiAgentFingerprint(value?: CodexMultiAgentConfig) {
  return JSON.stringify(copyMultiAgent(value));
}
export function validateMultiAgent(value: CodexMultiAgentConfig, members: Set<string>) {
  const errors: Record<number, { account?: string; model?: string }> = {};
  const seen = new Set<string>();
  value.compatModels.forEach((entry, index) => {
    const model = entry.model.trim();
    const identity = `${entry.accountId}\0${model.toLowerCase()}`;
    const error: { account?: string; model?: string } = {};
    if (!members.has(entry.accountId)) error.account = 'multiAgent.errorAccount';
    if (!model || new TextEncoder().encode(model).length > 256 || /[\p{Cc}*]/u.test(model) || seen.has(identity)) error.model = 'multiAgent.errorModel';
    seen.add(identity);
    if (Object.keys(error).length) errors[index] = error;
  });
  return errors;
}
