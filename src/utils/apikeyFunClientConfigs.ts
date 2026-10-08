/**
 * APIKEY.FUN「使用 API 密钥」弹窗的配置模板。
 *
 * 模板按官网弹窗（2026-09-21 采集，见 docs/apikey-fun-client-configs.md）整理：
 * 分组 → 默认模型 / 上下文 / Base URL，客户端 → 写入哪个文件。
 * 只保留我们已经支持的一键落盘客户端：Codex App / Codex CLI / Claude Code / Claude Desktop。
 */

export type ApiKeyFunFamily =
  | "gemini"
  | "grok"
  | "glm"
  | "kimi"
  | "deepseek"
  | "codex"
  | "claude";

export type ApiKeyFunClientId =
  | "codex_app"
  | "codex_cli"
  | "claude_code"
  | "claude_desktop";

export type ApiKeyFunPlatform = "macos" | "windows" | "linux";

export interface ApiKeyFunConfigBlock {
  /** 文件路径或终端标签。 */
  label: string;
  language: "toml" | "json" | "bash" | "powershell" | "bat" | "text";
  text: string;
}

export interface ApiKeyFunFamilyPreset {
  id: ApiKeyFunFamily;
  /** 品牌名，不翻译。 */
  label: string;
  /** 用真实模型列表判断分组的前缀。 */
  prefixes: readonly string[];
  /** 从真实模型列表里优先挑选的默认模型。 */
  preferredModels: readonly string[];
  /** 官网模板给出的默认模型（真实列表里没有时兜底）。 */
  fallbackModel: string;
  contextWindow: number;
  /** Codex 的 base_url：GPT 组用不带 /v1 的地址。 */
  codexBaseUrl: string;
  /** 官网在 GPT 组额外注入的请求头。 */
  codexHttpHeaders?: Record<string, string>;
  /** 官网在 GPT 组写入的 [features]。 */
  codexFeatureGoals?: boolean;
  /** 支持一键落盘的客户端。 */
  clients: readonly ApiKeyFunClientId[];
}

export const APIKEY_FUN_FAMILIES: readonly ApiKeyFunFamilyPreset[] = [
  {
    id: "gemini",
    label: "Gemini",
    prefixes: ["gemini-"],
    preferredModels: ["gemini-3-pro-preview", "gemini-2.5-pro", "gemini-2.5-flash"],
    fallbackModel: "gemini-2.0-flash",
    contextWindow: 1048576,
    codexBaseUrl: "https://api.apikey.fan/v1",
    clients: [],
  },
  {
    id: "grok",
    label: "Grok",
    prefixes: ["grok-", "grok"],
    preferredModels: ["grok-4.6", "grok-4.3", "grok-build-0.1"],
    fallbackModel: "grok-4.6",
    contextWindow: 500000,
    codexBaseUrl: "https://api.apikey.fan/v1",
    clients: ["codex_app", "codex_cli", "claude_code", "claude_desktop"],
  },
  {
    id: "glm",
    label: "智谱 GLM",
    prefixes: ["glm-", "zhipu-"],
    preferredModels: ["glm-5.2", "glm-5.1", "glm-4.6"],
    fallbackModel: "glm-5.2",
    contextWindow: 1000000,
    codexBaseUrl: "https://api.apikey.fan/v1",
    clients: ["codex_app", "codex_cli", "claude_code"],
  },
  {
    id: "kimi",
    label: "Kimi",
    prefixes: ["kimi-", "moonshot-"],
    preferredModels: ["kimi-k3", "kimi-k2.6"],
    fallbackModel: "kimi-k3",
    contextWindow: 1000000,
    codexBaseUrl: "https://api.apikey.fan/v1",
    clients: ["codex_app", "codex_cli", "claude_code"],
  },
  {
    id: "deepseek",
    label: "DeepSeek",
    prefixes: ["deepseek-"],
    preferredModels: ["deepseek-v4-pro", "deepseek-v4-flash", "deepseek-chat"],
    fallbackModel: "deepseek-v4-pro",
    contextWindow: 1000000,
    codexBaseUrl: "https://api.apikey.fan/v1",
    clients: ["codex_app", "codex_cli", "claude_code"],
  },
  {
    id: "codex",
    label: "ChatGPT",
    prefixes: ["gpt-", "o1", "o3", "o4", "codex-"],
    preferredModels: ["gpt-5.6-sol", "gpt-5.6-luna", "gpt-5.5"],
    fallbackModel: "gpt-5.6-sol",
    contextWindow: 272000,
    codexBaseUrl: "https://api.apikey.fan",
    codexHttpHeaders: { "x-openai-actor-authorization": "apikey.fan" },
    codexFeatureGoals: true,
    clients: ["codex_app", "codex_cli"],
  },
  {
    id: "claude",
    label: "Claude",
    prefixes: ["claude-"],
    preferredModels: ["claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-5"],
    fallbackModel: "claude-opus-4-6",
    contextWindow: 200000,
    codexBaseUrl: "https://api.apikey.fan",
    clients: ["claude_code", "claude_desktop"],
  },
] as const;

