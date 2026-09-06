/**
 * 模型名称工具
 * 用于将模型 ID 转换为友好的显示名称
 * 
 * 注意：桌面端使用小写格式（如 gemini-3-flash），插件端使用大写格式（如 MODEL_PLACEHOLDER_M18）
 */

/** 模型 ID 到显示名称的映射（支持两种格式） */
const MODEL_DISPLAY_NAMES: Record<string, string> = {
  // 桌面端格式（小写）
  'claude-opus-4-5-thinking': 'Claude Opus 4.5 (Thinking)',
  'claude-opus-4-6-thinking': 'Claude Opus 4.6 (Thinking)',
  'claude-sonnet-4-5': 'Claude Sonnet 4.5',
  'claude-sonnet-4-5-thinking': 'Claude Sonnet 4.5 (Thinking)',
  'gemini-3-flash': 'Gemini 3 Flash',
  'gemini-3-pro-high': 'Gemini 3 Pro (High)',
  'gemini-3-pro-low': 'Gemini 3 Pro (Low)',
  'gemini-3-pro-image': 'Gemini 3 Pro Image',
  'gpt-oss-120b-medium': 'GPT-OSS 120B (Medium)',
  'gpt-6-astra': '6 Astra',
  // Gemini 2.5 系列
  'gemini-2.5-flash': 'Gemini 2.5 Flash',
  'gemini-2.5-flash-lite': 'Gemini 2.5 Flash Lite',
  'gemini-2.5-flash-thinking': 'Gemini 2.5 Flash (Thinking)',
  'gemini-2.5-pro': 'Gemini 2.5 Pro',
  
  // 插件端格式（大写）
  'MODEL_PLACEHOLDER_M12': 'Claude Opus 4.5 (Thinking)',
  'MODEL_PLACEHOLDER_M26': 'Claude Opus 4.6 (Thinking)',
  'MODEL_CLAUDE_4_5_SONNET': 'Claude Sonnet 4.5',
  'MODEL_CLAUDE_4_5_SONNET_THINKING': 'Claude Sonnet 4.5 (Thinking)',
  'MODEL_PLACEHOLDER_M18': 'Gemini 3 Flash',
  'MODEL_PLACEHOLDER_M7': 'Gemini 3 Pro (High)',
  'MODEL_PLACEHOLDER_M8': 'Gemini 3 Pro (Low)',
  'MODEL_PLACEHOLDER_M9': 'Gemini 3 Pro Image',
  'MODEL_OPENAI_GPT_OSS_120B_MEDIUM': 'GPT-OSS 120B (Medium)',
};

/**
 * 与 AntigravityCockpit 对齐的授权模式黑名单。
 * 同时兼容常量 ID 和桌面端常见的小写模型 ID。
 */
const AUTH_MODEL_BLACKLIST_IDS = [
  'MODEL_CHAT_20706',
  'MODEL_CHAT_23310',
  'MODEL_GOOGLE_GEMINI_2_5_FLASH',
  'MODEL_GOOGLE_GEMINI_2_5_FLASH_THINKING',
  'MODEL_GOOGLE_GEMINI_2_5_FLASH_LITE',
  'MODEL_GOOGLE_GEMINI_2_5_PRO',
  'MODEL_PLACEHOLDER_M19',
  'chat_20706',
  'chat_23310',
  'gemini-2.5-flash',
  'gemini-2.5-flash-thinking',
  'gemini-2.5-flash-lite',
  'gemini-2.5-pro',
];

const AUTH_MODEL_BLACKLIST_DISPLAY_NAMES = [
  'Gemini 2.5 Flash',
  'Gemini 2.5 Flash (Thinking)',
  'Gemini 2.5 Flash Lite',
  'Gemini 2.5 Pro',
  'chat_20706',
  'chat_23310',
];

const AUTH_MODEL_BLACKLIST_ID_SET = new Set(
  AUTH_MODEL_BLACKLIST_IDS.map((id) => id.toLowerCase()),
);

const AUTH_MODEL_BLACKLIST_NAME_SET = new Set(
  AUTH_MODEL_BLACKLIST_DISPLAY_NAMES.map((name) => name.toLowerCase()),
);

