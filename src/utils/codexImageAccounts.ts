import {
  isCodexApiKeyAccount,
  isCodexChatgptPlanAccount,
  isCodexAgentIdentityAccount,
  isCodexWebSessionAccount,
  type CodexAccount,
} from '../types/codex';

export type CodexImageAccountModels = Record<string, string>;
export const DEFAULT_CODEX_IMAGE_MODELS = ['gpt-image-2.5', 'gpt-image-2'];

/** 发现只读取已有模型目录，不探测网络，也不把视频型号加入图片目录。 */
export function discoverImageModels(accounts: CodexAccount[]): string[] {
  return [...new Set(accounts.filter((account) => !isCodexChatgptPlanAccount(account)).flatMap((account) => [...(account.api_model_catalog ?? []), ...Object.values(account.api_image_generation_account_models ?? {})])
    .map((id) => id.trim()).filter((id) => id.length <= 200 && !/\s|[\x00-\x1f\x7f]/u.test(id) &&
      (id.startsWith('gpt-image-') || id.startsWith('grok-imagine-image')) && !id.toLowerCase().includes('video')))];
}

/** 裸2/2.5按族筛选，其他自定义全名只exact匹配。 */
export function isImageModelInFamily(model: string, family?: string | null): boolean {
  const selected = family?.trim();
  const actual = model.trim();
  if (!selected) return false;
  if (!DEFAULT_CODEX_IMAGE_MODELS.includes(selected)) return actual === selected;
  return actual === selected || (actual.startsWith(`${selected}-`) && actual.length > selected.length + 1);
}

export function imageModelsForAccount(account: CodexAccount, family?: string | null, deleted: string[] = []): string[] {
  if (isCodexChatgptPlanAccount(account)) return [];
  return [...new Set((account.api_model_catalog ?? []).map((id) => id.trim()).filter((id) => isImageModelInFamily(id, family) && !deleted.includes(id)))];
}

export function isImageGenerationEligibleAccount(account: CodexAccount, model?: string | null, deleted: string[] = []): boolean {
  if (isCodexChatgptPlanAccount(account)) return false;
  if (isCodexAgentIdentityAccount(account) || isCodexWebSessionAccount(account)) return false;
  const selected = model?.trim();
  if (!selected) return false;
  if (isCodexApiKeyAccount(account)) return imageModelsForAccount(account, selected, deleted).length > 0;
  return DEFAULT_CODEX_IMAGE_MODELS.includes(selected) && !deleted.includes(selected) && !(account.plan_type || '').trim().toLowerCase().includes('free');
}

export function eligibleImageAccountIds(ids: string[], accounts: CodexAccount[], model?: string | null, deleted: string[] = []): string[] {
  const eligible = new Set(accounts.filter((account) => isImageGenerationEligibleAccount(account, model, deleted)).map((account) => account.id));
  return [...new Set(ids)].filter((id) => eligible.has(id));
}

/** 失效显式型号禁止回退；只有从未配置过map且catalog支持裸名时才使用裸名。 */
export function resolveImageAccountModel(account: CodexAccount, model?: string | null, models: CodexImageAccountModels = {}, deleted: string[] = []): string | null {
  if (!isCodexApiKeyAccount(account)) return null;
  const available = imageModelsForAccount(account, model, deleted);
  const explicit = models[account.id]?.trim();
  if (explicit) return available.includes(explicit) ? explicit : null;
  return model && available.includes(model.trim()) ? model.trim() : null;
}

export function selectedImageAccountModels(ids: string[], accounts: CodexAccount[], model?: string | null, models: CodexImageAccountModels = {}, deleted: string[] = []): CodexImageAccountModels {
  return Object.fromEntries(accounts.filter((account) => ids.includes(account.id)).flatMap((account) => {
    const actual = resolveImageAccountModel(account, model, models, deleted);
    return actual ? [[account.id, actual]] : [];
  }));
}

export function configuredImageAccountIds(ids: string[], accounts: CodexAccount[], model?: string | null, models: CodexImageAccountModels = {}, deleted: string[] = []): string[] {
  return eligibleImageAccountIds(ids, accounts, model, deleted).filter((id) => {
    const account = accounts.find((item) => item.id === id)!;
    return !isCodexApiKeyAccount(account) || resolveImageAccountModel(account, model, models, deleted) !== null;
  });
}
