export interface CodexSwitcherQuota {
  hourly_percentage?: number | null;
  hourly_reset_time?: number | null;
  hourly_window_minutes?: number | null;
  hourly_window_present?: boolean | null;
  weekly_percentage?: number | null;
  weekly_reset_time?: number | null;
  weekly_window_minutes?: number | null;
  weekly_window_present?: boolean | null;
  raw_data?: unknown;
}

export interface CodexSwitcherQuotaError {
  code?: string | null;
  message: string;
  timestamp?: number | null;
}

export interface CodexSwitcherAccount {
  id: string;
  email?: string | null;
  auth_mode?: "oauth" | "apikey" | string | null;
  api_base_url?: string | null;
  api_provider_mode?: "openai_builtin" | "custom" | string | null;
  api_provider_id?: string | null;
  api_provider_name?: string | null;
  bound_oauth_account_id?: string | null;
  name?: string | null;
  account_name?: string | null;
  user_id?: string | null;
  plan_type?: string | null;
  subscription_active_until?: string | null;
  auth_file_plan_type?: string | null;
  organization_id?: string | null;
  account_id?: string | null;
  account_structure?: string | null;
  quota?: CodexSwitcherQuota | null;
  quota_error?: CodexSwitcherQuotaError | null;
  requires_reauth?: boolean | null;
  reauth_reason?: string | null;
  is_api_key_auth?: boolean | null;
  banned?: boolean | null;
  disabled?: boolean | null;
  is_current?: boolean | null;
  current?: boolean | null;
  created_at?: number | null;
  last_used?: number | null;
  updated_at?: number | null;
}

export interface CodexSwitcherSettings {
  restart_app_after_switch?: boolean;
  delete_banned_accounts?: boolean;
}

export interface CodexSwitcherListResponse {
  accounts: CodexSwitcherAccount[];
  current_account_id?: string | null;
}

export type CodexSwitcherAdBlock =
  | {
      type: "text";
      text: string;
    }
  | {
      type: "markdown";
      markdown: string;
    }
  | {
      type: "image";
      src: string;
      alt?: string;
      href?: string;
    }
  | {
      type: "video";
      src: string;
      poster?: string;
      title?: string;
    }
  | {
      type: "button";
      label: string;
      href: string;
    };

export interface CodexSwitcherRemoteAd {
  id?: string;
  title?: string;
  blocks?: CodexSwitcherAdBlock[];
}
