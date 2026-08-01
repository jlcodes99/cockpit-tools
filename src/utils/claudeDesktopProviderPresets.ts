import type {
  ClaudeDesktopGatewayConnectionMode,
  ClaudeDesktopGatewayFamilyTier,
  ClaudeDesktopGatewayModelMapping,
} from '../types/claude';
import {
  APIKEY_FUN_GLOBAL_ENDPOINT,
  APIKEY_FUN_REGISTER_URL,
  APIKEY_FUN_SOURCE_TAG,
} from './apikeyFunLinks';

export type ClaudeDesktopGatewayAuthScheme = 'bearer' | 'x-api-key' | 'auto';
export type ClaudeDesktopGatewayApiKeyField = 'ANTHROPIC_AUTH_TOKEN' | 'ANTHROPIC_API_KEY';

export interface ClaudeDesktopGatewayProviderPreset {
  id: string;
  name: string;
  baseUrls: string[];
  authScheme: ClaudeDesktopGatewayAuthScheme;
  apiKeyField?: ClaudeDesktopGatewayApiKeyField;
  connectionMode: ClaudeDesktopGatewayConnectionMode;
  website?: string;
  apiKeyUrl?: string;
  isOfficial?: boolean;
  isPartner?: boolean;
  sourceTag?: string;
  modelMappings: ClaudeDesktopGatewayModelMapping[];
}

export const CLAUDE_DESKTOP_GATEWAY_PROVIDER_CUSTOM_ID = 'custom';
export const CLAUDE_DESKTOP_GATEWAY_APIKEY_FUN_PROVIDER_ID = 'apikey_fun';

export const CLAUDE_DESKTOP_GATEWAY_DEFAULT_MODELS = [
  'claude-opus-4-8',
  'claude-fable-5',
  'claude-opus-4-7',
  'claude-opus-4-6',
  'claude-sonnet-4-6',
  'claude-haiku-4-5',
] as const;

export const CLAUDE_DESKTOP_GATEWAY_FAMILY_TIERS = [
  'haiku',
  'sonnet',
  'opus',
  'fable',
  'mythos',
] as const satisfies readonly ClaudeDesktopGatewayFamilyTier[];

export function inferClaudeDesktopGatewayFamilyTier(
  model: string,
): ClaudeDesktopGatewayFamilyTier | null {
  const parts = model.trim().toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
  return CLAUDE_DESKTOP_GATEWAY_FAMILY_TIERS.find((tier) => parts.includes(tier)) ?? null;
}

function uniqueNonEmpty(values: Array<string | null | undefined>): string[] {
  return Array.from(new Set(values.map((value) => value?.trim() ?? '').filter(Boolean)));
}

function createRoute(
  desktopModel: string,
  upstreamModel: string,
  options: {
    labelOverride?: string | null;
    supports1m?: boolean;
    anthropicFamilyTier?: ClaudeDesktopGatewayFamilyTier | null;
  } = {},
): ClaudeDesktopGatewayModelMapping {
  const anthropicFamilyTier = options.anthropicFamilyTier
    ?? inferClaudeDesktopGatewayFamilyTier(desktopModel);
  return {
    desktopModel,
    upstreamModel,
    ...(options.labelOverride?.trim() ? { labelOverride: options.labelOverride.trim() } : {}),
    ...(options.supports1m ? { supports1m: true } : {}),
    ...(anthropicFamilyTier ? { anthropicFamilyTier } : {}),
  };
}

function directRoutes(
  models: readonly string[] = CLAUDE_DESKTOP_GATEWAY_DEFAULT_MODELS,
  options: { supports1m?: boolean } = {},
): ClaudeDesktopGatewayModelMapping[] {
  return uniqueNonEmpty([...models]).map((model) => createRoute(model, model, options));
}

function roleMappedRoutes(
  sonnet: string,
  opus: string,
  haiku: string,
  options: { supports1m?: boolean } = {},
): ClaudeDesktopGatewayModelMapping[] {
  return [
    createRoute('claude-sonnet-4-6', sonnet, options),
    createRoute('claude-opus-4-8', opus, options),
    createRoute('claude-haiku-4-5', haiku, options),
  ].filter((mapping) => mapping.upstreamModel.trim());
}

