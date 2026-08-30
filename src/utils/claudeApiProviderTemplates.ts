import type { ClaudeApiProviderPreset } from './claudeProviderPresets';

/** 根据 Provider 预设生成表单初始值，统一处理编辑值与默认值的优先级。 */
export function getClaudeApiProviderTemplateInitialValues(
  preset?: ClaudeApiProviderPreset | null,
): Record<string, string> {
  return Object.fromEntries(
    Object.entries(preset?.templateValues ?? {}).map(([key, config]) => [
      key,
      config.editorValue ?? config.defaultValue ?? '',
    ]),
  );
}

/** 将 Provider 配置中的环境变量占位符替换为当前表单值。 */
export function applyClaudeApiProviderTemplateValue(
  value: string,
  templateValues: Record<string, string>,
): string {
  return value.replace(/\$\{([A-Z0-9_]+)\}/g, (matched, key: string) => templateValues[key] ?? matched);
}

/** 解析 Provider 预设附加环境变量，供账号保存和启动配置共用。 */
export function resolveClaudeApiProviderExtraEnv(
  preset: ClaudeApiProviderPreset | null | undefined,
  templateValues: Record<string, string>,
): Record<string, string> | null {
  const entries = Object.entries(preset?.extraEnv ?? {}).map(([key, value]) => [
    key,
    applyClaudeApiProviderTemplateValue(value, templateValues),
  ]);
  return entries.length > 0 ? Object.fromEntries(entries) : null;
}
