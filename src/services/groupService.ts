/**
 * 分组配置服务
 * 与后端 group_settings 模块交互
 */

import { invoke } from '@tauri-apps/api/core';

/** 分组配置 */
export interface GroupSettings {
  groupMappings: Record<string, string>;  // modelId -> groupId
  groupNames: Record<string, string>;     // groupId -> displayName
  groupOrder: string[];                   // 分组排序
  updatedAt: number;                      // 最后更新时间戳
  updatedBy: 'plugin' | 'desktop';        // 最后更新来源
}

/** 显示用分组信息 */
export interface DisplayGroup {
  id: string;
  name: string;
  models: string[];
}

const FIXED_DISPLAY_GROUPS: DisplayGroup[] = [
  { id: 'g4_argon', name: 'Gemini 4 Argon', models: ['gemini-4-argon', 'gemini-4-argon-medium', 'gemini-4-argon-high', 'argon:weekly', 'gemini-argon-wtu-weekly'] },
  {
    id: 'claude_45',
    name: 'Claude',
    models: [
        'claude-opus-5-5',
        'claude-opus-5-5-medium',
        'claude-opus-5-5-high',
        'claude-sonnet-5-5',
        'claude-sonnet-5-5-medium',
        'claude-sonnet-5-5-high',

      'claude-opus-4-6-thinking',
      'claude-opus-4-6',
      'claude-opus-4-5-thinking',
      'claude-sonnet-4-6',
      'claude-sonnet-4-6-thinking',
      'claude-sonnet-4-5',
      'claude-sonnet-4-5-thinking',
      'gpt-oss-120b-medium',
      'MODEL_PLACEHOLDER_M12',
      'MODEL_PLACEHOLDER_M26',
      'MODEL_PLACEHOLDER_M35',
      'MODEL_CLAUDE_4_5_SONNET',
      'MODEL_CLAUDE_4_5_SONNET_THINKING',
      'MODEL_OPENAI_GPT_OSS_120B_MEDIUM',
    ],
  },
  {
    id: 'g3_pro',
    name: 'Gemini Pro',
    models: [
      'gemini-3.1-pro-high',
      'gemini-3.1-pro-low',
      'gemini-3-pro-high',
      'gemini-3-pro-low',
      'gemini-3-pro-image',
      'MODEL_PLACEHOLDER_M7',
      'MODEL_PLACEHOLDER_M8',
      'MODEL_PLACEHOLDER_M9',
      'MODEL_PLACEHOLDER_M36',
      'MODEL_PLACEHOLDER_M37',
    ],
  },
  {
    id: 'g3_flash',
    name: 'Gemini Flash',
    models: [
        'gemini-3.8-flash',
        'gemini-3.8-flash-low',
        'gemini-3.8-flash-medium',
        'gemini-3.8-flash-high',
        'MODEL_PLACEHOLDER_M318',

      'gemini-3-flash',
      'gemini-3.1-flash',
      'gemini-3-flash-image',
      'gemini-3.1-flash-image',
      'gemini-3-flash-lite',
      'gemini-3.1-flash-lite',
      'MODEL_PLACEHOLDER_M18',
    ],
  },
];

/**
 * 获取完整分组配置
 */
export async function getGroupSettings(): Promise<GroupSettings> {
  return invoke<GroupSettings>('get_group_settings');
}

/**
 * 保存完整分组配置
 */
export async function saveGroupSettings(
  groupMappings: Record<string, string>,
  groupNames: Record<string, string>,
  groupOrder: string[]
): Promise<void> {
  return invoke('save_group_settings', {
    groupMappings,
    groupNames,
    groupOrder,
  });
}

/**
 * 设置模型的分组
 */
export async function setModelGroup(modelId: string, groupId: string): Promise<void> {
  return invoke('set_model_group', { modelId, groupId });
}

/**
 * 移除模型的分组
 */
export async function removeModelGroup(modelId: string): Promise<void> {
  return invoke('remove_model_group', { modelId });
}

/**
 * 设置分组名称
 */
export async function setGroupName(groupId: string, name: string): Promise<void> {
  return invoke('set_group_name', { groupId, name });
}

/**
 * 删除分组
 */
export async function deleteGroup(groupId: string): Promise<void> {
  return invoke('delete_group', { groupId });
}

/**
 * 更新分组排序
 */
export async function updateGroupOrder(order: string[]): Promise<void> {
  return invoke('update_group_order', { order });
}

/**
 * 获取显示用分组列表（最多3个）
 */
export async function getDisplayGroups(): Promise<DisplayGroup[]> {
  return FIXED_DISPLAY_GROUPS.map((group) => ({
    id: group.id,
    name: group.name,
    models: [...group.models],
  }));
}

/**
 * 默认分组配置（用于初始化）
 */
export const DEFAULT_GROUP_SETTINGS: GroupSettings = {
  groupMappings: {},
  groupNames: {},
  groupOrder: [],
  updatedAt: 0,
  updatedBy: 'desktop',
};

/**
 * 根据模型配额计算分组配额
 * @param groupId 分组 ID
 * @param modelQuotas 模型配额 { modelId: percentage }
 * @param settings 分组配置
 */
export function calculateGroupQuota(
  groupId: string,
  modelQuotas: Record<string, number>,
  settings: GroupSettings
): number | null {
  const modelsInGroup = Object.entries(settings.groupMappings)
    .filter(([, gid]) => gid === groupId)
    .map(([mid]) => mid);
  
  if (modelsInGroup.length === 0) {
    return null;
  }
  
  let total = 0;
  let count = 0;
  
  for (const modelId of modelsInGroup) {
    if (modelId in modelQuotas) {
      total += modelQuotas[modelId];
      count++;
    }
  }
  
  return count > 0 ? Math.round(total / count) : null;
}

/**
 * 计算账号综合配额
 * @param modelQuotas 模型配额 { modelId: percentage }
 */
export function calculateOverallQuota(modelQuotas: Record<string, number>): number {
  const values = Object.values(modelQuotas);
  if (values.length === 0) return 0;
  return Math.round(values.reduce((a, b) => a + b, 0) / values.length);
}
