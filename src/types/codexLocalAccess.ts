export type CodexLocalAccessAddressKind = "local" | "lan";
export type CodexLocalAccessScope = "localhost" | "lan";
export type CodexLocalAccessClientBaseUrlHost = "localhost" | "127.0.0.1";
export type CodexLocalAccessImageGenerationMode =
  "enabled" | "images_only" | "disabled";
export type CodexLocalAccessGatewayMode = "legacy" | "sidecar";
export type CodexLocalAccessRequestKind =
  "text" | "image_generation" | "image_edit" | "other";
export type CodexLocalAccessImageGenerationStatus =
  "unknown" | "available" | "unavailable" | "disabled";

export type CodexLocalAccessRoutingStrategy =
  | "auto"
  | "random"
  | "single_account"
  | "quota_high_first"
  | "quota_low_first"
  | "plan_high_first"
  | "plan_low_first"
  | "expiry_soon_first"
  | "custom";

export interface CodexLocalAccessCustomRoutingRule {
  accountId: string;
  priority: number;
  weight: number;
  isBackup: boolean;
}

export interface CodexLocalAccessOAuthQuotaReserve {
  hourlyPercent: number;
  weeklyPercent: number;
}

export interface CodexLocalAccessAccountModelRule {
  accountId: string;
  excludedModels: string[];
}

export interface CodexLocalAccessModelAlias {
  sourceModel: string;
  alias: string;
  fork: boolean;
}

export interface CodexLocalAccessModelPricing {
  modelId: string;
  longContextThresholdTokens?: number | null;
  inputUsdPerMillion: number;
  outputUsdPerMillion: number;
  cachedInputUsdPerMillion?: number | null;
  standardLongInputUsdPerMillion?: number | null;
  standardLongOutputUsdPerMillion?: number | null;
  standardLongCachedInputUsdPerMillion?: number | null;
  priorityInputUsdPerMillion?: number | null;
  priorityOutputUsdPerMillion?: number | null;
  priorityCachedInputUsdPerMillion?: number | null;
  priorityLongInputUsdPerMillion?: number | null;
  priorityLongOutputUsdPerMillion?: number | null;
  priorityLongCachedInputUsdPerMillion?: number | null;
}

export interface CodexLocalAccessApiKey {
  id: string;
  label: string;
  key: string;
  providerGateway?: unknown | null;
  inheritAccountPool?: boolean;
  accountIds?: string[];
  priorityAccountIds?: string[];
  modelPrefix?: string | null;
  allowedModels: string[];
  excludedModels: string[];
  enabled: boolean;
  createdAt: number;
  updatedAt: number;
  lastUsedAt?: number | null;
}

export interface CodexLocalAccessTimeouts {
  legacyRequestReadTimeoutMs: number;
  legacyUpstreamConnectTimeoutMs: number;
  legacyStreamIdleTimeoutMs: number;
  legacyStreamTotalTimeoutMs: number;
  sidecarStreamOpenTimeoutMs: number;
  sidecarStreamIdleTimeoutMs: number;
  sidecarImageStreamOpenTimeoutMs: number;
  sidecarImageStreamIdleTimeoutMs: number;
  sidecarStreamOpenMaxAttempts: number;
  sidecarStreamKeepaliveSeconds: number;
  websocketConnectTimeoutMs: number;
  websocketInitialMessageTimeoutMs: number;
  websocketIdleTimeoutMs: number;
  websocketHeartbeatIntervalMs: number;
  upstreamSendRetryAttempts: number;
  upstreamSendRetryBaseDelayMs: number;
  upstreamSendRetryMaxDelayMs: number;
  singleAccountStatusRetryAttempts: number;
  singleAccountStatusRetryBaseDelayMs: number;
  singleAccountStatusRetryMaxDelayMs: number;
  sidecarStreamingBootstrapRetries: number;
}

export interface CodexLocalAccessTimeoutPreset {
  id: string;
  name: string;
  timeouts: CodexLocalAccessTimeouts;
  createdAt: number;
  updatedAt: number;
}

