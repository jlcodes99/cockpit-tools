/**
 * WorkBuddy 模型目录
 *
 * 数据来源：WorkBuddy 官方桌面客户端内置模型目录（逆向自 app.asar，2026-11），
 * 对应服务端 tencent-* 模型组（tencent-token-plan / tencent-coding 等）。
 *
 * 注意：客户端运行时还会合并企业目录（/console/enterprises/{id}/config/models）
 * 与用户自定义模型（users/private-configs），所以本列表覆盖的是内置目录；
 * 若要测的模型不在这里，仍可直接手输模型 ID。
 */

export interface WorkbuddyModelOption {
  /** 发给探测接口的模型标识 */
  id: string;
  /** 展示名（仅用于下拉建议，不参与请求） */
  label: string;
  /** 客户端目录里标记为 recommended */
  recommended?: boolean;
  /** 支持推理/思考模式 */
  reasoning?: boolean;
  /** 支持图片输入 */
  vision?: boolean;
}

export interface WorkbuddyModelGroup {
  group: string;
  options: WorkbuddyModelOption[];
}

/**
 * WorkBuddy 内置模型目录（按系列分组，ID 均来自客户端实际数据）。
 */
export const WORKBUDDY_MODEL_GROUPS: WorkbuddyModelGroup[] = [
  {
    group: '智能路由',
    options: [
      { id: 'auto', label: 'Auto（智能路由）', recommended: true, reasoning: true },
      { id: 'tc-code-latest', label: 'Auto（编码路由）', recommended: true },
    ],
  },
  {
    group: '腾讯混元',
    options: [
      { id: 'hy3', label: 'Hy3', recommended: true, reasoning: true },
      { id: 'hy3-preview', label: 'Hy3 Preview', reasoning: true },
      { id: 'hunyuan-t1', label: 'Hunyuan-T1', reasoning: true },
      { id: 'hunyuan-turbos', label: 'Hunyuan-TurboS' },
      { id: 'hunyuan-2.0-instruct', label: 'Hunyuan 2.0 Instruct' },
      { id: 'hunyuan-2.0-thinking', label: 'Hunyuan 2.0 Thinking', reasoning: true },
    ],
  },
  {
    group: 'DeepSeek',
    options: [
      { id: 'deepseek-v4-pro', label: 'DeepSeek-V4-Pro', reasoning: true },
      { id: 'deepseek-v4-flash', label: 'DeepSeek-V4-Flash', reasoning: true },
      { id: 'deepseek-v4-pro-202606', label: 'DeepSeek-V4-Pro（原厂直供）', reasoning: true },
      { id: 'deepseek-v4-flash-202605', label: 'DeepSeek-V4-Flash（原厂直供）', reasoning: true },
    ],
  },
  {
    group: '智谱 GLM',
    options: [
      { id: 'glm-5.2', label: 'GLM-5.2', recommended: true, reasoning: true },
      { id: 'glm-5.1', label: 'GLM-5.1', reasoning: true },
      { id: 'glm-5', label: 'GLM-5', reasoning: true },
      { id: 'glm-5-turbo', label: 'GLM-5-Turbo', reasoning: true },
    ],
  },
  {
    group: '月之暗面 Kimi',
    options: [
      { id: 'kimi-k2.6', label: 'Kimi-K2.6', reasoning: true, vision: true },
      { id: 'kimi-k2.5', label: 'Kimi-K2.5', reasoning: true, vision: true },
      { id: 'kimi-k2.7-code', label: 'Kimi K2.7 Code', reasoning: true },
      { id: 'kimi-k2.7-code-highspeed', label: 'Kimi K2.7 Code HighSpeed', reasoning: true },
    ],
  },
  {
    group: 'MiniMax',
    options: [
      { id: 'minimax-m3', label: 'MiniMax-M3', reasoning: true, vision: true },
      { id: 'minimax-m2.7', label: 'MiniMax-M2.7', reasoning: true },
      { id: 'minimax-m2.5', label: 'MiniMax-M2.5', reasoning: true },
    ],
  },
];

/**
 * 扁平建议列表（供 datalist 使用）：按分组顺序展开，recommended 靠前。
 */
export const WORKBUDDY_SUGGESTED_MODELS: WorkbuddyModelOption[] = (() => {
  const flat = WORKBUDDY_MODEL_GROUPS.flatMap((g) => g.options);
  const recommended = flat.filter((m) => m.recommended);
  const rest = flat.filter((m) => !m.recommended);
  return [...recommended, ...rest];
})();