// 按模型家族分组，忽略具体版本号（如 Gemini 3.1 / 4 / 4.5）
const GEMINI_ANY_PRO_TIER_ID_PATTERN = /^gemini-\d+(?:\.\d+)?-pro-(high|low)(?:-|$)/;
const GEMINI_ANY_FLASH_ID_PATTERN = /^gemini-\d+(?:\.\d+)?-flash(?:-|$)/;

const GEMINI_ANY_PRO_TIER_NAME_PATTERN = /^gemini \d+(?:\.\d+)? pro(?: \((high|low)\)| (high|low))\b/;
const GEMINI_ANY_FLASH_NAME_PATTERN = /^gemini \d+(?:\.\d+)? flash\b/;

type GroupPrefixMatcher = (normalizedModelId: string, normalizedModelName: string) => boolean;

const DEFAULT_GROUP_PREFIX_MATCHERS: Record<string, GroupPrefixMatcher> = {
  claude_45: (normalizedModelId, normalizedModelName) =>
    normalizedModelId.startsWith('claude-') ||
    normalizedModelId.startsWith('model_claude') ||
    normalizedModelName.startsWith('claude '),
  g3_pro: (normalizedModelId, normalizedModelName) =>
    GEMINI_ANY_PRO_TIER_ID_PATTERN.test(normalizedModelId) ||
    GEMINI_ANY_PRO_TIER_NAME_PATTERN.test(normalizedModelName),
  g3_flash: (normalizedModelId, normalizedModelName) =>
    GEMINI_ANY_FLASH_ID_PATTERN.test(normalizedModelId) ||
    GEMINI_ANY_FLASH_NAME_PATTERN.test(normalizedModelName),
};

/**
 * 获取模型的友好显示名称
 * @param modelId 模型 ID
 * @returns 友好的显示名称
 */