export interface CodexLocalAccessCollection {
  enabled: boolean;
  port: number;
  apiKey: string;
  apiKeys: CodexLocalAccessApiKey[];
  accessScope: CodexLocalAccessScope;
  clientBaseUrlHost: CodexLocalAccessClientBaseUrlHost;
  imageGenerationMode: CodexLocalAccessImageGenerationMode;
  gatewayMode: CodexLocalAccessGatewayMode;
  upstreamProxyUrl?: string | null;
  routingStrategy: CodexLocalAccessRoutingStrategy;
  customRoutingRules: CodexLocalAccessCustomRoutingRule[];
  accountModelRules: CodexLocalAccessAccountModelRule[];
  modelAliases: CodexLocalAccessModelAlias[];
  modelPricingVersion: number;
  modelPricings: CodexLocalAccessModelPricing[];
  debugLogs: boolean;
  immediateSseResponse: boolean;
  maxConcurrentImageRequests: number;
  excludedModels: string[];
  sessionAffinity: boolean;
  sessionAffinityTtlMs: number;
  sessionAffinityDefaultEnabledMigrated?: boolean;
  responsesWebsocketsEnabled: boolean;
  maxRetryCredentials: number;
  maxRetryIntervalMs: number;
  timeouts: CodexLocalAccessTimeouts;
  activeTimeoutPresetId: string;
  timeoutPresets: CodexLocalAccessTimeoutPreset[];
  disableCooling: boolean;
  restrictFreeAccounts: boolean;
  boundOauthAccountId?: string | null;
  boundOauthQuotaReserve?: CodexLocalAccessOAuthQuotaReserve | null;
  accountIds: string[];
  createdAt: number;
  updatedAt: number;
}

export interface CodexLocalAccessUsageStats {
  requestCount: number;
  successCount: number;
  failureCount: number;
  clientCanceledCount: number;
  upstreamResponseFailedCount: number;
  streamIncompleteCount: number;
  totalLatencyMs: number;
  textRequestCount: number;
  imageRequestCount: number;
  imageGenerationRequestCount: number;
  imageEditRequestCount: number;
  imageGenerationCapabilityFailureCount: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  cachedTokens: number;
  reasoningTokens: number;
  estimatedCostUsd: number;
}

export interface CodexLocalAccessAccountStats {
  accountId: string;
  email: string;
  usage: CodexLocalAccessUsageStats;
  updatedAt: number;
}

export interface CodexLocalAccessModelStats {
  modelId: string;
  usage: CodexLocalAccessUsageStats;
  updatedAt: number;
}

export interface CodexLocalAccessApiKeyStats {
  apiKeyId: string;
  label: string;
  usage: CodexLocalAccessUsageStats;
  updatedAt: number;
}

export interface CodexLocalAccessStatsWindow {
  since: number;
  updatedAt: number;
  totals: CodexLocalAccessUsageStats;
  accounts: CodexLocalAccessAccountStats[];
  models: CodexLocalAccessModelStats[];
  apiKeys: CodexLocalAccessApiKeyStats[];
}

export interface CodexLocalAccessUsageEvent {
  timestamp: number;
  requestId: string;
  accountId: string;
  email: string;
  apiKeyId: string;
  apiKeyLabel: string;
  /** 多开实例目录 ID（x-cockpit-instance-id） */
  clientInstanceId?: string;
  modelId: string;
  gatewayMode?: CodexLocalAccessGatewayMode | null;
  requestKind: CodexLocalAccessRequestKind;
  serviceTier?: string | null;
  success: boolean;
  httpStatus?: number | null;
  errorCategory: string;
  errorMessage: string;
  latencyMs: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  cachedTokens: number;
  reasoningTokens: number;
  estimatedCostUsd: number;
  modelPricingVersion: number;
  inputUsdPerMillion: number;
  outputUsdPerMillion: number;
  cachedInputUsdPerMillion?: number | null;
}

export interface CodexLocalAccessStats {
  since: number;
  updatedAt: number;
  totals: CodexLocalAccessUsageStats;
  accounts: CodexLocalAccessAccountStats[];
  models: CodexLocalAccessModelStats[];
  apiKeys: CodexLocalAccessApiKeyStats[];
  daily: CodexLocalAccessStatsWindow;
  weekly: CodexLocalAccessStatsWindow;
  monthly: CodexLocalAccessStatsWindow;
  events: CodexLocalAccessUsageEvent[];
}

export interface CodexLocalAccessUsageEventPage {
  events: CodexLocalAccessUsageEvent[];
  total: number;
  page: number;
  pageSize: number;
  totalPages: number;
}

