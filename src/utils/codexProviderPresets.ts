export interface CodexApiProviderPreset {
  id: string;
  name: string;
  baseUrls: string[];
  modelCatalog?: string[];
  website?: string;
  apiKeyUrl?: string;
  isOfficial?: boolean;
  isPartner?: boolean;
  isService?: boolean;
}

export const CODEX_API_PROVIDER_CUSTOM_ID = "custom";
export const COCKPIT_API_PROVIDER_ID = "cockpit_api";
export const COCKPIT_API_PROVIDER_NAME = "Cockpit Api";
export const COCKPIT_API_BASE_URL = "https://chongcodex.cn/v1";

const COCKPIT_API_HIDDEN_BASE_URLS = [COCKPIT_API_BASE_URL] as const;

export const CODEX_API_PROVIDER_PRESETS: readonly CodexApiProviderPreset[] = [
  {
    id: "openai_official",
    name: "OpenAI Official",
    baseUrls: ["https://api.openai.com/v1"],
    website: "https://chatgpt.com/codex",
    apiKeyUrl: "https://platform.openai.com/api-keys",
    isOfficial: true,
  },
  {
    id: "azure_openai",
    name: "Azure OpenAI",
    baseUrls: ["https://YOUR_RESOURCE_NAME.openai.azure.com/openai"],
    website:
      "https://learn.microsoft.com/en-us/azure/ai-foundry/openai/how-to/codex",
    isOfficial: true,
  },
  {
    id: "packycode",
    name: "PackyCode",
    baseUrls: [
      "https://www.packyapi.com/v1",
      "https://api-slb.packyapi.com/v1",
    ],
    website: "https://www.packyapi.com",
    apiKeyUrl: "https://www.packyapi.com/register?aff=cc-switch",
    isPartner: true,
  },
  {
    id: "cubence",
    name: "Cubence",
    baseUrls: [
      "https://api.cubence.com/v1",
      "https://api-cf.cubence.com/v1",
      "https://api-dmit.cubence.com/v1",
      "https://api-bwg.cubence.com/v1",
    ],
    website: "https://cubence.com",
    apiKeyUrl: "https://cubence.com/signup?code=CCSWITCH&source=ccs",
    isPartner: true,
  },
  {
    id: "aigocode",
    name: "AIGoCode",
    baseUrls: ["https://api.aigocode.com"],
    website: "https://aigocode.com",
    apiKeyUrl: "https://aigocode.com/invite/CC-SWITCH",
    isPartner: true,
  },
  {
    id: "rightcode",
    name: "RightCode",
    baseUrls: ["https://right.codes/codex/v1"],
    website: "https://www.right.codes",
    apiKeyUrl: "https://www.right.codes/register?aff=CCSWITCH",
    isPartner: true,
  },
  {
    id: "sssaicode",
    name: "SSSAiCode",
    baseUrls: [
      "https://node-hk.sssaicode.com/api/v1",
      "https://claude2.sssaicode.com/api/v1",
      "https://anti.sssaicode.com/api/v1",
    ],
    website: "https://www.sssaicode.com",
    apiKeyUrl: "https://www.sssaicode.com/register?ref=DCP0SM",
    isPartner: true,
  },
  {
    id: "micu",
    name: "Micu",
    baseUrls: ["https://www.openclaudecode.cn/v1"],
    website: "https://www.openclaudecode.cn",
    apiKeyUrl: "https://www.openclaudecode.cn/register?aff=aOYQ",
    isPartner: true,
  },
  {
    id: "x_code_api",
    name: "X-Code API",
    baseUrls: ["https://x-code.cc/v1"],
    website: "https://x-code.cc",
    apiKeyUrl: "https://x-code.cc",
    isPartner: true,
  },
  {
    id: "ctok_ai",
    name: "CTok.ai",
    baseUrls: ["https://api.ctok.ai/v1"],
    website: "https://ctok.ai",
    apiKeyUrl: "https://ctok.ai",
    isPartner: true,
  },
  {
    id: "aihubmix",
    name: "AiHubMix",
    baseUrls: ["https://aihubmix.com/v1", "https://api.aihubmix.com/v1"],
    website: "https://aihubmix.com",
  },
  {
    id: "dmxapi",
    name: "DMXAPI",
    baseUrls: ["https://www.dmxapi.cn/v1"],
    website: "https://www.dmxapi.cn",
    isPartner: true,
  },
  {
    id: "compshare",
    name: "优云智算",
    baseUrls: ["https://api.modelverse.cn/v1"],
    website: "https://www.compshare.cn",
    apiKeyUrl:
      "https://www.compshare.cn/coding-plan?ytag=GPU_YY_YX_git_cc-switch",
    isPartner: true,
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    baseUrls: ["https://openrouter.ai/api/v1"],
    website: "https://openrouter.ai/",
    apiKeyUrl: "https://openrouter.ai/keys",
  },
  {
    id: "aicodemirror",
    name: "AICodeMirror",
    baseUrls: [
      "https://api.aicodemirror.com/api/codex/backend-api/codex",
      "https://api.claudecode.net.cn/api/codex/backend-api/codex",
    ],
    website: "https://www.aicodemirror.com",
    apiKeyUrl: "https://www.aicodemirror.com/register?invitecode=9915W3",
    isPartner: true,
  },
  {
    id: "aicoding",
    name: "AICoding",
    baseUrls: ["https://api.aicoding.sh"],
    website: "https://aicoding.sh",
    apiKeyUrl: "https://aicoding.sh/i/CCSWITCH",
    isPartner: true,
  },
  {
    id: "crazyrouter",
    name: "CrazyRouter",
    baseUrls: ["https://crazyrouter.com/v1"],
    website: "https://www.crazyrouter.com",
    apiKeyUrl: "https://www.crazyrouter.com/register?aff=OZcm&ref=cc-switch",
    isPartner: true,
  },
  {
    id: "deepseek",
    name: "DeepSeek",
    baseUrls: ["https://api.deepseek.com", "https://api.deepseek.com/v1"],
    modelCatalog: ["deepseek-v4-pro", "deepseek-v4-flash"],
    website: "https://platform.deepseek.com/",
    apiKeyUrl: "https://platform.deepseek.com/api_keys",
  },
  {
    id: "moonshot",
    name: "Moonshot",
    baseUrls: ["https://api.moonshot.cn/v1"],
    modelCatalog: ["kimi-k2.6"],
    website: "https://platform.moonshot.cn/",
  },
  {
    id: "siliconflow",
    name: "SiliconFlow",
    baseUrls: ["https://api.siliconflow.cn/v1"],
    website: "https://cloud.siliconflow.cn/",
  },
  {
    id: "siliconflow_en",
    name: "SiliconFlow en",
    baseUrls: ["https://api.siliconflow.com/v1"],
    website: "https://siliconflow.com/",
  },
  {
    id: "zhipu_glm",
    name: "Zhipu GLM",
    baseUrls: ["https://open.bigmodel.cn/api/coding/paas/v4"],
    modelCatalog: ["glm-5.1"],
    website: "https://open.bigmodel.cn",
    apiKeyUrl: "https://www.bigmodel.cn/claude-code?ic=RRVJPB5SII",
  },
  {
    id: "zhipu_glm_en",
    name: "Zhipu GLM en",
    baseUrls: ["https://api.z.ai/api/coding/paas/v4"],
    modelCatalog: ["glm-5.1"],
    website: "https://z.ai",
    apiKeyUrl: "https://z.ai/subscribe?ic=8JVLJQFSKB",
  },
  {
    id: "volcengine_agentplan",
    name: "火山Agentplan",
    baseUrls: ["https://ark.cn-beijing.volces.com/api/coding/v3"],
    website: "https://www.volcengine.com/product/ark",
  },
  {
    id: "byteplus",
    name: "BytePlus",
    baseUrls: ["https://ark.ap-southeast.bytepluses.com/api/coding/v3"],
    website: "https://www.byteplus.com/en/product/ark",
  },
  {
    id: "doubaoseed",
    name: "DouBaoSeed",
    baseUrls: ["https://ark.cn-beijing.volces.com/api/v3"],
    website: "https://www.volcengine.com/product/ark",
  },
  {
    id: "qianfan_coding",
    name: "Baidu Qianfan Coding Plan",
    baseUrls: ["https://qianfan.baidubce.com/v2/coding"],
    website: "https://qianfan.cloud.baidu.com/",
  },
  {
    id: "bailian",
    name: "Bailian",
    baseUrls: ["https://dashscope.aliyuncs.com/compatible-mode/v1"],
    website: "https://bailian.console.aliyun.com/",
  },
  {
    id: "stepfun",
    name: "StepFun",
    baseUrls: ["https://api.stepfun.com/step_plan/v1"],
    website: "https://platform.stepfun.com/",
  },
  {
    id: "stepfun_en",
    name: "StepFun en",
    baseUrls: ["https://api.stepfun.ai/step_plan/v1"],
    website: "https://platform.stepfun.ai/",
  },
  {
    id: "modelscope",
    name: "ModelScope",
    baseUrls: ["https://api-inference.modelscope.cn/v1"],
    website: "https://modelscope.cn/",
  },
  {
    id: "longcat",
    name: "Longcat",
    baseUrls: ["https://api.longcat.chat/openai/v1"],
    website: "https://longcat.chat/",
  },
  {
    id: "minimax",
    name: "MiniMax",
    baseUrls: ["https://api.minimaxi.com/v1"],
    website: "https://www.minimaxi.com/",
  },
  {
    id: "minimax_en",
    name: "MiniMax en",
    baseUrls: ["https://api.minimax.io/v1"],
    website: "https://www.minimax.io/",
  },
  {
    id: "bailing",
    name: "BaiLing",
    baseUrls: ["https://api.tbox.cn/api/llm/v1"],
    website: "https://www.tbox.cn/",
  },
  {
    id: "xiaomi_mimo",
    name: "Xiaomi MiMo",
    baseUrls: ["https://api.xiaomimimo.com/v1"],
    website: "https://www.xiaomimimo.com/",
  },
  {
    id: "xiaomi_mimo_token_plan",
    name: "Xiaomi MiMo Token Plan",
    baseUrls: ["https://token-plan-cn.xiaomimimo.com/v1"],
    website: "https://www.xiaomimimo.com/",
  },
  {
    id: "novita",
    name: "Novita AI",
    baseUrls: ["https://api.novita.ai/openai/v1"],
    website: "https://novita.ai/",
  },
  {
    id: "nvidia",
    name: "Nvidia",
    baseUrls: ["https://integrate.api.nvidia.com/v1"],
    website: "https://build.nvidia.com/",
  },
  {
    id: "runapi",
    name: "RunAPI",
    baseUrls: ["https://runapi.co/v1"],
    website: "https://runapi.co/",
  },
  {
    id: "relaxycode",
    name: "RelaxyCode",
    baseUrls: ["https://www.relaxycode.com/v1"],
    website: "https://www.relaxycode.com/",
  },
  {
    id: "compshare_coding",
    name: "Compshare Coding Plan",
    baseUrls: ["https://cp.compshare.cn/v1"],
    website: "https://www.compshare.cn",
    apiKeyUrl:
      "https://www.compshare.cn/coding-plan?ytag=GPU_YY_YX_git_cc-switch",
  },
  {
    id: "lemondata",
    name: "E-FlowCode",
    baseUrls: ["https://api.lemondata.cc/v1", "https://e-flowcode.cc/v1"],
    website: "https://e-flowcode.cc/",
  },
  {
    id: "pipellm",
    name: "PIPELLM",
    baseUrls: ["https://cc-api.pipellm.ai/v1"],
    website: "https://code.pipellm.ai/",
  },
  {
    id: "therouter",
    name: "TheRouter",
    baseUrls: ["https://api.therouter.ai/v1"],
    website: "https://therouter.ai/",
  },
];

function normalizeCodexProviderBaseUrl(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:")
      return null;
    return `${parsed.origin}${parsed.pathname}`
      .replace(/\/+$/, "")
      .toLowerCase();
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

export function isCockpitApiProviderBaseUrl(rawBaseUrl: string): boolean {
  const normalized = normalizeCodexProviderBaseUrl(rawBaseUrl);
  if (!normalized) return false;
  return COCKPIT_API_HIDDEN_BASE_URLS.some(
    (baseUrl) => normalizeCodexProviderBaseUrl(baseUrl) === normalized,
  );
}

export function resolveCodexApiProviderPresetId(rawBaseUrl: string): string {
  return (
    findCodexApiProviderPresetByBaseUrl(rawBaseUrl)?.id ??
    CODEX_API_PROVIDER_CUSTOM_ID
  );
}
