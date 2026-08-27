export type CodebuddyLocalAccessScope = "localhost" | "lan";
export type CodebuddyLocalAccessImageGenerationMode =
  | "enabled"
  | "images_only"
  | "disabled";
export type CodebuddyLocalAccessRequestKind =
  | "text"
  | "image_generation"
  | "image_edit"
  | "other";
export type CodebuddyLocalAccessImageGenerationStatus =
  | "unknown"
  | "available"
  | "unavailable"
  | "disabled";
export type CodebuddyLocalAccessRoutingStrategy =
  | "auto"
  | "random"
  | "single_account"
  | "quota_high_first"
  | "quota_low_first"
  | "plan_high_first"
  | "plan_low_first"
  | "expiry_soon_first"
  | "custom";

export interface CodebuddyLocalAccessCustomRoutingRule {
  accountId: string;
  priority: number;
  weight: number;
  isBackup: boolean;
  isPreferred: boolean;
}

export interface CodebuddyLocalAccessModelAlias {
  sourceModel: string;
  alias: string;
  fork: boolean;
}

export interface CodebuddyLocalAccessApiKey {
  id: string;
  name: string;
  key: string;
  enabled: boolean;
  accountIds?: string[] | null;
  createdAt: number;
  updatedAt: number;
}

export interface CodebuddyLocalAccessCollection {
  enabled: boolean;
  port: number;
  bindHost: string;
  scope: CodebuddyLocalAccessScope;
  intlAccountIds: string[];
  cnAccountIds: string[];
  modelAliases: CodebuddyLocalAccessModelAlias[];
  excludedModels: string[];
  debugLogs: boolean;
  sessionAffinity: boolean;
  sessionAffinityTtlMs: number;
  routingStrategy: CodebuddyLocalAccessRoutingStrategy;
  customRoutingRules: CodebuddyLocalAccessCustomRoutingRule[];
  maxRetryCredentials: number;
  maxRetryIntervalMs: number;
  disableCooling: boolean;
  requestTimeoutMs: number;
  apiKeys: CodebuddyLocalAccessApiKey[];
  imageGenerationMode: CodebuddyLocalAccessImageGenerationMode;
  maxConcurrentImageRequests: number;
}

export interface CodebuddyLocalAccessAccountOption {
  id: string;
  email: string;
  region: "intl" | "cn";
  uid?: string | null;
  enterpriseId?: string | null;
  planType?: string | null;
}

export interface CodebuddyLocalAccessUsageStats {
  requestCount: number;
  successCount: number;
  failureCount: number;
  totalLatencyMs: number;
  textRequestCount: number;
  imageRequestCount: number;
  imageGenerationRequestCount: number;
  imageEditRequestCount: number;
  imageGenerationCapabilityFailureCount: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  totalCredit: number;
  promptCacheHitTokens: number;
  promptCacheMissTokens: number;
  promptCacheWriteTokens: number;
}

export interface CodebuddyLocalAccessModelStats {
  modelId: string;
  usage: CodebuddyLocalAccessUsageStats;
}

export interface CodebuddyLocalAccessApiKeyStats {
  apiKeyId: string;
  usage: CodebuddyLocalAccessUsageStats;
}

export interface CodebuddyLocalAccessAccountStats {
  accountId: string;
  usage: CodebuddyLocalAccessUsageStats;
}

export interface CodebuddyLocalAccessRequestLog {
  requestId: string;
  timestamp: number;
  model: string;
  apiKeyId: string;
  accountId: string;
  status: number;
  success: boolean;
  latencyMs: number;
  inputTokens: number;
  outputTokens: number;
  credit: number;
  promptCacheHitTokens: number;
  promptCacheMissTokens: number;
  promptCacheWriteTokens: number;
  requestKind: CodebuddyLocalAccessRequestKind;
  errorMessage?: string | null;
}

export interface CodebuddyLocalAccessStats {
  since: number;
  totals: CodebuddyLocalAccessUsageStats;
  byModel: CodebuddyLocalAccessModelStats[];
  byApiKey: CodebuddyLocalAccessApiKeyStats[];
  byAccount: CodebuddyLocalAccessAccountStats[];
  recentLogs: CodebuddyLocalAccessRequestLog[];
}

export interface CodebuddyLocalAccessLogPage {
  logs: CodebuddyLocalAccessRequestLog[];
  total: number;
  page: number;
  pageSize: number;
  totalPages: number;
}

export interface CodebuddyLocalAccessAccountHealth {
  accountId: string;
  email: string;
  imageGenerationStatus: CodebuddyLocalAccessImageGenerationStatus;
  imageGenerationCheckedAt?: number | null;
}

export interface CodebuddyLocalAccessState {
  collection: CodebuddyLocalAccessCollection;
  running: boolean;
  actualPort?: number | null;
  lastError?: string | null;
  intlAccounts: CodebuddyLocalAccessAccountOption[];
  cnAccounts: CodebuddyLocalAccessAccountOption[];
  accountHealth: CodebuddyLocalAccessAccountHealth[];
  baseUrl: string;
  /** 局域网模式下供外部设备使用的 URL（如 http://192.168.1.10:11435） */
  lanBaseUrl?: string | null;
}
