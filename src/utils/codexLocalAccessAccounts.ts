import {
  isCodexApiKeyAccount,
  isCodexExplicitFreePlanType,
  type CodexAccount,
} from '../types/codex.ts';

const CHAT_COMPLETIONS_PROVIDER_HOSTS = [
  "api.deepseek.com",
  "api.moonshot.cn",
  "api.siliconflow.cn",
  "api.siliconflow.com",
  "open.bigmodel.cn",
  "api.z.ai",
  "volces.com",
  "bytepluses.com",
  "qianfan.baidubce.com",
  "dashscope.aliyuncs.com",
  "api.stepfun.com",
  "api.stepfun.ai",
  "modelscope.cn",
  "api.longcat.chat",
  "api.minimax.io",
  "api.mini-max.chat",
  "api.minimaxi.com",
  "api.tbox.cn",
  "api.mimo.dev",
  "api.xiaomimimo.com",
  "token-plan-cn.xiaomimimo.com",
  "api.novita.ai",
  "integrate.api.nvidia.com",
  "runapi.co",
  "www.relaxycode.com",
  "cp.compshare.cn",
  "api.lemondata.cc",
  "e-flowcode.cc",
  "cc-api.pipellm.ai",
  "openrouter.ai",
  "api.therouter.ai",
];

export type CodexLocalAccessAccountIneligibleReason =
  | "chat_completions_api_key"
  | "free_restricted";

export function isCodexChatCompletionsApiKeyAccount(account: CodexAccount): boolean {
  if (!isCodexApiKeyAccount(account)) {
    return false;
  }
  const wireApi = (account.api_wire_api || "").trim();
  if (wireApi === "chat_completions") {
    return true;
  }
  if (wireApi === "responses") {
    return false;
  }
  const baseUrl = (account.api_base_url || "").trim().toLowerCase();
  if (!baseUrl) {
    return false;
  }
  if (baseUrl.includes("/chat/completions")) {
    return true;
  }
  try {
    const host = new URL(baseUrl).hostname.toLowerCase();
    return CHAT_COMPLETIONS_PROVIDER_HOSTS.some((pattern) =>
      host.includes(pattern),
    );
  } catch {
    return false;
  }
}

export function getCodexLocalAccessAccountIneligibleReason(
  account: CodexAccount,
  restrictFreeAccounts: boolean,
): CodexLocalAccessAccountIneligibleReason | null {
  if (isCodexChatCompletionsApiKeyAccount(account)) {
    return "chat_completions_api_key";
  }
  if (restrictFreeAccounts && isCodexExplicitFreePlanType(account.plan_type)) {
    return "free_restricted";
  }
  return null;
}

export function isCodexLocalAccessEligibleAccount(
  account: CodexAccount,
  restrictFreeAccounts: boolean,
): boolean {
  return getCodexLocalAccessAccountIneligibleReason(
    account,
    restrictFreeAccounts,
  ) === null;
}

export function filterCodexLocalAccessAccountIds(
  accountIds: string[],
  accounts: CodexAccount[],
  restrictFreeAccounts: boolean,
): string[] {
  const accountById = new Map(accounts.map((account) => [account.id, account]));
  const seen = new Set<string>();
  const next: string[] = [];

  for (const accountId of accountIds) {
    const account = accountById.get(accountId);
    if (!account || !isCodexLocalAccessEligibleAccount(account, restrictFreeAccounts)) {
      continue;
    }
    if (!seen.has(accountId)) {
      seen.add(accountId);
      next.push(accountId);
    }
  }

  return next;
}
