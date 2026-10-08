import type { CodexAccount } from '../types/codex';
import { canUseCodexAccountProxy } from './codexAccountProxy';
import { proxySummary } from './codexProxyPresentation';

/** 后端返回的出口外观；字段与账号响应里的脱敏摘要保持一致。 */
export interface CodexOAuthProxyUseSummary {
  protocol: string;
  server?: string;
  port?: number;
  name?: string;
  sourceName?: string;
  sourceId?: string;
  itemId?: string;
  groupId?: string | null;
  selectedName?: string | null;
}

/** 本次登录实际使用的出口；`input` 只在地址可回填时返回。 */
export interface CodexOAuthProxyUse {
  source: 'account' | 'explicit';
  summary: CodexOAuthProxyUseSummary;
  input?: string;
}

/**
 * 重新授权已有账号是否默认沿用该账号生效出口。
 *
 * 首次添加没有账号记录，macOS 当前构建不支持内置授权窗口代理，
 * 非普通 OAuth 账号也没有账号代理，这几种情况都保持原有默认行为。
 */
export function shouldDefaultReauthProxy(options: {
  active: boolean;
  isMacOS: boolean;
  account: CodexAccount | null | undefined;
}): boolean {
  if (!options.active || options.isMacOS || !options.account) return false;
  return canUseCodexAccountProxy(options.account);
}

/**
 * 发起授权时发送的代理参数。
 *
 * 关闭代理时不发送任何出口；输入框留空且处于账号出口模式时只发送账号 ID，
 * 由后端按「账号独立绑定 > 统一代理」解析，前端不推测具体地址。
 */
export function oauthStartProxyArgs(options: {
  enabled: boolean;
  input: string;
  usesAccountExit: boolean;
  reauthAccountId: string;
}): { proxyUrl: string | null; reauthAccountId: string | null } {
  if (!options.enabled) return { proxyUrl: null, reauthAccountId: null };
  const input = options.input.trim();
  if (input) return { proxyUrl: input, reauthAccountId: null };
  const reauthAccountId = options.reauthAccountId.trim();
  if (options.usesAccountExit && reauthAccountId) {
    return { proxyUrl: null, reauthAccountId };
  }
  return { proxyUrl: null, reauthAccountId: null };
}

export interface OauthProxyStateAfterStart {
  enabled: boolean;
  usesAccountExit: boolean;
  /** 已确认的出口展示文案；null 表示该账号当前没有生效出口。 */
  label: string | null;
  /** 回填输入框的地址；null 表示保持用户输入或留空。 */
  input: string | null;
}

/**
 * 后端确认本次登录出口后如何更新界面。
 *
 * 只有账号出口模式才改写代理状态：用户自己填写的地址不干预；
 * 账号当前没有生效出口时回到原有默认授权路径，不借用其他账号的代理。
 */
export function oauthProxyUseAfterStart(options: {
  usesAccountExit: boolean;
  input: string;
  proxy: CodexOAuthProxyUse | null | undefined;
}): OauthProxyStateAfterStart | null {
  if (!options.usesAccountExit) return null;
  const proxy = options.proxy;
  if (!proxy) {
    return { enabled: false, usesAccountExit: false, label: null, input: null };
  }
  return {
    enabled: true,
    usesAccountExit: true,
    label: proxySummary(proxy.summary) || proxy.summary.protocol,
    input: proxy.input && !options.input.trim() ? proxy.input : null,
  };
}
