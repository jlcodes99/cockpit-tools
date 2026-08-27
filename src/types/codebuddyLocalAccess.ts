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
  /** SSE 流式请求立即返回 200（先写 SSE 头再转发上游）。 */
  immediateSseResponse?: boolean;
  /** 客户端 Key 启用 Responses WebSocket 传输（对齐 Codex responsesWebsockets）。 */
  responsesWebsocketsEnabled?: boolean;
  /** 纯文本模型视觉子代理开关（agentic 模式）：开启后纯文本模型可接收图片并自主调用混元视觉模型看图。 */
  visionToolEnabled?: boolean;
}

export interface CodebuddyLocalAccessAccountOption {
  id: string;
  email: string;
  region: "intl" | "cn";
  uid?: string | null;
  enterpriseId?: string | null;
  planType?: string | null;
  /// 计划徽章 CSS 类（K12 / TEAM / FREE / PLUS / PRO / ENTERPRISE）。
  planClass?: string | null;
  /// 剩余 credits。
  quotaRemain?: number | null;
  /// 总容量 credits。
  quotaTotal?: number | null;
  /// 计划过期时间戳（毫秒）。
  subscriptionExpiryMs?: number | null;
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
  /** 客户端主动取消的请求数（HTTP aborted / 流 client_gone）。 */
  clientCanceledCount?: number;
  /** 上游响应失败（非 2xx / 鉴权失败等）的请求数。 */
  upstreamResponseFailedCount?: number;
  /** 流式响应未正常完成（idle 超时 / 写失败 / 流错误）的请求数。 */
  streamIncompleteCount?: number;
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
  /** 失败分类（client_canceled / stream_incomplete / upstream_response_failed），成功或未分类为 undefined。 */
  errorCategory?: string | null;
  errorMessage?: string | null;
  /** 是否由「纯文本视觉子代理」处理（主模型 + 视觉模型协作）。 */
  visionSubagent?: boolean;
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

export interface CodebuddyLocalAccessAccountCooldown {
  modelId: string;
  nextRetryAt: number;
  remainingMs: number;
  reason: string;
}

export interface CodebuddyLocalAccessAccountHealth {
  accountId: string;
  email: string;
  /** 账号是否可用于调度（连续失败 / 鉴权异常 / 冷却未过期时为 false）。 */
  available?: boolean;
  /** 连续失败次数（成功后清零）。 */
  consecutiveFailures?: number;
  /** 最近一次失败时间戳（毫秒）。 */
  lastFailureAt?: number | null;
  /** 最近一次失败分类（上游错误码）。 */
  lastFailureCategory?: string | null;
  /** 当前生效的冷却列表。 */
  cooldowns?: CodebuddyLocalAccessAccountCooldown[];
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
