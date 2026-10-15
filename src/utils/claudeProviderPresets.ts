import {
  APIKEY_FUN_DEFAULT_MODEL_CATALOG,
  APIKEY_FUN_DIRECT_ENDPOINT,
  APIKEY_FUN_GLOBAL_ENDPOINT,
  APIKEY_FUN_REGISTER_URL,
  APIKEY_FUN_SOURCE_TAG,
} from './apikeyFunLinks';

export type ClaudeApiKeyField = 'ANTHROPIC_AUTH_TOKEN' | 'ANTHROPIC_API_KEY';

export interface ClaudeApiProviderTemplateValue {
  label: string;
  placeholder: string;
  defaultValue?: string;
  editorValue?: string;
}

export interface ClaudeApiProviderPreset {
  id: string;
  name: string;
  baseUrls: string[];
  apiKeyField: ClaudeApiKeyField;
  website?: string;
  apiKeyUrl?: string;
  isOfficial?: boolean;
  isPartner?: boolean;
  sourceTag?: string;
  modelCatalog?: string[];
  extraEnv?: Record<string, string>;
  templateValues?: Record<string, ClaudeApiProviderTemplateValue>;
}

export const CLAUDE_API_PROVIDER_CUSTOM_ID = 'custom';
export const CLAUDE_APIKEY_FUN_PROVIDER_ID = 'apikey_fun';
export const CLAUDE_APIKEY_FUN_BASE_URL = APIKEY_FUN_GLOBAL_ENDPOINT;

const CLAUDE_MODEL_ENV_KEYS = [
  'ANTHROPIC_MODEL',
  'ANTHROPIC_DEFAULT_HAIKU_MODEL',
  'ANTHROPIC_DEFAULT_SONNET_MODEL',
  'ANTHROPIC_DEFAULT_OPUS_MODEL',
] as const;

function uniqueNonEmpty(values: Array<string | null | undefined>): string[] {
  return Array.from(new Set(values.map((value) => value?.trim() ?? '').filter(Boolean)));
}

function createClaudeApiProviderPreset(
  input: Omit<ClaudeApiProviderPreset, 'baseUrls' | 'apiKeyField' | 'modelCatalog'> & {
    baseUrl?: string;
    baseUrls?: string[];
    apiKeyField?: ClaudeApiKeyField;
    modelCatalog?: string[];
  },
): ClaudeApiProviderPreset {
  const baseUrls = input.baseUrls ?? [input.baseUrl ?? ''];
  const modelCatalog =
    input.modelCatalog ??
    uniqueNonEmpty(CLAUDE_MODEL_ENV_KEYS.map((key) => input.extraEnv?.[key]));
  return {
    ...input,
    baseUrls,
    apiKeyField: input.apiKeyField ?? 'ANTHROPIC_AUTH_TOKEN',
    ...(modelCatalog.length > 0 ? { modelCatalog } : {}),
  };
}

function modelEnv(
  model: string,
  overrides: Partial<Record<(typeof CLAUDE_MODEL_ENV_KEYS)[number], string>> = {},
  extraEnv: Record<string, string> = {},
): Record<string, string> {
  return {
    ANTHROPIC_MODEL: model,
    ANTHROPIC_DEFAULT_HAIKU_MODEL: overrides.ANTHROPIC_DEFAULT_HAIKU_MODEL ?? model,
    ANTHROPIC_DEFAULT_SONNET_MODEL: overrides.ANTHROPIC_DEFAULT_SONNET_MODEL ?? model,
    ANTHROPIC_DEFAULT_OPUS_MODEL: overrides.ANTHROPIC_DEFAULT_OPUS_MODEL ?? model,
    ...extraEnv,
  };
}

