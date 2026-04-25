export interface ClaudeAccount {
  id: string;
  email: string;
  name?: string | null;
  tags?: string[] | null;

  config_dir: string;
  login_mode: string;
  login_hint_email?: string | null;
  anthropic_base_url?: string | null;
  anthropic_auth_token?: string | null;
  disable_nonessential_traffic?: boolean;

  logged_in: boolean;
  auth_method?: string | null;
  api_provider?: string | null;
  org_id?: string | null;
  org_name?: string | null;
  subscription_type?: string | null;

  status_raw?: unknown;

  created_at: number;
  last_used: number;
  last_synced_at?: number | null;
}

export interface ClaudeUsage {
  inlineSuggestionsUsedPercent: number | null;
  chatMessagesUsedPercent: number | null;
  totalPercentUsed: number | null;
}

export function getClaudeAccountDisplayName(account: ClaudeAccount): string {
  const email = account.email?.trim();
  if (email) return email;
  const name = account.name?.trim();
  if (name) return name;
  const hint = account.login_hint_email?.trim();
  if (hint) return hint;
  const baseUrl = account.anthropic_base_url?.trim();
  if (baseUrl) return baseUrl;
  return account.id;
}

export function getClaudeLoginModeLabel(account: Pick<ClaudeAccount, 'login_mode'>): string {
  switch ((account.login_mode || '').trim().toLowerCase()) {
    case 'auth_token':
      return 'Auth Token';
    case 'console':
      return 'Console';
    case 'sso':
      return 'SSO';
    case 'email':
      return 'Email Hint';
    case 'claudeai':
    default:
      return 'Claude.ai';
  }
}

export function getClaudePlanBadge(account: ClaudeAccount): string {
  const subscription = (account.subscription_type || '').trim();
  if (subscription) return subscription.toUpperCase();
  if ((account.login_mode || '').trim().toLowerCase() === 'auth_token') {
    return account.logged_in ? 'TOKEN' : 'ENV';
  }
  return account.logged_in ? 'ACTIVE' : 'PROFILE';
}

export function getClaudePlanBadgeClass(account: ClaudeAccount): string {
  const subscription = (account.subscription_type || '').trim().toLowerCase();
  if (!subscription) return account.logged_in ? 'pro' : 'unknown';
  if (subscription.includes('enterprise') || subscription.includes('team')) return 'enterprise';
  if (subscription.includes('pro') || subscription.includes('max')) return 'pro';
  if (subscription.includes('free')) return 'free';
  return 'unknown';
}

export function getClaudeUsage(): ClaudeUsage {
  return {
    inlineSuggestionsUsedPercent: null,
    chatMessagesUsedPercent: null,
    totalPercentUsed: null,
  };
}
