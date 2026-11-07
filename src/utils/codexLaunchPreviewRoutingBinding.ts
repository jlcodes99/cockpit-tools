import type { CodexAccount } from "../types/codex";
import { isStandardCodexOAuthAccount } from "../types/codex";

/**
 * 「第三方 API 路由」（混合模型路由）需要的底座账号。
 *
 * 优先级：预览账号本身 → 实例当前绑定（已经是可用的 OAuth 底座时不要改动它）→ 账号列表里
 * 任意一个直接登录的 OAuth 订阅账号；都没有时返回 null（调用方需要给出明确提示）。
 */
export function resolveLaunchPreviewRoutingBaseAccount(
  accounts: CodexAccount[],
  launchAccount?: CodexAccount | null,
  preferredAccountId?: string | null,
): CodexAccount | null {
  if (launchAccount && isStandardCodexOAuthAccount(launchAccount)) {
    return launchAccount;
  }
  const preferred = preferredAccountId?.trim();
  if (preferred) {
    const preferredAccount = accounts.find((item) => item.id === preferred);
    if (preferredAccount && isStandardCodexOAuthAccount(preferredAccount)) {
      return preferredAccount;
    }
  }
  return accounts.find((item) => isStandardCodexOAuthAccount(item)) ?? null;
}

/**
 * 目标实例当前生效的绑定账号。
 *
 * 默认实例开启「跟随本地账号」时，实例绑定会被本地当前账号覆盖，校验与保存看到的都是它。
 */
export function resolveInstanceEffectiveBindAccountId(options: {
  isDefaultInstance: boolean;
  followLocalAccount: boolean;
  bindAccountId?: string | null;
  localCurrentAccountId?: string | null;
}): string | null {
  if (options.isDefaultInstance && options.followLocalAccount) {
    return options.localCurrentAccountId?.trim() || null;
  }
  return options.bindAccountId?.trim() || null;
}

/**
 * 需要随路由配置一起写入实例的绑定账号。
 *
 * 后端保存路由配置时会用「实例当前生效的绑定账号」校验路由底座，所以只要实例当前绑定不是
 * 这个 OAuth 账号就必须一起下发；否则会出现「明明要切到这个 OAuth 账号，却提示必须先绑定
 * OAuth 订阅账号」。与实例表单一致：绑定值没变化时不下发。
 */
export function resolveLaunchPreviewRoutingBindAccountId(options: {
  routingEnabled: boolean;
  routingBaseAccountId?: string | null;
  instanceEffectiveBindAccountId?: string | null;
}): string | undefined {
  if (!options.routingEnabled) {
    return undefined;
  }
  const routingBaseAccountId = options.routingBaseAccountId?.trim();
  if (!routingBaseAccountId) {
    return undefined;
  }
  if (routingBaseAccountId === (options.instanceEffectiveBindAccountId?.trim() || null)) {
    return undefined;
  }
  return routingBaseAccountId;
}