export const APIKEY_FUN_ANTHROPIC_BASE_URL = "https://api.apikey.fan";

/** 客户端展示名（品牌名，不翻译）。 */
export const APIKEY_FUN_CLIENT_LABELS: Record<ApiKeyFunClientId, string> = {
  codex_app: "Codex App",
  codex_cli: "Codex CLI",
  claude_code: "Claude Code",
  claude_desktop: "Claude Desktop",
};

export function detectApiKeyFunPlatform(): ApiKeyFunPlatform {
  const ua = typeof navigator === "undefined" ? "" : navigator.userAgent;
  if (/Windows/i.test(ua)) return "windows";
  if (/Mac OS X|Macintosh/i.test(ua)) return "macos";
  return "linux";
}

/** 按真实模型列表判断分组：取命中前缀最多的一组。 */
export function resolveApiKeyFunFamily(models: readonly string[]): ApiKeyFunFamilyPreset | null {
  const counts = new Map<ApiKeyFunFamily, number>();
  for (const raw of models) {
    const model = raw.trim().toLowerCase().split("/").pop() ?? "";
    if (!model) continue;
    for (const preset of APIKEY_FUN_FAMILIES) {
      if (preset.prefixes.some((prefix) => model.startsWith(prefix))) {
        counts.set(preset.id, (counts.get(preset.id) ?? 0) + 1);
      }
    }
  }
  const top = [...counts.entries()].sort((left, right) => right[1] - left[1])[0];
  if (!top) return null;
  return APIKEY_FUN_FAMILIES.find((preset) => preset.id === top[0]) ?? null;
}

export function resolveDefaultModel(
  preset: ApiKeyFunFamilyPreset,
  models: readonly string[],
): string {
  const normalized = models.map((model) => model.trim()).filter(Boolean);
  for (const preferred of preset.preferredModels) {
    const hit = normalized.find((model) => model.toLowerCase() === preferred);
    if (hit) return hit;
  }
  const byPrefix = normalized.find((model) => {
    const key = model.toLowerCase().split("/").pop() ?? "";
    return preset.prefixes.some((prefix) => key.startsWith(prefix));
  });
  return byPrefix ?? normalized[0] ?? preset.fallbackModel;
}

