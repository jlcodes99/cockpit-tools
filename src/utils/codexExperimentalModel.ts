import type { TFunction } from 'i18next';

export function getCodexExperimentalModelErrorMessage(
  t: TFunction,
  error: unknown,
): string | null {
  const message = String(error);
  if (message.includes('EXPERIMENTAL_MODEL_CATALOG_MODELS_REQUIRED')) {
    return t(
      'codex.experimentalModelCatalog.models.validation.required',
      '至少保留一个模型。',
    );
  }
  if (message.includes('EXPERIMENTAL_MODEL_CATALOG_MODEL_ID_INVALID')) {
    return t(
      'codex.experimentalModelCatalog.models.validation.modelId',
      '模型 ID 只能包含字母、数字、点、横线、下划线、斜杠和冒号。',
    );
  }
  if (message.includes('EXPERIMENTAL_MODEL_CATALOG_DISPLAY_NAME_INVALID')) {
    return t(
      'codex.experimentalModelCatalog.models.validation.displayName',
      '请输入不超过 100 个字符的展示名。',
    );
  }
  if (message.includes('EXPERIMENTAL_MODEL_CATALOG_MODEL_ID_DUPLICATE')) {
    return t(
      'codex.experimentalModelCatalog.models.validation.duplicate',
      '模型 ID 不能重复。',
    );
  }
  if (message.includes('EXPERIMENTAL_MODEL_CATALOG_REASONING_EFFORT_INVALID')) {
    return t(
      'codex.experimentalModelCatalog.models.validation.reasoningEffort',
      '推理强度选项无效。',
    );
  }
  if (message.includes('EXPERIMENTAL_MODEL_CATALOG_CONTEXT_WINDOW_INVALID')) {
    return t(
      'codex.experimentalModelCatalog.models.validation.contextWindow',
      '上下文窗口必须是大于 0 的整数。',
    );
  }
  if (message.includes('EXPERIMENTAL_MODEL_CATALOG_AUTO_COMPACT_INVALID')) {
    return t(
      'codex.experimentalModelCatalog.models.validation.autoCompact',
      '压缩阈值必须是大于 0 的整数。',
    );
  }
  if (message.includes('EXPERIMENTAL_MODEL_CATALOG_AUTO_COMPACT_RANGE_INVALID')) {
    return t(
      'codex.experimentalModelCatalog.models.validation.autoCompactRange',
      '压缩阈值必须小于上下文窗口。',
    );
  }
  return null;
}
