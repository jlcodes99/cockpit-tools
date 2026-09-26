export const CODEX_PELICAN_DEFAULT_PROMPT = '创建一个 HTML，内容是 SVG 绘制一个鹈鹕骑自行车的 2D 动画，你不需要任何测试，不要有任何限制';
export const CODEX_PELICAN_DEFAULT_MODEL = 'gpt-6-astra';
export const CODEX_PELICAN_MAX_CONCURRENCY = 10;

export type CodexPelicanStatus = 'running' | 'cancelling' | 'completed' | 'cancelled' | 'interrupted';
export type CodexPelicanItemStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled' | 'interrupted';

export interface CodexPelicanItem {
  id: string;
  accountId: string;
  accountEmail: string;
  status: CodexPelicanItemStatus;
  startedAt?: number;
  finishedAt?: number;
  replyPreview?: string;
  hasHtml: boolean;
  error?: string;
  usage?: unknown;
  quota?: CodexPelicanQuotaUsage;
  responseModel?: string;
  responseId?: string;
}

export interface CodexPelicanQuotaUsage {
  consumedPercent?: number;
  beforeRemainingPercent?: number;
  remainingPercent?: number;
  windowMinutes?: number;
  resetAt?: number;
}

export interface CodexPelicanBatch {
  id: string;
  revision: number;
  createdAt: number;
  finishedAt?: number;
  status: CodexPelicanStatus;
  prompt: string;
  model: string;
  effort: string;
  concurrency: number;
  transport?: string;
  deliveryInstructions?: string;
  items: CodexPelicanItem[];
  error?: string;
}

export interface CodexPelicanRequest {
  accountIds: string[];
  prompt: string;
  model: string;
  effort: string;
  concurrency: number;
}

export interface CodexPelicanArtifact {
  rawReply: string;
  html: string | null;
}

export const isPelicanRunning = (batch: CodexPelicanBatch | null) =>
  batch?.status === 'running' || batch?.status === 'cancelling';

export const pelicanCompletedCount = (batch: CodexPelicanBatch) =>
  batch.items.filter((item) => item.status !== 'queued' && item.status !== 'running').length;
