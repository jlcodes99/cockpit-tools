import type { CodexExperimentalModelDefinition } from '../types/codex';
import type { CodexModelSourceMetadata } from '../services/codexModelCatalogSourceService';

type ContextValueSource = 'model' | 'global' | 'source' | 'builtin';

export function resolveEffectiveModelContext(
  model: CodexExperimentalModelDefinition,
  globalContextWindow?: number,
  globalAutoCompactTokenLimit?: number,
  sourceMetadata?: CodexModelSourceMetadata,
) {
  const contextWindow = model.context_window ?? globalContextWindow ?? sourceMetadata?.contextWindow;
  const autoCompactTokenLimit = model.auto_compact_token_limit ?? globalAutoCompactTokenLimit ?? sourceMetadata?.autoCompactTokenLimit;
  const contextSource: ContextValueSource = model.context_window !== undefined ? 'model'
    : globalContextWindow !== undefined ? 'global' : sourceMetadata?.contextWindow !== undefined ? 'source' : 'builtin';
  const compactSource: ContextValueSource = model.auto_compact_token_limit !== undefined ? 'model'
    : globalAutoCompactTokenLimit !== undefined ? 'global' : sourceMetadata?.autoCompactTokenLimit !== undefined ? 'source' : 'builtin';
  return { contextWindow, autoCompactTokenLimit, contextSource, compactSource };
}

/** 自动压缩阈值统一取上下文窗口的 90%（与 1M 档 1000000/900000 一致）。 */
export const AUTO_COMPACT_PERCENT = 90;

/** 按 90% 派生压缩阈值；上下文非法时返回 NaN。 */
export function deriveAutoCompactTokenLimit(contextWindow: number): number {
  if (!Number.isSafeInteger(contextWindow) || contextWindow <= 0) {
    return Number.NaN;
  }
  return Math.floor((contextWindow * AUTO_COMPACT_PERCENT) / 100);
}

/** 表单用：按 90% 派生压缩阈值，上下文非法时返回空串（不写死兜底值）。 */
export function deriveAutoCompactTokenLimitInput(contextWindow: string): string {
  const derived = deriveAutoCompactTokenLimit(Number(contextWindow.trim()));
  return Number.isSafeInteger(derived) && derived > 0 ? String(derived) : '';
}

/**
 * 读取已有配置时的归一化：只写了上下文、或压缩阈值缺失/非正/不小于上下文（含等于）
 * 时，按 90% 派生；上下文非法时保留原样，交给上层校验报错。
 */
export function resolveStoredCompactLimitInput(
  contextWindow: number | null | undefined,
  compactLimit: number | null | undefined,
): string {
  const context = contextWindow ?? Number.NaN;
  const fallback = compactLimit === null || compactLimit === undefined ? '' : String(compactLimit);
  if (!Number.isSafeInteger(context) || context <= 0) {
    return fallback;
  }
  if (
    compactLimit === null ||
    compactLimit === undefined ||
    !Number.isSafeInteger(compactLimit) ||
    compactLimit <= 0 ||
    compactLimit >= context
  ) {
    return deriveAutoCompactTokenLimitInput(String(context));
  }
  return String(compactLimit);
}

export function validateModelContext(model: Pick<CodexExperimentalModelDefinition,
  'context_window' | 'auto_compact_token_limit'>): string | null {
  const context = model.context_window;
  const compact = model.auto_compact_token_limit;
  const prefix = 'codex.experimentalModelCatalog.models.validation.';
  if (context === undefined && compact === undefined) return null;
  if (context !== undefined && (!Number.isSafeInteger(context) || context <= 0)) return prefix + 'contextWindow';
  if (compact !== undefined && (!Number.isSafeInteger(compact) || compact <= 0)) return prefix + 'autoCompact';
  if (context !== undefined && compact !== undefined && compact >= context) return prefix + 'autoCompactRange';
  return null;
}

export function validateEffectiveModelContexts(
  models: CodexExperimentalModelDefinition[],
  globalContextWindow?: number,
  globalAutoCompactTokenLimit?: number,
  sourceMetadata?: Record<string, CodexModelSourceMetadata>,
): string | null {
  for (const model of models) {
    const { contextWindow: window, autoCompactTokenLimit: limit } = resolveEffectiveModelContext(
      model, globalContextWindow, globalAutoCompactTokenLimit, sourceMetadata?.[model.model_id],
    );
    if (window !== undefined && limit !== undefined && limit >= window) {
      return `模型 ${model.model_id} 的自动压缩阈值 ${limit} 必须小于上下文窗口 ${window}`;
    }
  }
  return null;
}