function escapeToml(value: string): string {
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

/** Codex 的 config.toml（Codex App 与 Codex CLI 只在 WebSocket 开关上有差别）。 */
export function buildCodexConfigToml(input: {
  family: ApiKeyFunFamilyPreset;
  client: "codex_app" | "codex_cli";
  model: string;
}): string {
  const { family, client, model } = input;
  const supportsWebsockets = client === "codex_app" && family.id === "codex";
  const lines: string[] = [
    `model_provider = "codex"`,
    `model = "${escapeToml(model)}"`,
    `review_model = "${escapeToml(model)}"`,
  ];
  if (family.id === "codex" || family.id === "grok") {
    lines.push(`model_reasoning_effort = "high"`);
  }
  // 压缩阈值统一按上下文窗口的 90% 派生；等于窗口值会导致永不触发压缩。
  const autoCompactTokenLimit = Math.floor((family.contextWindow * 90) / 100);
  lines.push(
    "disable_response_storage = true",
    'network_access = "enabled"',
    "windows_wsl_setup_acknowledged = true",
    `model_context_window = ${family.contextWindow}`,
    `model_auto_compact_token_limit = ${autoCompactTokenLimit}`,
    "effective_context_window_percent = 95",
    "",
    "[model_providers.codex]",
    'name = "codex"',
    `base_url = "${escapeToml(family.codexBaseUrl)}"`,
    'wire_api = "responses"',
    "supports_websockets = " + (supportsWebsockets ? "true" : "false"),
    "requires_openai_auth = true",
  );
  if (family.codexHttpHeaders) {
    const pairs = Object.entries(family.codexHttpHeaders)
      .map(([key, value]) => `"${escapeToml(key)}" = "${escapeToml(value)}"`)
      .join(", ");
    lines.push(`http_headers = { ${pairs} }`);
  }
  const features: string[] = [];
  if (supportsWebsockets) features.push("responses_websockets_v2 = true");
  if (family.codexFeatureGoals) features.push("goals = true");
  if (features.length > 0) {
    lines.push("", "[features]", ...features);
  }
  return lines.join("\n");
}

export function buildClaudeCodeSettingsJson(input: {
  apiKey: string;
  extraEnv?: Record<string, string>;
}): { json: string; env: Record<string, string> } {
  const env: Record<string, string> = {
    ANTHROPIC_BASE_URL: APIKEY_FUN_ANTHROPIC_BASE_URL,
    ANTHROPIC_AUTH_TOKEN: input.apiKey,
    CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: "1",
    CLAUDE_CODE_ATTRIBUTION_HEADER: "0",
    ...(input.extraEnv ?? {}),
  };
  const json = JSON.stringify(
    {
      $schema: "https://json.schemastore.org/claude-code-settings.json",
      env,
    },
    null,
    2,
  );
  return { json, env };
}

export function buildShellEnvCommands(
  env: Record<string, string>,
  platform: ApiKeyFunPlatform,
): string {
  const entries = Object.entries(env);
  if (platform === "windows") {
    return [
      ...entries.map(([key, value]) => `set ${key}=${value}`),
      "# 或 PowerShell：",
      ...entries.map(([key, value]) => `$env:${key}="${value}"`),
    ].join("\n");
  }
  return entries.map(([key, value]) => `export ${key}="${value}"`).join("\n");
}

export interface ApiKeyFunClientPreview {
  client: ApiKeyFunClientId;
  /** 可一键落盘的写入类型；claude_desktop 没有落盘能力。 */
  applyKind: "codex" | "claude_code" | null;
  blocks: ApiKeyFunConfigBlock[];
}

/** 生成某个客户端在当前分组下的配置预览。 */
export function buildApiKeyFunClientPreview(input: {
  family: ApiKeyFunFamilyPreset;
  client: ApiKeyFunClientId;
  apiKey: string;
  model: string;
  platform: ApiKeyFunPlatform;
}): ApiKeyFunClientPreview {
  const { family, client, apiKey, model, platform } = input;
  if (client === "codex_app" || client === "codex_cli") {
    const scope = platform === "windows" ? "C:\\Users\\<你>\\.codex" : "~/.codex";
    return {
      client,
      applyKind: "codex",
      blocks: [
        {
          label: `${scope}/config.toml`,
          language: "toml",
          text: buildCodexConfigToml({ family, client, model }),
        },
        {
          label: `${scope}/auth.json`,
          language: "json",
          text: JSON.stringify({ OPENAI_API_KEY: apiKey }, null, 2),
        },
      ],
    };
  }
  if (client === "claude_code") {
    const scope = platform === "windows" ? "%USERPROFILE%\\.claude" : "~/.claude";
    const { json, env } = buildClaudeCodeSettingsJson({ apiKey });
    return {
      client,
      applyKind: "claude_code",
      blocks: [
        {
          label: platform === "windows" ? "Command Prompt / PowerShell" : "Terminal",
          language: platform === "windows" ? "bat" : "bash",
          text: buildShellEnvCommands(env, platform),
        },
        {
          label: `${scope}/settings.json`,
          language: "json",
          text: json,
        },
      ],
    };
  }
  return {
    client,
    applyKind: null,
    blocks: [
      {
        label: "Gateway Base URL",
        language: "text",
        text: APIKEY_FUN_ANTHROPIC_BASE_URL,
      },
      { label: "Gateway API Key", language: "text", text: apiKey },
    ],
  };
}

/** 无可用模型列表时的兜底：按分组给一个默认模型。 */
export function resolveFallbackModel(
  family: ApiKeyFunFamilyPreset,
  models: readonly string[],
): string {
  return resolveDefaultModel(family, models);
}
