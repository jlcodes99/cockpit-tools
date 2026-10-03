import type { CodebuddyUsage, QuotaCategoryGroup } from "./codebuddy-suite";

/** Credential kind as stored in pi's auth.json. */
export type PiCredentialKind = "api_key" | "oauth" | string;

export interface PiProviderSummary {
  provider: string;
  kind: PiCredentialKind;
  /** OAuth expiry (ms since epoch) when present. */
  expires_at?: number | null;
  /** Set when the provider is a third-party gateway in models.json. */
  base_url?: string | null;
  api?: string | null;
  name?: string | null;
  auth_header?: boolean;
  models?: string[];
  /** Masked key tail, e.g. `****9liY`. */
  key_hint?: string | null;
}

/**
 * Sanitized pi profile returned to the UI. Credentials stay in the Rust
 * backend; `access_token` is always empty for shared-view compatibility.
 */
export interface PiAccount {
  id: string;
  email: string;
  access_token: "";
  tags?: string[] | null;
  providers: PiProviderSummary[];
  default_provider?: string | null;
  default_model?: string | null;
  default_thinking_level?: string | null;
  working_dir?: string | null;
  created_at: number;
  last_used: number;
}

export function getPiAccountDisplayEmail(account: PiAccount): string {
  return account.email?.trim() || account.id;
}

/** Badge shows provider ids raw (project rule: no localization of plan values). */
export function getPiPlanBadge(account: PiAccount): string {
  const providers = account.providers ?? [];
  if (providers.length === 0) return "";
  if (providers.length === 1) return providers[0].provider;
  return `${providers[0].provider} +${providers.length - 1}`;
}

export function getPiProvidersText(account: PiAccount): string {
  return (account.providers ?? [])
    .map((item) => `${item.provider} (${item.kind})`)
    .join(", ");
}

export function isPiOAuthExpired(item: PiProviderSummary, now = Date.now()): boolean {
  return item.kind === "oauth" && typeof item.expires_at === "number" && item.expires_at < now;
}

export function getPiUsage(account: PiAccount): CodebuddyUsage {
  const providers = account.providers ?? [];
  const isNormal = providers.length > 0;
  return {
    dosageNotifyCode: isNormal ? "normal" : "no_credentials",
    isNormal,
    inlineSuggestionsUsedPercent: null,
    chatMessagesUsedPercent: null,
    allowanceResetAt: null,
  };
}

/** pi has no quota API; shared view expects a groups adapter. */
export function getPiQuotaGroups(): QuotaCategoryGroup[] {
  return [];
}
