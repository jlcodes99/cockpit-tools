import type { CodexProviderWireApi } from './codexProviderGateway';

// Shared by wakeup defaults and provider test previews.
export const DEFAULT_WAKEUP_MODEL = 'gpt-5.6-luna';

const RESPONSES_NATIVE_CHAT_TEST_MODEL_PRIORITY = [
  DEFAULT_WAKEUP_MODEL,
  'gpt-6.1-sol',
  'gpt-6-astra',
  'gpt-6-sol',
  'gpt-6-luna',
  'gpt-5.6-sol',
  'gpt-5.6-terra',
  'gpt-5.5',
];

export function isImageGenerationModelId(modelId: string): boolean {
  const lower = modelId.trim().toLowerCase();
  return lower.startsWith('gpt-image') || lower.startsWith('dall-e') || lower.includes('image-gen');
}

export function selectProviderBatchTestModelId(
  wireApi: CodexProviderWireApi,
  modelCatalog?: string[] | null,
): string | null {
  const models = (modelCatalog ?? []).map((item) => item.trim()).filter(Boolean);
  if (wireApi === 'responses') {
    for (const preferred of RESPONSES_NATIVE_CHAT_TEST_MODEL_PRIORITY) {
      const model = models.find((item) => item.toLowerCase() === preferred);
      if (model) return model;
    }
    const textModel = models.find((item) => !isImageGenerationModelId(item));
    if (textModel) return textModel;
  }
  return models[0] ?? null;
}
