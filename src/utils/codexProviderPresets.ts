export interface CodexApiProviderPreset {
  id: string;
  name: string;
  baseUrls: string[];
  website?: string;
  apiKeyUrl?: string;
  isOfficial?: boolean;
  isPartner?: boolean;
}

export const CODEX_API_PROVIDER_CUSTOM_ID = 'custom';

export const CODEX_API_PROVIDER_PRESETS: readonly CodexApiProviderPreset[] = [
  {
    id: 'openai_official',
    name: 'OpenAI Official',
    baseUrls: ['https://api.openai.com/v1'],
    website: 'https://chatgpt.com/codex',
    apiKeyUrl: 'https://platform.openai.com/api-keys',
    isOfficial: true,
  },
  {
    id: 'azure_openai',
    name: 'Azure OpenAI',
    baseUrls: ['https://YOUR_RESOURCE_NAME.openai.azure.com/openai'],
    website: 'https://learn.microsoft.com/en-us/azure/ai-foundry/openai/how-to/codex',
    isOfficial: true,
  },
  {
    id: 'packycode',
    name: 'PackyCode',
    baseUrls: ['https://www.packyapi.com/v1', 'https://api-slb.packyapi.com/v1'],
    website: 'https://www.packyapi.com',
    apiKeyUrl: 'https://www.packyapi.com/register?aff=cc-switch',
    isPartner: true,
  },
  {
    id: 'cubence',
    name: 'Cubence',
    baseUrls: [
      'https://api.cubence.com/v1',
      'https://api-cf.cubence.com/v1',
      'https://api-dmit.cubence.com/v1',
      'https://api-bwg.cubence.com/v1',
    ],
    website: 'https://cubence.com',
    apiKeyUrl: 'https://cubence.com/signup?code=CCSWITCH&source=ccs',
    isPartner: true,
  },
  {
    id: 'aigocode',
    name: 'AIGoCode',
    baseUrls: ['https://api.aigocode.com'],
    website: 'https://aigocode.com',
    apiKeyUrl: 'https://aigocode.com/invite/CC-SWITCH',
    isPartner: true,
  },
  {
    id: 'rightcode',
    name: 'RightCode',
    baseUrls: ['https://right.codes/codex/v1'],
    website: 'https://www.right.codes',
    apiKeyUrl: 'https://www.right.codes/register?aff=CCSWITCH',
    isPartner: true,
  },
  {
    id: 'sssaicode',
    name: 'SSSAiCode',
    baseUrls: [
      'https://node-hk.sssaicode.com/api/v1',
      'https://claude2.sssaicode.com/api/v1',
      'https://anti.sssaicode.com/api/v1',
    ],
    website: 'https://www.sssaicode.com',
    apiKeyUrl: 'https://www.sssaicode.com/register?ref=DCP0SM',
    isPartner: true,
  },
  {
    id: 'micu',
    name: 'Micu',
    baseUrls: ['https://www.openclaudecode.cn/v1'],
    website: 'https://www.openclaudecode.cn',
    apiKeyUrl: 'https://www.openclaudecode.cn/register?aff=aOYQ',
    isPartner: true,
  },
  {
    id: 'x_code_api',
    name: 'X-Code API',
    baseUrls: ['https://x-code.cc/v1'],
    website: 'https://x-code.cc',
    apiKeyUrl: 'https://x-code.cc',
    isPartner: true,
  },
  {
    id: 'ctok_ai',
    name: 'CTok.ai',
    baseUrls: ['https://api.ctok.ai/v1'],
    website: 'https://ctok.ai',
    apiKeyUrl: 'https://ctok.ai',
    isPartner: true,
  },
  {
    id: 'aihubmix',
    name: 'AiHubMix',
    baseUrls: ['https://aihubmix.com/v1', 'https://api.aihubmix.com/v1'],
    website: 'https://aihubmix.com',
  },
  {
    id: 'dmxapi',
    name: 'DMXAPI',
    baseUrls: ['https://www.dmxapi.cn/v1'],
    website: 'https://www.dmxapi.cn',
    isPartner: true,
  },
  {
    id: 'compshare',
    name: '优云智算',
    baseUrls: ['https://api.modelverse.cn/v1'],
    website: 'https://www.compshare.cn',
    apiKeyUrl: 'https://www.compshare.cn/coding-plan?ytag=GPU_YY_YX_git_cc-switch',
    isPartner: true,
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    baseUrls: ['https://openrouter.ai/api/v1'],
    website: 'https://openrouter.ai/',
    apiKeyUrl: 'https://openrouter.ai/keys',
  },
  {
    id: 'aicodemirror',
    name: 'AICodeMirror',
    baseUrls: [
      'https://api.aicodemirror.com/api/codex/backend-api/codex',
      'https://api.claudecode.net.cn/api/codex/backend-api/codex',
    ],
    website: 'https://www.aicodemirror.com',
    apiKeyUrl: 'https://www.aicodemirror.com/register?invitecode=9915W3',
    isPartner: true,
  },
  {
    id: 'aicoding',
    name: 'AICoding',
    baseUrls: ['https://api.aicoding.sh'],
    website: 'https://aicoding.sh',
    apiKeyUrl: 'https://aicoding.sh/i/CCSWITCH',
    isPartner: true,
  },
  {
    id: 'crazyrouter',
    name: 'CrazyRouter',
    baseUrls: ['https://crazyrouter.com/v1'],
    website: 'https://www.crazyrouter.com',
    apiKeyUrl: 'https://www.crazyrouter.com/register?aff=OZcm&ref=cc-switch',
    isPartner: true,
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    baseUrls: ['https://api.deepseek.com/v1'],
    website: 'https://platform.deepseek.com/',
    apiKeyUrl: 'https://platform.deepseek.com/api_keys',
  },
  {
    id: 'moonshot',
    name: 'Moonshot',
    baseUrls: ['https://api.moonshot.cn/v1'],
    website: 'https://platform.moonshot.cn/',
  },
  {
    id: 'siliconflow',
    name: 'SiliconFlow',
    baseUrls: ['https://api.siliconflow.cn/v1'],
    website: 'https://cloud.siliconflow.cn/',
  },
];

function normalizeCodexProviderBaseUrl(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
    return `${parsed.origin}${parsed.pathname}`.replace(/\/+$/, '').toLowerCase();
  } catch {
    return null;
  }
}

export function findCodexApiProviderPresetById(
  id: string,
): CodexApiProviderPreset | null {
  return CODEX_API_PROVIDER_PRESETS.find((preset) => preset.id === id) ?? null;
}

export function findCodexApiProviderPresetByBaseUrl(
  rawBaseUrl: string,
): CodexApiProviderPreset | null {
  const normalized = normalizeCodexProviderBaseUrl(rawBaseUrl);
  if (!normalized) return null;

  return (
    CODEX_API_PROVIDER_PRESETS.find((preset) =>
      preset.baseUrls.some(
        (baseUrl) => normalizeCodexProviderBaseUrl(baseUrl) === normalized,
      ),
    ) ?? null
  );
}

export function resolveCodexApiProviderPresetId(rawBaseUrl: string): string {
  return (
    findCodexApiProviderPresetByBaseUrl(rawBaseUrl)?.id ??
    CODEX_API_PROVIDER_CUSTOM_ID
  );
}