const CC_SWITCH_DIRECT_CLAUDE_PROVIDER_PRESETS: readonly ClaudeApiProviderPreset[] = [
  createClaudeApiProviderPreset({
    id: 'doubaoseed',
    name: 'DouBaoSeed',
    baseUrl: 'https://ark.cn-beijing.volces.com/api/compatible',
    website:
      'https://console.volcengine.com/ark/region:ark+cn-beijing/apiKey?apikey=%7B%7D&utm_campaign=hw&utm_content=ccswitch&utm_medium=devrel_tool_web&utm_source=OWO&utm_term=ccswitch',
    apiKeyUrl:
      'https://console.volcengine.com/ark/region:ark+cn-beijing/apiKey?apikey=%7B%7D&utm_campaign=hw&utm_content=ccswitch&utm_medium=devrel_tool_web&utm_source=OWO&utm_term=ccswitch',
    isPartner: true,
    extraEnv: modelEnv('doubao-seed-2-0-code-preview-latest', {}, { API_TIMEOUT_MS: '3000000' }),
  }),
  createClaudeApiProviderPreset({
    id: 'ccsub',
    name: 'CCSub',
    baseUrl: 'https://www.ccsub.net',
    website: 'https://www.ccsub.net',
    apiKeyUrl: 'https://www.ccsub.net/register?ref=Y6Z8DXEA',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'unity2_ai',
    name: 'Unity2.ai',
    baseUrl: 'https://api.unity2.ai',
    website: 'https://unity2.ai',
    apiKeyUrl: 'https://unity2.ai/register?source=ccs',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'zhipu_glm',
    name: 'Zhipu GLM',
    baseUrl: 'https://open.bigmodel.cn/api/anthropic',
    website: 'https://open.bigmodel.cn',
    apiKeyUrl: 'https://www.bigmodel.cn/claude-code?ic=RRVJPB5SII',
    extraEnv: modelEnv('glm-5.1'),
  }),
  createClaudeApiProviderPreset({
    id: 'zhipu_glm_global',
    name: 'Zhipu GLM en',
    baseUrl: 'https://api.z.ai/api/anthropic',
    website: 'https://z.ai',
    apiKeyUrl: 'https://z.ai/subscribe?ic=8JVLJQFSKB',
    extraEnv: modelEnv('glm-5.1'),
  }),
  createClaudeApiProviderPreset({
    id: 'baidu_qianfan_coding_plan',
    name: 'Baidu Qianfan Coding Plan',
    baseUrl: 'https://qianfan.baidubce.com/anthropic/coding',
    website: 'https://cloud.baidu.com/product/qianfan_modelbuilder',
    apiKeyUrl: 'https://console.bce.baidu.com/qianfan/ais/console/applicationConsole/application',
    extraEnv: modelEnv('qianfan-code-latest'),
  }),
  createClaudeApiProviderPreset({
    id: 'bailian',
    name: 'Bailian',
    baseUrl: 'https://dashscope.aliyuncs.com/apps/anthropic',
    website: 'https://bailian.console.aliyun.com',
  }),
  createClaudeApiProviderPreset({
    id: 'bailian_coding',
    name: 'Bailian For Coding',
    baseUrl: 'https://coding.dashscope.aliyuncs.com/apps/anthropic',
    website: 'https://bailian.console.aliyun.com',
  }),
  createClaudeApiProviderPreset({
    id: 'kimi',
    name: 'Kimi',
    baseUrl: 'https://api.moonshot.cn/anthropic',
    website: 'https://platform.moonshot.cn/console?aff=cc-switch',
    extraEnv: modelEnv('kimi-k2.7-code'),
  }),
  createClaudeApiProviderPreset({
    id: 'kimi_coding',
    name: 'Kimi For Coding',
    baseUrl: 'https://api.kimi.com/coding/',
    website: 'https://www.kimi.com/code/docs/?aff=cc-switch',
  }),
  createClaudeApiProviderPreset({
    id: 'stepfun',
    name: 'StepFun',
    baseUrl: 'https://api.stepfun.com/step_plan',
    website: 'https://platform.stepfun.com/step-plan',
    apiKeyUrl: 'https://platform.stepfun.com/interface-key',
    extraEnv: modelEnv('step-3.5-flash-2603'),
  }),
  createClaudeApiProviderPreset({
    id: 'stepfun_global',
    name: 'StepFun en',
    baseUrl: 'https://api.stepfun.ai/step_plan',
    website: 'https://platform.stepfun.ai/step-plan',
    apiKeyUrl: 'https://platform.stepfun.ai/interface-key',
    extraEnv: modelEnv('step-3.5-flash-2603'),
  }),
  createClaudeApiProviderPreset({
    id: 'modelscope',
    name: 'ModelScope',
    baseUrl: 'https://api-inference.modelscope.cn',
    website: 'https://modelscope.cn',
    extraEnv: modelEnv('ZhipuAI/GLM-5.1'),
  }),
  createClaudeApiProviderPreset({
    id: 'kat_coder',
    name: 'KAT-Coder',
    baseUrl: 'https://vanchin.streamlake.ai/api/gateway/v1/endpoints/${ENDPOINT_ID}/claude-code-proxy',
    website: 'https://console.streamlake.ai',
    apiKeyUrl: 'https://console.streamlake.ai/console/api-key',
    extraEnv: modelEnv('KAT-Coder-Pro V1', {
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'KAT-Coder-Air V1',
    }),
    templateValues: {
      ENDPOINT_ID: {
        label: 'Vanchin Endpoint ID',
        placeholder: 'ep-xxx-xxx',
        defaultValue: '',
        editorValue: '',
      },
    },
  }),
  createClaudeApiProviderPreset({
    id: 'longcat',
    name: 'Longcat',
    baseUrl: 'https://api.longcat.chat/anthropic',
    website: 'https://longcat.chat/platform',
    apiKeyUrl: 'https://longcat.chat/platform/api_keys',
    extraEnv: modelEnv('LongCat-Flash-Chat', {}, {
      CLAUDE_CODE_MAX_OUTPUT_TOKENS: '6000',
      CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
    }),
  }),
  createClaudeApiProviderPreset({
    id: 'minimax',
    name: 'MiniMax',
    baseUrl: 'https://api.minimaxi.com/anthropic',
    website: 'https://platform.minimaxi.com/docs',
    apiKeyUrl: 'https://platform.minimaxi.com/subscribe/coding-plan',
    modelCatalog: ['MiniMax-M3', 'MiniMax-M2.7'],
    extraEnv: modelEnv('MiniMax-M3', {}, {
      API_TIMEOUT_MS: '3000000',
      CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
    }),
  }),
  createClaudeApiProviderPreset({
    id: 'minimax_global',
    name: 'MiniMax en',
    baseUrl: 'https://api.minimax.io/anthropic',
    website: 'https://platform.minimax.io/docs',
    apiKeyUrl: 'https://platform.minimax.io/subscribe/coding-plan',
    modelCatalog: ['MiniMax-M3', 'MiniMax-M2.7'],
    extraEnv: modelEnv('MiniMax-M3', {}, {
      API_TIMEOUT_MS: '3000000',
      CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
    }),
  }),
  createClaudeApiProviderPreset({
    id: 'bailing',
    name: 'BaiLing',
    baseUrl: 'https://api.tbox.cn/api/anthropic',
    website: 'https://alipaytbox.yuque.com/sxs0ba/ling/get_started',
    extraEnv: modelEnv('Ling-2.5-1T'),
  }),
  createClaudeApiProviderPreset({
    id: 'aihubmix',
    name: 'AiHubMix',
    baseUrls: ['https://aihubmix.com', 'https://api.aihubmix.com'],
    apiKeyField: 'ANTHROPIC_API_KEY',
    website: 'https://aihubmix.com',
    apiKeyUrl: 'https://aihubmix.com',
  }),
  createClaudeApiProviderPreset({
    id: 'cherryin',
    name: 'CherryIN',
    baseUrl: 'https://open.cherryin.net',
    website: 'https://open.cherryin.ai',
    apiKeyUrl: 'https://open.cherryin.ai/console/token',
    extraEnv: modelEnv('anthropic/claude-sonnet-4.6', {
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'anthropic/claude-haiku-4.5',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'anthropic/claude-opus-4.8',
    }),
  }),
  createClaudeApiProviderPreset({
    id: 'siliconflow',
    name: 'SiliconFlow',
    baseUrl: 'https://api.siliconflow.cn',
    website: 'https://siliconflow.cn',
    apiKeyUrl: 'https://cloud.siliconflow.cn/i/drGuwc9k',
    isPartner: true,
    extraEnv: modelEnv('Pro/MiniMaxAI/MiniMax-M2.7'),
  }),
  createClaudeApiProviderPreset({
    id: 'siliconflow_global',
    name: 'SiliconFlow en',
    baseUrl: 'https://api.siliconflow.com',
    website: 'https://siliconflow.com',
    apiKeyUrl: 'https://cloud.siliconflow.cn/i/drGuwc9k',
    isPartner: true,
    extraEnv: modelEnv('MiniMaxAI/MiniMax-M2.7'),
  }),
  createClaudeApiProviderPreset({
    id: 'dmxapi',
    name: 'DMXAPI',
    baseUrls: ['https://www.dmxapi.cn', 'https://api.dmxapi.cn'],
    website: 'https://www.dmxapi.cn',
    apiKeyUrl: 'https://www.dmxapi.cn',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'packycode',
    name: 'PackyCode',
    baseUrls: ['https://www.packyapi.com', 'https://api-slb.packyapi.com'],
    website: 'https://www.packyapi.com',
    apiKeyUrl: 'https://www.packyapi.com/register?aff=cc-switch',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'apinebula',
    name: 'APINebula',
    baseUrl: 'https://apinebula.com',
    website: 'https://apinebula.com',
    apiKeyUrl: 'https://apinebula.com/02rw5X',
    isPartner: true,
    extraEnv: {
      CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
    },
  }),
  createClaudeApiProviderPreset({
    id: 'atlascloud',
    name: 'AtlasCloud',
    baseUrl: 'https://api.atlascloud.ai',
    website: 'https://www.atlascloud.ai/console/coding-plan',
    apiKeyUrl: 'https://www.atlascloud.ai/console/coding-plan',
    isPartner: true,
    extraEnv: modelEnv('zai-org/glm-5.1', {}, {
      CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS: '1',
    }),
  }),
  createClaudeApiProviderPreset({
    id: 'sudocode',
    name: 'SudoCode',
    baseUrls: ['https://sudocode.us', 'https://sudocode.run'],
    website: 'https://sudocode.us',
    apiKeyUrl: 'https://sudocode.us',
    extraEnv: {
      API_TIMEOUT_MS: '300000',
    },
  }),
  createClaudeApiProviderPreset({
    id: 'claudeapi',
    name: 'ClaudeAPI',
    baseUrl: 'https://gw.claudeapi.com',
    website: 'https://claudeapi.com',
    apiKeyUrl: 'https://console.claudeapi.com/register?aff=pCLD',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'claudecn',
    name: 'ClaudeCN',
    baseUrl: 'https://claudecn.top',
    website: 'https://claudecn.top',
    apiKeyUrl: 'https://claudecn.top/register?aff=ccswitch',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'runapi',
    name: 'RunAPI',
    baseUrl: 'https://runapi.co',
    website: 'https://runapi.co',
    apiKeyUrl: 'https://runapi.co',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'relaxycode',
    name: 'RelaxyCode',
    baseUrl: 'https://www.relaxycode.com',
    website: 'https://www.relaxycode.com',
    apiKeyUrl: 'https://www.relaxycode.com/register',
  }),
  createClaudeApiProviderPreset({
    id: 'cubence',
    name: 'Cubence',
    baseUrls: [
      'https://api.cubence.com',
      'https://api-cf.cubence.com',
      'https://api-dmit.cubence.com',
      'https://api-bwg.cubence.com',
    ],
    website: 'https://cubence.com',
    apiKeyUrl: 'https://cubence.com/signup?code=CCSWITCH&source=ccs',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'aigocode',
    name: 'AIGoCode',
    baseUrl: 'https://api.aigocode.com',
    website: 'https://aigocode.com',
    apiKeyUrl: 'https://aigocode.com/invite/CC-SWITCH',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'rightcode',
    name: 'RightCode',
    baseUrl: 'https://www.right.codes/claude',
    website: 'https://www.right.codes',
    apiKeyUrl: 'https://www.right.codes/register?aff=CCSWITCH',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'aicodemirror',
    name: 'AICodeMirror',
    baseUrls: [
      'https://api.aicodemirror.com/api/claudecode',
      'https://api.claudecode.net.cn/api/claudecode',
    ],
    website: 'https://www.aicodemirror.com',
    apiKeyUrl: 'https://www.aicodemirror.com/register?invitecode=9915W3',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'crazyrouter',
    name: 'CrazyRouter',
    baseUrl: 'https://cn.crazyrouter.com',
    website: 'https://www.crazyrouter.com',
    apiKeyUrl: 'https://www.crazyrouter.com/register?aff=OZcm&ref=cc-switch',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'sssaicode',
    name: 'SSSAiCode',
    baseUrls: [
      'https://node-hk.sssaicodeapi.com/api',
      'https://node-hk.sssaiapi.com/api',
      'https://node-cf.sssaicodeapi.com/api',
    ],
    website: 'https://sssaicodeapi.com',
    apiKeyUrl: 'https://sssaicodeapi.com/register?ref=DCP0SM',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'compshare',
    name: 'Compshare',
    baseUrl: 'https://api.modelverse.cn',
    website: 'https://www.compshare.cn',
    apiKeyUrl: 'https://www.compshare.cn/coding-plan?ytag=GPU_YY_YX_git_cc-switch',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'compshare_coding_plan',
    name: 'Compshare Coding Plan',
    baseUrl: 'https://cp.compshare.cn',
    website: 'https://www.compshare.cn',
    apiKeyUrl: 'https://www.compshare.cn/coding-plan?ytag=GPU_YY_YX_git_cc-switch',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'micu',
    name: 'Micu',
    baseUrl: 'https://www.micuapi.ai',
    website: 'https://www.micuapi.ai',
    apiKeyUrl: 'https://www.micuapi.ai/register?aff=aOYQ',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'ctok_ai',
    name: 'CTok.ai',
    baseUrl: 'https://api.ctok.ai',
    website: 'https://ctok.ai',
    apiKeyUrl: 'https://ctok.ai',
    isPartner: true,
  }),
  createClaudeApiProviderPreset({
    id: 'e_flowcode',
    name: 'E-FlowCode',
    baseUrl: 'https://e-flowcode.cc',
    website: 'https://e-flowcode.cc',
    apiKeyUrl: 'https://e-flowcode.cc',
  }),
  createClaudeApiProviderPreset({
    id: 'openrouter',
    name: 'OpenRouter',
    baseUrl: 'https://openrouter.ai/api',
    website: 'https://openrouter.ai',
    apiKeyUrl: 'https://openrouter.ai/keys',
    extraEnv: modelEnv('anthropic/claude-sonnet-4.6', {
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'anthropic/claude-haiku-4.5',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'anthropic/claude-opus-4.8',
    }),
  }),
  createClaudeApiProviderPreset({
    id: 'therouter',
    name: 'TheRouter',
    baseUrl: 'https://api.therouter.ai',
    apiKeyField: 'ANTHROPIC_API_KEY',
    website: 'https://therouter.ai',
    apiKeyUrl: 'https://dashboard.therouter.ai',
    extraEnv: modelEnv('anthropic/claude-sonnet-4.6', {
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'anthropic/claude-haiku-4.5',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'anthropic/claude-opus-4.8',
    }),
  }),
  createClaudeApiProviderPreset({
    id: 'novita_ai',
    name: 'Novita AI',
    baseUrl: 'https://api.novita.ai/anthropic',
    website: 'https://novita.ai',
    apiKeyUrl: 'https://novita.ai',
    extraEnv: modelEnv('zai-org/glm-5.1'),
  }),
  createClaudeApiProviderPreset({
    id: 'pipellm',
    name: 'PIPELLM',
    baseUrl: 'https://cc-api.pipellm.ai',
    website: 'https://code.pipellm.ai',
    apiKeyUrl: 'https://code.pipellm.ai/login?ref=uvw650za',
    extraEnv: modelEnv('claude-opus-4-8', {
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'claude-haiku-4-5-20251001',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'claude-sonnet-4-6',
    }),
  }),
  createClaudeApiProviderPreset({
    id: 'xiaomi_mimo',
    name: 'Xiaomi MiMo',
    baseUrl: 'https://api.xiaomimimo.com/anthropic',
    website: 'https://platform.xiaomimimo.com',
    apiKeyUrl: 'https://platform.xiaomimimo.com/#/console/api-keys',
    extraEnv: modelEnv('mimo-v2.5-pro'),
  }),
  createClaudeApiProviderPreset({
    id: 'xiaomi_mimo_token_plan_cn',
    name: 'Xiaomi MiMo Token Plan (China)',
    baseUrl: 'https://token-plan-cn.xiaomimimo.com/anthropic',
    website: 'https://platform.xiaomimimo.com/#/token-plan',
    apiKeyUrl: 'https://platform.xiaomimimo.com/#/console/plan-manage',
    extraEnv: modelEnv('mimo-v2.5-pro'),
  }),
  createClaudeApiProviderPreset({
    id: 'aws_bedrock_aksk',
    name: 'AWS Bedrock (AKSK)',
    baseUrl: 'https://bedrock-runtime.${AWS_REGION}.amazonaws.com',
    website: 'https://aws.amazon.com/bedrock/',
    extraEnv: modelEnv('global.anthropic.claude-opus-4-8', {
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'global.anthropic.claude-haiku-4-5-20251001-v1:0',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'global.anthropic.claude-sonnet-4-6',
    }, {
      AWS_ACCESS_KEY_ID: '${AWS_ACCESS_KEY_ID}',
      AWS_SECRET_ACCESS_KEY: '${AWS_SECRET_ACCESS_KEY}',
      AWS_REGION: '${AWS_REGION}',
      CLAUDE_CODE_USE_BEDROCK: '1',
    }),
    templateValues: {
      AWS_REGION: { label: 'AWS Region', placeholder: 'us-west-2', editorValue: 'us-west-2' },
      AWS_ACCESS_KEY_ID: { label: 'Access Key ID', placeholder: 'AKIA...', editorValue: '' },
      AWS_SECRET_ACCESS_KEY: { label: 'Secret Access Key', placeholder: 'your-secret-key', editorValue: '' },
    },
  }),
  createClaudeApiProviderPreset({
    id: 'aws_bedrock_api_key',
    name: 'AWS Bedrock (API Key)',
    baseUrl: 'https://bedrock-runtime.${AWS_REGION}.amazonaws.com',
    website: 'https://aws.amazon.com/bedrock/',
    extraEnv: modelEnv('global.anthropic.claude-opus-4-8', {
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'global.anthropic.claude-haiku-4-5-20251001-v1:0',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'global.anthropic.claude-sonnet-4-6',
    }, {
      AWS_REGION: '${AWS_REGION}',
      CLAUDE_CODE_USE_BEDROCK: '1',
    }),
    templateValues: {
      AWS_REGION: { label: 'AWS Region', placeholder: 'us-west-2', editorValue: 'us-west-2' },
    },
  }),
];

