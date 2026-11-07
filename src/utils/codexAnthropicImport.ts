export interface CodexAnthropicImport {
  baseUrl: string;
  apiKey: string;
  authScheme: 'x-api-key' | 'bearer';
  models: string[];
}

/** Explicit clipboard import of Claude Code settings or Desktop Gateway config. */
export function parseCodexAnthropicImport(raw: string): CodexAnthropicImport | null {
  let config: Record<string, unknown>;
  try {
    const value: unknown = JSON.parse(raw);
    if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
    config = value as Record<string, unknown>;
  } catch { return null; }
  const env = config.env && typeof config.env === 'object' && !Array.isArray(config.env)
    ? config.env as Record<string, unknown> : config;
  const text = (value: unknown) => typeof value === 'string' ? value.trim() : '';
  const baseUrl = text(config.inferenceGatewayBaseUrl) || text(env.ANTHROPIC_BASE_URL);
  const apiToken = text(env.ANTHROPIC_AUTH_TOKEN);
  const nativeKey = text(env.ANTHROPIC_API_KEY);
  const apiKey = text(config.inferenceGatewayApiKey) || nativeKey || apiToken;
  if (!baseUrl || !apiKey) return null;
  try {
    const parsed = new URL(baseUrl);
    if (!['https:', 'http:'].includes(parsed.protocol) || parsed.username || parsed.password || parsed.search || parsed.hash) return null;
  } catch { return null; }
  const authScheme = text(config.inferenceGatewayAuthScheme)
    || (nativeKey ? 'x-api-key' : apiToken ? 'bearer' : 'x-api-key');
  const models = [
    env.ANTHROPIC_MODEL, env.ANTHROPIC_DEFAULT_OPUS_MODEL,
    env.ANTHROPIC_DEFAULT_SONNET_MODEL, env.ANTHROPIC_DEFAULT_HAIKU_MODEL,
    ...(Array.isArray(config.inferenceModels) ? config.inferenceModels.map(model =>
      typeof model === 'string' ? model : model && typeof model === 'object' ? (model as Record<string, unknown>).name : '') : []),
  ].map(text).filter(Boolean);
  return { baseUrl, apiKey, authScheme: authScheme === 'bearer' ? 'bearer' : 'x-api-key', models: [...new Set(models)] };
}
