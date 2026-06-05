export interface Account {
    id: string;
    email: string;
    name?: string;
    tags?: string[];
    notes?: string;
    token: TokenData;
    quota?: QuotaData;
    quota_error?: QuotaErrorInfo;
    disabled?: boolean;
    disabled_reason?: string;
    disabled_at?: number;
    protected_models?: string[];
    created_at: number;
    last_used: number;
}

export interface TokenData {
    access_token: string;
    refresh_token: string;
    expires_in: number;
    expiry_timestamp: number;
    token_type: string;
    email?: string;
    project_id?: string;
    is_gcp_tos?: boolean;
    session_id?: string;
}

export interface QuotaData {
    models: ModelQuota[];
    last_updated: number;
    is_forbidden?: boolean;
    subscription_tier?: string;
    credits?: CreditInfo[];
    tier_id?: string;
}

export interface CreditInfo {
    credit_type: string;
    credit_amount?: string;
    minimum_credit_amount_for_usage?: string;
}

export interface QuotaErrorInfo {
    code?: number;
    message: string;
    timestamp: number;
}

export interface ModelQuota {
    name: string;
    display_name?: string;
    percentage: number;
    reset_time: string;
}

export interface RefreshStats {
    total: number;
    success: number;
    failed: number;
    details: string[];
}
