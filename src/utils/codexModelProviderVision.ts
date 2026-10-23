import modelCatalog from "../../sidecars/cockpit-cliproxy/third_party/CLIProxyAPI/internal/registry/models/models.json";
import {
  DEEPSEEK_API_PROVIDER_ID,
  resolveCodexApiProviderPresetId,
} from "./codexProviderPresets";

export interface CodexModelProviderVisionInput {
  baseUrl: string;
  supportsVision?: boolean;
  modelCapabilities?: Record<string, { supportsVision?: boolean }>;
  modelCatalog?: string[];
}

export function resolveProviderModelVisionState(
  modelId: string,
  states: Record<string, boolean> | undefined,
  providerDefault = false,
): boolean {
  return states?.[modelId.trim().toLowerCase()] ??
    (providerDefault || providerModelDefaultsToVisionInput(modelId));
}

/** 根据模型目录生成完整的最终能力表；显式模型值优先于供应商默认值和模型默认值。 */
export function buildProviderModelVisionCapabilities(
  modelText: string,
  states: Record<string, boolean>,
  modelCatalog: string[] = [],
  providerDefault = false,
): Record<string, { supportsVision: boolean }> {
  const capabilities: Record<string, { supportsVision: boolean }> = {};
  if (modelCatalog.length > 0) {
    for (const model of modelCatalog) {
      const key = model.trim().toLowerCase();
      if (!key) continue;
      capabilities[key] = {
        supportsVision: resolveProviderModelVisionState(model, states, providerDefault),
      };
    }
    return capabilities;
  }
  for (const model of modelText.split(/[\n,]+/)) {
    const key = model.trim().toLowerCase();
    if (key) capabilities[key] = { supportsVision: true };
  }
  for (const [model, supportsVision] of Object.entries(states)) {
    const key = model.trim().toLowerCase();
    if (key) capabilities[key] = { supportsVision };
  }
  return capabilities;
}

/**
 * 模型识图默认规则（与 Rust 侧 `codex_account::model_defaults_to_vision_input` 同一口径）：
 * 按随程序附带的 models.json 查找完整模型 ID，输入类型包含 image 时默认支持图片。
 * 未找到模型或缺少图片输入声明时默认 false。
 * 允许 `openai/gpt-5.6-sol` 这类命名空间前缀；逐模型设置与供应商默认值仍优先。
 */
export function providerModelDefaultsToVisionInput(modelId: string): boolean {
  const normalized = (modelId ?? "").trim().toLowerCase();
  const name = normalized.split("/").pop() ?? "";
  return Object.values(modelCatalog).some((models) =>
    models.some((model) =>
      model.id.trim().toLowerCase() === name &&
      "supportedInputModalities" in model &&
      model.supportedInputModalities?.includes("image") === true,
    ),
  );
}

/**
 * v1.3.49 之前识图开关挂在供应商上，之后收敛为逐模型能力。
 * 升级后旧供应商可能只有 `supportsVision`，逐模型表为空，Provider Gateway 会把
 * 支持图片的模型当成 text-only 并静默删除 `input_image`（issue #2535）。
 *
 * 这里把旧的供应商级开关展开到模型目录，返回是否发生变化；调用方负责持久化。
 * 官方 DeepSeek 走自己的逐模型默认值，不参与该迁移。
 */
export function expandLegacyProviderVisionCapabilities(
  provider: CodexModelProviderVisionInput,
): boolean {
  if (provider.supportsVision !== true) return false;
  if (resolveCodexApiProviderPresetId(provider.baseUrl) === DEEPSEEK_API_PROVIDER_ID) {
    return false;
  }
  const catalog = provider.modelCatalog ?? [];
  if (catalog.length === 0) return false;

  const next: Record<string, { supportsVision?: boolean }> = {
    ...(provider.modelCapabilities ?? {}),
  };
  let changed = false;
  for (const rawModel of catalog) {
    const key = rawModel.trim().toLowerCase();
    if (!key || Object.prototype.hasOwnProperty.call(next, key)) continue;
    next[key] = { supportsVision: true };
    changed = true;
  }
  if (!changed) return false;
  provider.modelCapabilities = next;
  return true;
}

/** 将非 DeepSeek provider 的模型目录和默认规则展开并持久化为完整能力表。 */
export function materializeProviderModelVisionCapabilities(
  provider: CodexModelProviderVisionInput,
): boolean {
  if (resolveCodexApiProviderPresetId(provider.baseUrl) === DEEPSEEK_API_PROVIDER_ID) {
    return false;
  }
  const catalog = provider.modelCatalog ?? [];
  if (catalog.length === 0) return false;
  const states = Object.fromEntries(
    Object.entries(provider.modelCapabilities ?? {}).map(([model, capability]) => [
      model.trim().toLowerCase(),
      capability.supportsVision === true,
    ]),
  );
  const next = buildProviderModelVisionCapabilities(
    "",
    states,
    catalog,
    provider.supportsVision === true,
  );
  const previous = provider.modelCapabilities ?? {};
  const changed =
    Object.keys(previous).length !== Object.keys(next).length ||
    Object.entries(next).some(
      ([model, capability]) => previous[model]?.supportsVision !== capability.supportsVision,
    );
  if (!changed) return false;
  provider.modelCapabilities = next;
  return true;
}
