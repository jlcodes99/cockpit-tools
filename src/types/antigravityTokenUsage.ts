export type ConversationKind = 'main' | 'swarm' | 'subagent';

export type DataSourceKind = 'sqlite_db' | 'rpc_cache' | 'transcript_jsonl' | 'summary_only';

export type TimeGranularity = 'day' | 'week' | 'month';

export type DatePreset = 'today' | '7d' | '30d' | 'month' | 'all' | 'custom';

export type AntigravityUsageRangePreset = 'today' | '7d' | '30d' | 'month' | 'all';

export type ConversationSortField =
  | 'lastModifiedTime'
  | 'totalTokens'
  | 'inputTokens'
  | 'outputTokens'
  | 'combinedOutputTokens'
  | 'thinkingTokens'
  | 'cacheReadTokens'
  | 'stepCount'
  | 'turnCount'
  | 'title'
  | 'kind'
  | 'workspace'
  | 'model';

export type SortDirection = 'asc' | 'desc';

export interface ExactTokenTotals {
  inputTokens: number;
  outputTokens: number;
  thinkingTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
}

export interface TurnEstimatedTotals {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
}

export interface EstimatedTokenTotals {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
  estimatedTurnCount: number;
}

export interface TurnUsageRecord {
  stepIndex: number;
  timestamp: string | null;
  modelId: string;
  modelDisplayName: string;
  rawModelEnum?: string;
  responseModel?: string;
  apiProvider: string;
  isExact: boolean;
  source: DataSourceKind;
  status: number;
  exact: ExactTokenTotals;
  estimated: TurnEstimatedTotals;
  messageId?: string;
}

export interface ConversationModelSlice {
  modelId: string;
  modelDisplayName: string;
  apiProvider: string;
  turnCount: number;
  exact: ExactTokenTotals;
  estimatedTotalTokens: number;
}

export interface ChildConversationRef {
  conversationId: string;
  title: string;
  agentName: string;
  kind: ConversationKind;
  exactTotalTokens: number;
  estimatedTotalTokens: number;
}

export interface ConversationSummaryItem {
  conversationId: string;
  title: string;
  displayTitle: string;
  preview: string;
  kind: ConversationKind;
  hasSwarmChildren: boolean;
  agentName: string;
  parentConversationId: string;
  parentTitle: string;
  nestingDepth: number;
  workspaceUri: string;
  workspacePath: string;
  workspaceName: string;
  projectId: string;
  status: string;
  createdAt: string | null;
  lastModifiedTime: string;
  stepCount: number;
  turnCount: number;
  exactTurnCount: number;
  estimatedTurnCount: number;
  primaryModelId: string;
  primaryModelDisplayName: string;
  models: string[];
  modelDisplayNames: string[];
  dataSource: DataSourceKind;
  exact: ExactTokenTotals;
  estimated: EstimatedTokenTotals;
}

export interface ConversationDetail extends ConversationSummaryItem {
  modelBreakdown: ConversationModelSlice[];
  turns: TurnUsageRecord[];
  children: ChildConversationRef[];
}

export interface ModelUsageSummary {
  modelId: string;
  modelDisplayName: string;
  apiProvider: string;
  conversationCount: number;
  turnCount: number;
  exactTurnCount: number;
  estimatedTurnCount: number;
  exact: ExactTokenTotals;
  estimatedTotalTokens: number;
  sharePercent: number;
}

export interface WorkspaceUsageSummary {
  workspaceKey: string;
  workspacePath: string;
  workspaceName: string;
  conversationCount: number;
  turnCount: number;
  exact: ExactTokenTotals;
  estimatedTotalTokens: number;
  sharePercent: number;
  lastActiveTime: string | null;
}

export interface KindUsageSummary {
  kind: ConversationKind;
  conversationCount: number;
  turnCount: number;
  exact: ExactTokenTotals;
  estimatedTotalTokens: number;
  sharePercent: number;
}

export interface TimeBucketPoint {
  bucketKey: string;
  label: string;
  startDate: string;
  endDate: string;
  turnCount: number;
  conversationCount: number;
  exact: ExactTokenTotals;
  estimatedTotalTokens: number;
}

export interface ScanMetadata {
  dataDir: string;
  dataDirExists: boolean;
  scannedAt: string;
  scanDurationMs: number;
  totalDbFiles: number;
  cacheHits: number;
  cacheMisses: number;
  rpcCacheSessionsFound: number;
  brainDirsFound: number;
  warnings: string[];
}

export interface CatalogModelOption {
  modelId: string;
  modelDisplayName: string;
  count: number;
}

export interface CatalogWorkspaceOption {
  workspaceKey: string;
  workspaceName: string;
  workspacePath: string;
  count: number;
}

export interface FilterOptionsCatalog {
  models: CatalogModelOption[];
  workspaces: CatalogWorkspaceOption[];
  minDate: string | null;
  maxDate: string | null;
}

export interface KpiSummary {
  exact: ExactTokenTotals;
  estimated: EstimatedTokenTotals;
  totalConversations: number;
  mainConversations: number;
  swarmConversations: number;
  subagentConversations: number;
  totalSteps: number;
  totalTurns: number;
  exactTurns: number;
  estimatedTurns: number;
  exactCoveragePercent: number;
  activeDays: number;
  avgExactTokensPerActiveDay: number;
  avgExactTokensPerConversation: number;
}

export interface AppliedFilters {
  preset: DatePreset;
  from: string | null;
  to: string | null;
  granularity: TimeGranularity;
  model: string;
  workspace: string;
  kind: string;
  search: string;
  tzOffsetMinutes: number;
}

export interface DashboardSummaryResponse {
  scan: ScanMetadata;
  filters: AppliedFilters;
  catalog: FilterOptionsCatalog;
  kpi: KpiSummary;
  timeSeries: TimeBucketPoint[];
  byModel: ModelUsageSummary[];
  byWorkspace: WorkspaceUsageSummary[];
  byKind: KindUsageSummary[];
}

export interface ConversationListResponse {
  items: ConversationSummaryItem[];
  totalItems: number;
  page: number;
  pageSize: number;
  totalPages: number;
  sort: ConversationSortField;
  order: SortDirection;
}

export interface ConversationDetailResponse {
  scan: ScanMetadata;
  conversation: ConversationDetail;
}

export interface AntigravityTokenUsageSettings {
  customDataDir: string | null;
  effectiveDataDir: string;
  defaultDataDir: string;
  isCustomDataDir: boolean;
}

export interface DashboardQueryRequest {
  preset?: string;
  from?: string;
  to?: string;
  granularity?: string;
  model?: string;
  workspace?: string;
  kind?: string;
  search?: string;
  tzOffsetMinutes?: number;
  sort?: string;
  order?: string;
  page?: number;
  pageSize?: number;
}

export interface DashboardBundleResponse {
  summary: DashboardSummaryResponse;
  conversations: ConversationListResponse;
  settings: AntigravityTokenUsageSettings;
}