export interface CodexLocalAccessRequestLogQuery {
  page: number;
  pageSize: number;
  statsRange?: "daily" | "weekly" | "monthly" | null;
  startAt?: number | null;
  endAt?: number | null;
  modelQuery?: string | null;
  accountQuery?: string | null;
  apiKeyQuery?: string | null;
  instanceQuery?: string | null;
  gatewayMode?: CodexLocalAccessGatewayMode | null;
  requestKind?: CodexLocalAccessRequestKind | null;
  success?: boolean | null;
  errorCategory?: string | null;
}

export interface CodexLocalAccessAccountCooldown {
  modelId: string;
  nextRetryAt: number;
  remainingMs: number;
  reason: string;
}

export interface CodexLocalAccessAccountHealth {
  accountId: string;
  email: string;
  available: boolean;
  consecutiveFailures: number;
  lastSuccessAt: number | null;
  lastFailureAt: number | null;
  lastFailureStatus: number | null;
  lastFailureCategory: string | null;
  lastFailureMessage: string | null;
  imageGenerationStatus: CodexLocalAccessImageGenerationStatus;
  imageGenerationCheckedAt: number | null;
  schedulerAvailable: boolean | null;
  schedulerReason: string | null;
  schedulerNextRetryAt: number | null;
  cooldowns: CodexLocalAccessAccountCooldown[];
}

export interface CodexLocalAccessProfileAttachment {
  profileDir: string;
  attached: boolean;
  configAttached: boolean;
  authAttached: boolean;
  modelProvider: string | null;
  baseUrl: string | null;
  expectedBaseUrl: string | null;
  error: string | null;
}

export interface CodexLocalAccessQuotaReserveStatus {
  accountId: string;
  snapshotUpdatedAt: number | null;
  snapshotFresh: boolean;
  blocked: boolean;
  warning: boolean;
  effectiveWindow: "hourly" | "weekly" | null;
  effectiveRemainingPercent: number | null;
  effectiveReservePercent: number | null;
}

export interface CodexLocalAccessState {
  collection: CodexLocalAccessCollection | null;
  running: boolean;
  preparing: boolean;
  preparationTotal: number;
  preparationCompleted: number;
  refreshingAccounts: boolean;
  accountRefreshTotal: number;
  accountRefreshCompleted: number;
  defaultProfile: CodexLocalAccessProfileAttachment | null;
  apiPortUrl: string | null;
  baseUrl: string | null;
  lanBaseUrl: string | null;
  modelIds: string[];
  modelPricingPresets: CodexLocalAccessModelPricing[];
  lastError: string | null;
  memberCount: number;
  stats: CodexLocalAccessStats;
  accountHealth: CodexLocalAccessAccountHealth[];
  quotaReserveStatus: CodexLocalAccessQuotaReserveStatus | null;
}

export interface CodexLocalAccessAppendAccountSkipped {
  accountId: string;
  reason:
    | "not_found"
    | "chat_completions_api_key"
    | "free_restricted"
    | "pending_oauth";
}

export interface CodexLocalAccessAppendAccountsResult {
  state: CodexLocalAccessState;
  syncedAccountIds: string[];
  addedAccountIds: string[];
  skippedAccounts: CodexLocalAccessAppendAccountSkipped[];
}

export interface CodexLocalAccessTestResult {
  modelId: string | null;
  latencyMs: number | null;
  output: string | null;
  failure: CodexLocalAccessTestFailure | null;
}

export interface CodexLocalAccessTestFailure {
  title: string;
  stage: string;
  cause: string;
  suggestion: string;
  status: number | null;
  modelId: string | null;
  detail: string | null;
  gatewayOutput: string | null;
}

export interface CodexLocalAccessChatMessage {
  role: "system" | "user" | "assistant";
  content: string;
}

export interface CodexLocalAccessChatResult {
  modelId: string;
  latencyMs: number | null;
  output: string | null;
  failure: CodexLocalAccessTestFailure | null;
}

export type CodexLocalAccessChatStreamEvent =
  | {
      sessionId: string;
      type: "delta";
      content?: string;
      reasoning?: string;
    }
  | {
      sessionId: string;
      type: "done";
      modelId: string;
      latencyMs: number | null;
    }
  | {
      sessionId: string;
      type: "error";
      failure: CodexLocalAccessTestFailure;
    };

export interface CodexLocalAccessPortCleanupResult {
  killedCount: number;
  state: CodexLocalAccessState;
}