function brandedRoutes(
  sonnet: string,
  opus: string,
  haiku: string,
  options: { supports1m?: boolean } = {},
): ClaudeDesktopGatewayModelMapping[] {
  const seen = new Set<string>();
  return [
    ['claude-sonnet-4-6', sonnet],
    ['claude-opus-4-8', opus],
    ['claude-haiku-4-5', haiku],
  ]
    .map(([desktopModel, upstreamModel]) =>
      createRoute(desktopModel, upstreamModel, {
        labelOverride: upstreamModel,
        supports1m: options.supports1m,
      }),
    )
    .filter((mapping) => {
      const upstreamKey = mapping.upstreamModel.trim().toLowerCase();
      if (!upstreamKey || seen.has(upstreamKey)) return false;
      seen.add(upstreamKey);
      return true;
    });
}

function repeatedMappedRoute(
  upstreamModel: string,
  options: { supports1m?: boolean } = {},
): ClaudeDesktopGatewayModelMapping[] {
  return brandedRoutes(upstreamModel, upstreamModel, upstreamModel, options);
}

export const CLAUDE_DESKTOP_GATEWAY_PROVIDER_PRESETS: readonly ClaudeDesktopGatewayProviderPreset[] = [
  {
    id: CLAUDE_DESKTOP_GATEWAY_APIKEY_FUN_PROVIDER_ID,
    name: 'APIKEY.FUN',
    baseUrls: [APIKEY_FUN_GLOBAL_ENDPOINT, 'https://slb.apikey.fun'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://apikey.fun',
    apiKeyUrl: APIKEY_FUN_REGISTER_URL,
    isPartner: true,
    sourceTag: APIKEY_FUN_SOURCE_TAG,
    modelMappings: directRoutes(),
  },
  {
    id: 'anthropic_official',
    name: 'Anthropic Official',
    baseUrls: ['https://api.anthropic.com'],
    authScheme: 'x-api-key',
    apiKeyField: 'ANTHROPIC_API_KEY',
    connectionMode: 'direct',
    website: 'https://www.anthropic.com',
    apiKeyUrl: 'https://console.anthropic.com/settings/keys',
    isOfficial: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'shengsuanyun',
    name: 'Shengsuanyun',
    baseUrls: ['https://router.shengsuanyun.com/api'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://www.shengsuanyun.com/?from=CH_4HHXMRYF',
    apiKeyUrl: 'https://www.shengsuanyun.com/?from=CH_4HHXMRYF',
    isPartner: true,
    modelMappings: roleMappedRoutes(
      'anthropic/claude-sonnet-4.6',
      'anthropic/claude-opus-4.8',
      'anthropic/claude-haiku-4.5',
    ),
  },
  {
    id: 'pateway_ai',
    name: 'PatewayAI',
    baseUrls: ['https://api.pateway.ai'],
    authScheme: 'x-api-key',
    apiKeyField: 'ANTHROPIC_API_KEY',
    connectionMode: 'direct',
    website: 'https://pateway.ai',
    apiKeyUrl: 'https://pateway.ai/?ch=etzpm8&aff=WB6M6F67#/',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'volcengine_agentplan',
    name: '火山Agentplan',
    baseUrls: ['https://ark.cn-beijing.volces.com/api/coding'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://www.volcengine.com/activity/codingplan?ac=MMAP8JTTCAQ2&rc=6J6FV5N2&utm_campaign=hw&utm_content=ccswitch&utm_medium=devrel_tool_web&utm_source=OWO&utm_term=ccswitch',
    apiKeyUrl: 'https://www.volcengine.com/activity/codingplan?ac=MMAP8JTTCAQ2&rc=6J6FV5N2&utm_campaign=hw&utm_content=ccswitch&utm_medium=devrel_tool_web&utm_source=OWO&utm_term=ccswitch',
    isPartner: true,
    modelMappings: repeatedMappedRoute('ark-code-latest'),
  },
  {
    id: 'byteplus',
    name: 'BytePlus',
    baseUrls: ['https://ark.ap-southeast.bytepluses.com/api/coding'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://www.byteplus.com/en/product/modelark?utm_campaign=hw&utm_content=ccswitch&utm_medium=devrel_tool_web&utm_source=OWO&utm_term=ccswitch',
    apiKeyUrl: 'https://www.byteplus.com/en/product/modelark?utm_campaign=hw&utm_content=ccswitch&utm_medium=devrel_tool_web&utm_source=OWO&utm_term=ccswitch',
    isPartner: true,
    modelMappings: repeatedMappedRoute('ark-code-latest'),
  },
  {
    id: 'ccsub',
    name: 'CCSub',
    baseUrls: ['https://www.ccsub.net'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://www.ccsub.net',
    apiKeyUrl: 'https://www.ccsub.net/register?ref=Y6Z8DXEA',
    isPartner: true,
    modelMappings: directRoutes(CLAUDE_DESKTOP_GATEWAY_DEFAULT_MODELS, { supports1m: true }),
  },
  {
    id: 'unity2_ai',
    name: 'Unity2.ai',
    baseUrls: ['https://api.unity2.ai'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://unity2.ai',
    apiKeyUrl: 'https://unity2.ai/register?source=ccs',
    isPartner: true,
    modelMappings: directRoutes(CLAUDE_DESKTOP_GATEWAY_DEFAULT_MODELS, { supports1m: true }),
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    baseUrls: ['https://api.deepseek.com/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://platform.deepseek.com',
    modelMappings: brandedRoutes(
      'deepseek-v4-pro',
      'deepseek-v4-pro',
      'deepseek-v4-flash',
    ),
  },
  {
    id: 'kimi',
    name: 'Kimi',
    baseUrls: ['https://api.moonshot.cn/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://platform.moonshot.cn/console?aff=cc-switch',
    isPartner: true,
    modelMappings: repeatedMappedRoute('kimi-k2.7-code'),
  },
  {
    id: 'kimi_coding',
    name: 'Kimi For Coding',
    baseUrls: ['https://api.kimi.com/coding'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://www.kimi.com/code/docs/?aff=cc-switch',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'zhipu_glm',
    name: 'Zhipu GLM',
    baseUrls: ['https://open.bigmodel.cn/api/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://open.bigmodel.cn',
    apiKeyUrl: 'https://www.bigmodel.cn/claude-code?ic=RRVJPB5SII',
    modelMappings: repeatedMappedRoute('glm-5.1'),
  },
  {
    id: 'zhipu_glm_global',
    name: 'Zhipu GLM en',
    baseUrls: ['https://api.z.ai/api/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://z.ai',
    apiKeyUrl: 'https://z.ai/subscribe?ic=8JVLJQFSKB',
    modelMappings: repeatedMappedRoute('glm-5.1'),
  },
  {
    id: 'baidu_qianfan_coding',
    name: 'Baidu Qianfan Coding Plan',
    baseUrls: ['https://qianfan.baidubce.com/anthropic/coding'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://cloud.baidu.com/product/qianfan_modelbuilder',
    apiKeyUrl: 'https://console.bce.baidu.com/qianfan/ais/console/applicationConsole/application',
    modelMappings: repeatedMappedRoute('qianfan-code-latest'),
  },
  {
    id: 'bailian',
    name: 'Bailian',
    baseUrls: ['https://dashscope.aliyuncs.com/apps/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://bailian.console.aliyun.com',
    modelMappings: directRoutes(),
  },
  {
    id: 'bailian_coding',
    name: 'Bailian For Coding',
    baseUrls: ['https://coding.dashscope.aliyuncs.com/apps/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://bailian.console.aliyun.com',
    modelMappings: directRoutes(),
  },
  {
    id: 'stepfun',
    name: 'StepFun',
    baseUrls: ['https://api.stepfun.com/step_plan'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://platform.stepfun.com/step-plan',
    apiKeyUrl: 'https://platform.stepfun.com/interface-key',
    modelMappings: repeatedMappedRoute('step-3.5-flash-2603'),
  },
  {
    id: 'stepfun_global',
    name: 'StepFun en',
    baseUrls: ['https://api.stepfun.ai/step_plan'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://platform.stepfun.ai/step-plan',
    apiKeyUrl: 'https://platform.stepfun.ai/interface-key',
    modelMappings: repeatedMappedRoute('step-3.5-flash-2603'),
  },
  {
    id: 'modelscope',
    name: 'ModelScope',
    baseUrls: ['https://api-inference.modelscope.cn'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://modelscope.cn',
    modelMappings: repeatedMappedRoute('ZhipuAI/GLM-5.1'),
  },
  {
    id: 'longcat',
    name: 'Longcat',
    baseUrls: ['https://api.longcat.chat/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://longcat.chat/platform',
    apiKeyUrl: 'https://longcat.chat/platform/api_keys',
    modelMappings: repeatedMappedRoute('LongCat-Flash-Chat'),
  },
  {
    id: 'minimax',
    name: 'MiniMax',
    baseUrls: ['https://api.minimaxi.com/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://platform.minimaxi.com/docs',
    apiKeyUrl: 'https://platform.minimaxi.com/subscribe/coding-plan',
    modelMappings: [
      createRoute('claude-sonnet-4-6', 'MiniMax-M3', {
        labelOverride: 'MiniMax-M3',
        supports1m: true,
      }),
      createRoute('claude-haiku-4-5', 'MiniMax-M2.7', {
        labelOverride: 'MiniMax-M2.7',
      }),
    ],
  },
  {
    id: 'minimax_global',
    name: 'MiniMax en',
    baseUrls: ['https://api.minimax.io/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://platform.minimax.io/docs',
    apiKeyUrl: 'https://platform.minimax.io/subscribe/coding-plan',
    modelMappings: [
      createRoute('claude-sonnet-4-6', 'MiniMax-M3', {
        labelOverride: 'MiniMax-M3',
        supports1m: true,
      }),
      createRoute('claude-haiku-4-5', 'MiniMax-M2.7', {
        labelOverride: 'MiniMax-M2.7',
      }),
    ],
  },
  {
    id: 'bailing',
    name: 'BaiLing',
    baseUrls: ['https://api.tbox.cn/api/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://alipaytbox.yuque.com/sxs0ba/ling/get_started',
    modelMappings: repeatedMappedRoute('Ling-2.5-1T'),
  },
  {
    id: 'xiaomi_mimo',
    name: 'Xiaomi MiMo',
    baseUrls: ['https://api.xiaomimimo.com/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://platform.xiaomimimo.com',
    apiKeyUrl: 'https://platform.xiaomimimo.com/#/console/api-keys',
    modelMappings: repeatedMappedRoute('mimo-v2.5-pro'),
  },
  {
    id: 'xiaomi_mimo_token_plan_cn',
    name: 'Xiaomi MiMo Token Plan (China)',
    baseUrls: ['https://token-plan-cn.xiaomimimo.com/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://platform.xiaomimimo.com/#/token-plan',
    apiKeyUrl: 'https://platform.xiaomimimo.com/#/console/plan-manage',
    modelMappings: repeatedMappedRoute('mimo-v2.5-pro'),
  },
  {
    id: 'doubaoseed',
    name: 'DouBaoSeed',
    baseUrls: ['https://ark.cn-beijing.volces.com/api/compatible'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://console.volcengine.com/ark/region:ark+cn-beijing/apiKey?apikey=%7B%7D&utm_campaign=hw&utm_content=ccswitch&utm_medium=devrel_tool_web&utm_source=OWO&utm_term=ccswitch',
    apiKeyUrl: 'https://console.volcengine.com/ark/region:ark+cn-beijing/apiKey?apikey=%7B%7D&utm_campaign=hw&utm_content=ccswitch&utm_medium=devrel_tool_web&utm_source=OWO&utm_term=ccswitch',
    isPartner: true,
    modelMappings: repeatedMappedRoute('doubao-seed-2-0-code-preview-latest'),
  },
  {
    id: 'cherryin',
    name: 'CherryIN',
    baseUrls: ['https://open.cherryin.net'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://open.cherryin.ai',
    apiKeyUrl: 'https://open.cherryin.ai/console/token',
    modelMappings: roleMappedRoutes(
      'anthropic/claude-sonnet-4.6',
      'anthropic/claude-opus-4.8',
      'anthropic/claude-haiku-4.5',
    ),
  },
  {
    id: 'siliconflow',
    name: 'SiliconFlow',
    baseUrls: ['https://api.siliconflow.cn'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://siliconflow.cn',
    apiKeyUrl: 'https://cloud.siliconflow.cn/i/drGuwc9k',
    isPartner: true,
    modelMappings: repeatedMappedRoute('Pro/MiniMaxAI/MiniMax-M2.7'),
  },
  {
    id: 'siliconflow_global',
    name: 'SiliconFlow en',
    baseUrls: ['https://api.siliconflow.com'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://siliconflow.com',
    apiKeyUrl: 'https://cloud.siliconflow.cn/i/drGuwc9k',
    isPartner: true,
    modelMappings: repeatedMappedRoute('MiniMaxAI/MiniMax-M2.7'),
  },
  {
    id: 'aihubmix',
    name: 'AiHubMix',
    baseUrls: ['https://aihubmix.com', 'https://api.aihubmix.com'],
    authScheme: 'x-api-key',
    apiKeyField: 'ANTHROPIC_API_KEY',
    connectionMode: 'direct',
    website: 'https://aihubmix.com',
    apiKeyUrl: 'https://aihubmix.com',
    modelMappings: directRoutes(),
  },
  {
    id: 'dmxapi',
    name: 'DMXAPI',
    baseUrls: ['https://www.dmxapi.cn', 'https://api.dmxapi.cn'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://www.dmxapi.cn',
    apiKeyUrl: 'https://www.dmxapi.cn',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'packycode',
    name: 'PackyCode',
    baseUrls: ['https://www.packyapi.com', 'https://api-slb.packyapi.com'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://www.packyapi.com',
    apiKeyUrl: 'https://www.packyapi.com/register?aff=cc-switch',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'apinebula',
    name: 'APINebula',
    baseUrls: ['https://apinebula.com'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://apinebula.com',
    apiKeyUrl: 'https://apinebula.com/02rw5X',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'atlascloud',
    name: 'AtlasCloud',
    baseUrls: ['https://api.atlascloud.ai'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://www.atlascloud.ai/console/coding-plan',
    apiKeyUrl: 'https://www.atlascloud.ai/console/coding-plan',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'sudocode',
    name: 'SudoCode',
    baseUrls: ['https://sudocode.us', 'https://sudocode.run'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://sudocode.us',
    apiKeyUrl: 'https://sudocode.us',
    modelMappings: directRoutes(),
  },
  {
    id: 'claudeapi',
    name: 'ClaudeAPI',
    baseUrls: ['https://gw.claudeapi.com'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://claudeapi.com',
    apiKeyUrl: 'https://console.claudeapi.com/register?aff=pCLD',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'claudecn',
    name: 'ClaudeCN',
    baseUrls: ['https://claudecn.top'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://claudecn.top',
    apiKeyUrl: 'https://claudecn.top/register?aff=ccswitch',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'runapi',
    name: 'RunAPI',
    baseUrls: ['https://runapi.co'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://runapi.co',
    apiKeyUrl: 'https://runapi.co',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'relaxycode',
    name: 'RelaxyCode',
    baseUrls: ['https://www.relaxycode.com'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://www.relaxycode.com',
    apiKeyUrl: 'https://www.relaxycode.com/register',
    modelMappings: directRoutes(),
  },
  {
    id: 'cubence',
    name: 'Cubence',
    baseUrls: [
      'https://api.cubence.com',
      'https://api-cf.cubence.com',
      'https://api-dmit.cubence.com',
      'https://api-bwg.cubence.com',
    ],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://cubence.com',
    apiKeyUrl: 'https://cubence.com/signup?code=CCSWITCH&source=ccs',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'aigocode',
    name: 'AIGoCode',
    baseUrls: ['https://api.aigocode.com'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://aigocode.com',
    apiKeyUrl: 'https://aigocode.com/invite/CC-SWITCH',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'rightcode',
    name: 'RightCode',
    baseUrls: ['https://www.right.codes/claude'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://www.right.codes',
    apiKeyUrl: 'https://www.right.codes/register?aff=CCSWITCH',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'aicodemirror',
    name: 'AICodeMirror',
    baseUrls: [
      'https://api.aicodemirror.com/api/claudecode',
      'https://api.claudecode.net.cn/api/claudecode',
    ],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://www.aicodemirror.com',
    apiKeyUrl: 'https://www.aicodemirror.com/register?invitecode=9915W3',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'crazyrouter',
    name: 'CrazyRouter',
    baseUrls: ['https://cn.crazyrouter.com'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://www.crazyrouter.com',
    apiKeyUrl: 'https://www.crazyrouter.com/register?aff=OZcm&ref=cc-switch',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'sssaicode',
    name: 'SSSAiCode',
    baseUrls: [
      'https://node-hk.sssaicodeapi.com/api',
      'https://node-hk.sssaiapi.com/api',
      'https://node-cf.sssaicodeapi.com/api',
    ],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://sssaicodeapi.com',
    apiKeyUrl: 'https://sssaicodeapi.com/register?ref=DCP0SM',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'compshare',
    name: 'Compshare',
    baseUrls: ['https://api.modelverse.cn'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://www.compshare.cn',
    apiKeyUrl: 'https://www.compshare.cn/coding-plan?ytag=GPU_YY_YX_git_cc-switch',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'compshare_coding',
    name: 'Compshare Coding Plan',
    baseUrls: ['https://cp.compshare.cn'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://www.compshare.cn',
    apiKeyUrl: 'https://www.compshare.cn/coding-plan?ytag=GPU_YY_YX_git_cc-switch',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'micu',
    name: 'Micu',
    baseUrls: ['https://www.micuapi.ai'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://www.micuapi.ai',
    apiKeyUrl: 'https://www.micuapi.ai/register?aff=aOYQ',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'ctok',
    name: 'CTok.ai',
    baseUrls: ['https://api.ctok.ai'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://ctok.ai',
    apiKeyUrl: 'https://ctok.ai',
    isPartner: true,
    modelMappings: directRoutes(),
  },
  {
    id: 'eflowcode',
    name: 'E-FlowCode',
    baseUrls: ['https://e-flowcode.cc'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://e-flowcode.cc',
    apiKeyUrl: 'https://e-flowcode.cc',
    modelMappings: directRoutes(),
  },
  {
    id: 'openrouter',
    name: 'OpenRouter',
    baseUrls: ['https://openrouter.ai/api'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://openrouter.ai',
    apiKeyUrl: 'https://openrouter.ai/keys',
    modelMappings: roleMappedRoutes(
      'anthropic/claude-sonnet-4.6',
      'anthropic/claude-opus-4.8',
      'anthropic/claude-haiku-4.5',
      { supports1m: true },
    ),
  },
  {
    id: 'therouter',
    name: 'TheRouter',
    baseUrls: ['https://api.therouter.ai'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://therouter.ai',
    apiKeyUrl: 'https://dashboard.therouter.ai',
    modelMappings: roleMappedRoutes(
      'anthropic/claude-sonnet-4.6',
      'anthropic/claude-opus-4.8',
      'anthropic/claude-haiku-4.5',
      { supports1m: true },
    ),
  },
  {
    id: 'novita_ai',
    name: 'Novita AI',
    baseUrls: ['https://api.novita.ai/anthropic'],
    authScheme: 'bearer',
    connectionMode: 'local_mapping',
    website: 'https://novita.ai',
    apiKeyUrl: 'https://novita.ai',
    modelMappings: repeatedMappedRoute('zai-org/glm-5.1'),
  },
  {
    id: 'pipellm',
    name: 'PIPELLM',
    baseUrls: ['https://cc-api.pipellm.ai'],
    authScheme: 'bearer',
    connectionMode: 'direct',
    website: 'https://code.pipellm.ai',
    apiKeyUrl: 'https://code.pipellm.ai/login?ref=uvw650za',
    modelMappings: directRoutes(),
  },
];

export function getDefaultClaudeDesktopGatewayProviderPresetId(): string {
  return CLAUDE_DESKTOP_GATEWAY_APIKEY_FUN_PROVIDER_ID;
}

export function findClaudeDesktopGatewayProviderPresetById(
  id?: string | null,
): ClaudeDesktopGatewayProviderPreset | null {
  if (!id) return null;
  return CLAUDE_DESKTOP_GATEWAY_PROVIDER_PRESETS.find((preset) => preset.id === id) ?? null;
}

export function inferClaudeDesktopGatewayApiKeyField(
  preset?: ClaudeDesktopGatewayProviderPreset | null,
  authScheme?: ClaudeDesktopGatewayAuthScheme | string | null,
): ClaudeDesktopGatewayApiKeyField {
  if (preset?.apiKeyField) return preset.apiKeyField;
  return authScheme?.trim().toLowerCase() === 'x-api-key'
    ? 'ANTHROPIC_API_KEY'
    : 'ANTHROPIC_AUTH_TOKEN';
}