export const CLAUDE_API_PROVIDER_PRESETS: readonly ClaudeApiProviderPreset[] = [
  {
    id: CLAUDE_APIKEY_FUN_PROVIDER_ID,
    name: 'APIKEY.FUN',
    baseUrls: [CLAUDE_APIKEY_FUN_BASE_URL, APIKEY_FUN_DIRECT_ENDPOINT],
    apiKeyField: 'ANTHROPIC_AUTH_TOKEN',
    website: 'https://apikey.fan',
    apiKeyUrl: APIKEY_FUN_REGISTER_URL,
    isPartner: true,
    sourceTag: APIKEY_FUN_SOURCE_TAG,
    modelCatalog: [...APIKEY_FUN_DEFAULT_MODEL_CATALOG],
    extraEnv: {
      CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
    },
  },
  {
    id: 'anthropic_official',
    name: 'Anthropic Official',
    baseUrls: [''],
    apiKeyField: 'ANTHROPIC_API_KEY',
    website: 'https://www.anthropic.com/claude-code',
    apiKeyUrl: 'https://console.anthropic.com/settings/keys',
    isOfficial: true,
  },
  {
    id: 'shengsuanyun',
    name: 'Shengsuanyun',
    baseUrls: ['https://router.shengsuanyun.com/api'],
    apiKeyField: 'ANTHROPIC_AUTH_TOKEN',
    website: 'https://www.shengsuanyun.com/?from=CH_4HHXMRYF',
    apiKeyUrl: 'https://www.shengsuanyun.com/?from=CH_4HHXMRYF',
    isPartner: true,
    modelCatalog: [
      'anthropic/claude-sonnet-4.6',
      'anthropic/claude-haiku-4.5',
      'anthropic/claude-opus-4.8',
    ],
    extraEnv: {
      ANTHROPIC_MODEL: 'anthropic/claude-sonnet-4.6',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'anthropic/claude-haiku-4.5',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'anthropic/claude-sonnet-4.6',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'anthropic/claude-opus-4.8',
    },
  },
  {
    id: 'pateway_ai',
    name: 'PatewayAI',
    baseUrls: ['https://api.pateway.ai'],
    apiKeyField: 'ANTHROPIC_API_KEY',
    website: 'https://pateway.ai',
    apiKeyUrl: 'https://pateway.ai/?ch=etzpm8&aff=WB6M6F67#/',
    isPartner: true,
  },
  {
    id: 'volcengine_agentplan',
    name: '火山Agentplan',
    baseUrls: ['https://ark.cn-beijing.volces.com/api/coding'],
    apiKeyField: 'ANTHROPIC_AUTH_TOKEN',
    website: 'https://www.volcengine.com/product/ark',
    isPartner: true,
    modelCatalog: ['ark-code-latest'],
    extraEnv: {
      ANTHROPIC_MODEL: 'ark-code-latest',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'ark-code-latest',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'ark-code-latest',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'ark-code-latest',
    },
  },
  {
    id: 'byteplus',
    name: 'BytePlus',
    baseUrls: ['https://ark.ap-southeast.bytepluses.com/api/coding'],
    apiKeyField: 'ANTHROPIC_AUTH_TOKEN',
    website: 'https://www.byteplus.com/en/product/modelark',
    isPartner: true,
    modelCatalog: ['ark-code-latest'],
    extraEnv: {
      ANTHROPIC_MODEL: 'ark-code-latest',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'ark-code-latest',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'ark-code-latest',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'ark-code-latest',
    },
  },
  {
    id: 'deepseek',
    name: 'DeepSeek',
    baseUrls: ['https://api.deepseek.com/anthropic'],
    apiKeyField: 'ANTHROPIC_AUTH_TOKEN',
    website: 'https://platform.deepseek.com',
    modelCatalog: ['deepseek-v4-pro', 'deepseek-v4-flash'],
    extraEnv: {
      ANTHROPIC_MODEL: 'deepseek-v4-pro',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'deepseek-v4-flash',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'deepseek-v4-pro',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'deepseek-v4-pro',
    },
  },
  ...CC_SWITCH_DIRECT_CLAUDE_PROVIDER_PRESETS,
];

