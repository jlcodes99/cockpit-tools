/**
 * WorkBuddy 模型建议列表
 *
 * WorkBuddy 的模型目录是运行时从服务端拉取的（客户端里没有静态模型枚举），
 * 这里只提供一份“常见模型”的便捷输入建议，实际探测时请以 WorkBuddy 客户端
 * 中显示的准确模型 ID 为准（支持自由输入）。
 */

export interface WorkbuddyModelOption {
  /** 发给探测接口的模型标识 */
  id: string;
  /** 展示名（仅用于下拉建议，不参与请求） */
  label: string;
}

/**
 * 常用模型建议。按 WorkBuddy 常见提供方分组，便于挑选。
 * 注意：这只是输入提示，并非权威目录；若下拉里没有你要测的模型，直接手填即可。
 */
export const WORKBUDDY_SUGGESTED_MODELS: WorkbuddyModelOption[] = [
  // 腾讯混元
  { id: 'hunyuan-turbo', label: 'Hunyuan Turbo（混元）' },
  { id: 'hunyuan-pro', label: 'Hunyuan Pro（混元）' },
  { id: 'hunyuan-standard', label: 'Hunyuan Standard（混元）' },
  { id: 'hunyuan-vision', label: 'Hunyuan Vision（混元）' },
  // DeepSeek
  { id: 'deepseek-v3', label: 'DeepSeek V3' },
  { id: 'deepseek-r1', label: 'DeepSeek R1' },
  // Anthropic Claude
  { id: 'claude-3-5-sonnet', label: 'Claude 3.5 Sonnet' },
  { id: 'claude-3-7-sonnet', label: 'Claude 3.7 Sonnet' },
  { id: 'claude-sonnet-4', label: 'Claude Sonnet 4' },
  // OpenAI
  { id: 'gpt-4o', label: 'GPT-4o' },
  { id: 'gpt-4o-mini', label: 'GPT-4o mini' },
  { id: 'o3-mini', label: 'OpenAI o3-mini' },
  // 其他常见
  { id: 'glm-4', label: 'GLM-4（智谱）' },
  { id: 'moonshot-v1-8k', label: 'Kimi / Moonshot' },
  { id: 'qwen-max', label: 'Qwen Max（通义千问）' },
  { id: 'gemini-2.0-pro', label: 'Gemini 2.0 Pro' },
];