export function getModelDisplayName(modelId: string): string {
  if (MODEL_DISPLAY_NAMES[modelId]) {
    return MODEL_DISPLAY_NAMES[modelId];
  }
  
  // 格式化未知模型名
  return modelId
    .split('-')
    .map(word => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ');
}

/**
 * 是否命中授权模式黑名单
 */
export function isBlacklistedModel(modelId: string, displayName?: string): boolean {
  const normalizedId = modelId.trim().toLowerCase();
  if (!normalizedId) {
    return false;
  }
  if (AUTH_MODEL_BLACKLIST_ID_SET.has(normalizedId)) {
    return true;
  }
  const normalizedDisplayName = displayName?.trim().toLowerCase();
  return Boolean(
    normalizedDisplayName && AUTH_MODEL_BLACKLIST_NAME_SET.has(normalizedDisplayName),
  );
}

/** 默认分组配置 */
export interface DefaultGroup {
  id: string;
  name: string;
  desktopModels: string[];  // 桌面端模型 ID
  pluginModels: string[];   // 插件端模型 ID
}

function normalizeModelIdForGroupMatch(modelId: string): string {
  return modelId.trim().toLowerCase();
}

function normalizeModelNameForGroupMatch(modelId: string, displayName?: string): string {
  const source = (displayName?.trim() || modelId).toLowerCase();
  return source
    .replace(/[_-]+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();
}

function isExactDefaultGroupMatch(group: DefaultGroup, normalizedModelId: string): boolean {
  const isDesktopMatch = group.desktopModels.some(
    (id) => normalizeModelIdForGroupMatch(id) === normalizedModelId,
  );
  const isPluginMatch = group.pluginModels.some(
    (id) => normalizeModelIdForGroupMatch(id) === normalizedModelId,
  );
  return isDesktopMatch || isPluginMatch;
}

function matchesDefaultGroupPrefixRule(
  groupId: string,
  normalizedModelId: string,
  normalizedModelName: string,
): boolean {
  const matcher = DEFAULT_GROUP_PREFIX_MATCHERS[groupId];
  return matcher ? matcher(normalizedModelId, normalizedModelName) : false;
}

/** 获取默认分组配置（支持两种格式） */
export function getDefaultGroups(): DefaultGroup[] {
  return [
    {
      id: 'claude_45',
      name: 'Claude 4.5',
      desktopModels: [
        'claude-opus-4-5-thinking',
        'claude-opus-4-6-thinking',
        'claude-sonnet-4-5',
        'claude-sonnet-4-5-thinking',
        'gpt-oss-120b-medium',
      ],
      pluginModels: [
        'MODEL_PLACEHOLDER_M12',
        'MODEL_PLACEHOLDER_M26',
        'MODEL_CLAUDE_4_5_SONNET',
        'MODEL_CLAUDE_4_5_SONNET_THINKING',
        'MODEL_OPENAI_GPT_OSS_120B_MEDIUM',
      ],
    },
    {
      id: 'g3_pro',
      name: 'Gemini Pro',
      desktopModels: [
        'gemini-3-pro-high',
        'gemini-3-pro-low',
      ],
      pluginModels: [
        'MODEL_PLACEHOLDER_M7',
        'MODEL_PLACEHOLDER_M8',
      ],
    },
    {
      id: 'g3_flash',
      name: 'Gemini Flash',
      desktopModels: [
        'gemini-3-flash',
      ],
      pluginModels: [
        'MODEL_PLACEHOLDER_M18',
      ],
    },
  ];
}

/**
 * 解析模型默认分组
 * 匹配顺序：精确 ID 命中 -> 前缀/模式命中
 */
export function resolveDefaultGroupId(modelId: string, displayName?: string): string | null {
  const normalizedModelId = normalizeModelIdForGroupMatch(modelId);
  const normalizedModelName = normalizeModelNameForGroupMatch(modelId, displayName);
  if (!normalizedModelId) {
    return null;
  }

  const defaultGroups = getDefaultGroups();

  for (const group of defaultGroups) {
    if (isExactDefaultGroupMatch(group, normalizedModelId)) {
      return group.id;
    }
  }

  for (const group of defaultGroups) {
    if (matchesDefaultGroupPrefixRule(group.id, normalizedModelId, normalizedModelName)) {
      return group.id;
    }
  }

  return null;
}

/**
 * 自动分组模型（支持两种格式）
 * @param modelIds 模型 ID 列表
 * @returns 分组结果
 */
export function autoGroupModels(modelIds: string[]): { id: string; name: string; models: string[] }[] {
  const defaultGroups = getDefaultGroups();
  const groupedModels = new Map<string, string[]>();

  for (const modelId of modelIds) {
    const groupId = resolveDefaultGroupId(modelId);
    if (!groupId) {
      continue;
    }
    const existing = groupedModels.get(groupId);
    if (existing) {
      existing.push(modelId);
    } else {
      groupedModels.set(groupId, [modelId]);
    }
  }

  const result: { id: string; name: string; models: string[] }[] = [];
  for (const group of defaultGroups) {
    const models = groupedModels.get(group.id);
    if (!models?.length) {
      continue;
    }
    result.push({
      id: group.id,
      name: group.name,
      models,
    });
  }
  
  // 不生成"其他"分组，只保留预定义分组
  
  return result;
}

/** 推荐模型列表（支持两种格式） */
export const RECOMMENDED_MODELS = [
  // 桌面端格式
  'claude-opus-4-5-thinking',
  'claude-opus-4-6-thinking',
  'claude-sonnet-4-5',
  'claude-sonnet-4-5-thinking',
  'gemini-3-flash',
  'gemini-3-pro-high',
  'gemini-3-pro-low',
  'gpt-oss-120b-medium',
  // 插件端格式
  'MODEL_PLACEHOLDER_M12',
  'MODEL_PLACEHOLDER_M26',
  'MODEL_CLAUDE_4_5_SONNET',
  'MODEL_CLAUDE_4_5_SONNET_THINKING',
  'MODEL_PLACEHOLDER_M18',
  'MODEL_PLACEHOLDER_M7',
  'MODEL_PLACEHOLDER_M8',
  'MODEL_OPENAI_GPT_OSS_120B_MEDIUM',
];

/**
 * 检查模型是否为推荐模型
 */
export function isRecommendedModel(modelId: string): boolean {
  return RECOMMENDED_MODELS.includes(modelId);
}

/**
 * 过滤只保留推荐模型
 */
export function filterRecommendedModels(modelIds: string[]): string[] {
  return modelIds.filter(id => isRecommendedModel(id));
}