export function findClaudeApiProviderPresetById(
  id?: string | null,
): ClaudeApiProviderPreset | null {
  if (!id) return null;
  return CLAUDE_API_PROVIDER_PRESETS.find((preset) => preset.id === id) ?? null;
}

export function normalizeClaudeApiProviderBaseUrl(value?: string | null): string | null {
  const trimmed = value?.trim() ?? '';
  if (!trimmed) return '';
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
    return `${parsed.origin}${parsed.pathname}`.replace(/\/+$/, '');
  } catch {
    return null;
  }
}

export function inferClaudeApiKeyField(
  provider: ClaudeApiProviderPreset | null | undefined,
  baseUrl?: string | null,
): ClaudeApiKeyField {
  if (provider && provider.id !== CLAUDE_API_PROVIDER_CUSTOM_ID) {
    return provider.apiKeyField;
  }

  const normalizedBaseUrl = normalizeClaudeApiProviderBaseUrl(baseUrl);
  if (normalizedBaseUrl === '') {
    return 'ANTHROPIC_API_KEY';
  }
  if (normalizedBaseUrl === null) {
    return 'ANTHROPIC_AUTH_TOKEN';
  }

  try {
    const host = new URL(normalizedBaseUrl).hostname.toLowerCase();
    return host === 'api.anthropic.com' || host === 'api.claude.com'
      ? 'ANTHROPIC_API_KEY'
      : 'ANTHROPIC_AUTH_TOKEN';
  } catch {
    return 'ANTHROPIC_AUTH_TOKEN';
  }
}

export function getDefaultClaudeApiProviderPresetId(): string {
  return CLAUDE_APIKEY_FUN_PROVIDER_ID;
}
