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

/** Only stored entries and explicit user choices enter states; retain false overrides. */
export function buildProviderModelVisionCapabilities(
  modelText: string,
  states: Record<string, boolean>,
): Record<string, { supportsVision: boolean }> {
  const capabilities: Record<string, { supportsVision: boolean }> = {};
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
 * `gpt-5.5` 及以上（含 5.5）默认支持图片输入；其它模型不受影响。
 * 兼容 `openai/gpt-5.6-sol` 这类命名空间前缀与 `-luna` / `-sol` 这类后缀。
 */
export function providerModelDefaultsToVisionInput(modelId: string): boolean {
  const normalized = (modelId ?? "").trim().toLowerCase();
  const name = normalized.split("/").pop() ?? "";
  if (!name.startsWith("gpt-")) return false;
  const rest = name.slice("gpt-".length);
  const head = rest.split(/[^0-9.]/)[0] ?? "";
  const [majorRaw, minorRaw] = head.split(".");
  const major = Number.parseInt(majorRaw ?? "", 10);
  if (!Number.isFinite(major)) return false;
  const parsedMinor = Number.parseInt(minorRaw ?? "", 10);
  const minor = Number.isFinite(parsedMinor) ? parsedMinor : 0;
  return major > 5 || (major === 5 && minor >= 5);
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
