export type CodexBudgetPeriod = 'lifetime' | 'daily' | 'weekly' | 'monthly';
export interface CodexApiKeyBudget {
  period: CodexBudgetPeriod;
  limitMicrousd: number | null;
  dailyLimitMicrousd: number | null;
  maxConcurrent: number | null;
  dailyImageLimit: number | null;
}
export interface CodexBudgetBucket { start: number; usedMicrousd: number; imageRequests: number }
export interface CodexApiKeyBudgetUsage {
  lifetimeMicrousd: number;
  unmeteredRequests: number;
  daily: CodexBudgetBucket;
  weekly: CodexBudgetBucket;
  monthly: CodexBudgetBucket;
}
export interface CodexRemoteConnection {
  product: 'cockpit-tools';
  protocolVersion: 1;
  serviceId: string;
  serviceName: string;
  keyLabel: string;
  status: string;
  models: string[];
  modelCatalog?: { models: Array<{ slug: string; display_name?: string; priority?: number; [field: string]: unknown }> } | null;
  catalogRevision?: string | null;
  keyViewsVersion?: number;
  imageModels: string[];
  capabilities: { responses: boolean; images: boolean; imageEdits: boolean; hostedImages: boolean; usage: boolean; websockets: boolean };
  tokenLimit: number | null;
  tokenUsed: number;
  budget: {
    policy: CodexApiKeyBudget;
    usedMicrousd: number;
    remainingMicrousd: number | null;
    dailyUsedMicrousd: number;
    dailyRemainingMicrousd: number | null;
    resetsAt: number | null;
    lifetimeMicrousd: number;
    unmeteredRequests: number;
    dailyImageRequests: number;
    timezone: 'UTC';
    inFlightMayExceed: boolean;
  };
  updatedAt: number;
}

export interface CodexRemoteKeyQuery {
  startAt: number;
  endAt: number;
  beforeTime?: number;
  beforeId?: number;
  pageSize?: number;
  model?: string;
  success?: boolean;
  kind?: string;
  threadId?: string;
  instanceId?: string;
  installationId?: string;
  serviceTier?: string;
}
export interface CodexRemoteKeyUsage {
  requestCount: number;
  successCount: number;
  canceledCount: number;
  imageRequestCount: number;
  totalLatencyMs: number;
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  cachedTokens: number;
  estimatedCostUsd: number;
  settledMicrousd: number;
  unrecordedCount: number;
  unmeteredCount: number;
}
export interface CodexRemoteKeyResponse {
  ready: boolean;
  previousSnapshot?: boolean;
  startAt?: number;
  endAt?: number;
  timezone?: 'UTC';
  firstRetainedAt?: number | null;
  updatedAt: number;
}
export interface CodexRemoteKeyStats extends CodexRemoteKeyResponse {
  summary?: CodexRemoteKeyUsage;
  trend?: Array<{ bucketStart: number; usage: CodexRemoteKeyUsage }>;
  hourly?: boolean;
}
export interface CodexRemoteKeyRequest {
  id: number;
  timestamp: number;
  requestId: string;
  model: string;
  kind: string;
  outcome: 'success' | 'canceled' | 'incomplete' | 'timeout' | 'failed';
  httpStatus: number | null;
  latencyMs: number;
  firstResponseMs: number | null;
  outputGenerationMs?: number | null;
  inputTokens: number;
  outputTokens: number;
  cachedTokens: number;
  totalTokens: number;
  estimatedCostUsd: number;
  settledMicrousd: number | null;
  settlementStatus: 'settled' | 'unrecorded' | 'unmetered';
  settledAt: number | null;
}
export interface CodexRemoteKeyRequests extends CodexRemoteKeyResponse {
  items?: CodexRemoteKeyRequest[];
  nextCursor?: { beforeTime: number; beforeId: number } | null;
}
