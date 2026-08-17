import { invoke } from '@tauri-apps/api/core';
import type { CodexAccount } from '../types/codex';
import type {
  CodexProviderEnableModePreference,
  CodexProviderWireApi,
} from '../utils/codexProviderGateway';
import type { CodexLocalAccessTestResult } from '../types/codexLocalAccess';
import {
  DEEPSEEK_API_BASE_URL,
  DEEPSEEK_API_PROVIDER_ID,
  DEEPSEEK_CODEX_MODEL_CATALOG,
  findCodexApiProviderPresetById,
  resolveCodexApiProviderPresetId,
} from '../utils/codexProviderPresets';
import {
  APIKEY_FUN_DEFAULT_MODEL_CATALOG,
  isApiKeyFunProviderBaseUrl,
} from '../utils/apikeyFunLinks';
import {
  queryModelProviderUsage,
  type ModelProviderUsageSummary,
} from './modelProviderUsageService';
import { moveCodexProviderApiKey } from '../utils/codexModelProviderApiKeyMove';

export interface CodexModelProviderApiKey {
  id: string;
  name: string;
  apiKey: string;
  createdAt: number;
  updatedAt: number;
}

export interface CodexModelProvider {
  id: string;
  name: string;
  baseUrl: string;
  sourceTag?: string;
  integrationType?: 'sub2api' | 'new_api';
  modelCatalog?: string[];
  modelContextWindows?: Record<string, number>;
  supportsVision?: boolean;
  modelCapabilities?: Record<string, { supportsVision?: boolean }>;
  visionRoutingModel?: string;
  boundInstanceId?: string;
  website?: string;
  apiKeyUrl?: string;
  wireApi?: CodexProviderWireApi | null;
  supportsWebsockets: boolean;
  enableModePreference?: CodexProviderEnableModePreference;
  boundOauthAccountId?: string | null;
  apiKeys: CodexModelProviderApiKey[];
  createdAt: number;
  updatedAt: number;
}

export type CodexModelProviderUsageSummary = ModelProviderUsageSummary;

interface UpsertFromCredentialInput {
  providerId?: string | null;
  previousProviderId?: string | null;
  providerName?: string | null;
  apiBaseUrl: string;
  apiKey: string;
  apiKeyName?: string | null;
  sourceTag?: string | null;
  modelCatalog?: string[];
  modelContextWindows?: Record<string, number>;
  supportsVision?: boolean;
  modelCapabilities?: Record<string, { supportsVision?: boolean }>;
  visionRoutingModel?: string | null;
  website?: string | null;
  apiKeyUrl?: string | null;
  wireApi?: CodexProviderWireApi | null;
  supportsWebsockets?: boolean;
  integrationType?: 'sub2api' | 'new_api' | null;
}

let providerIdCounter = 0;
let keyIdCounter = 0;
let cachedProviders: CodexModelProvider[] | null = null;

function createProviderId(): string {
  return `cmp_${Date.now()}_${++providerIdCounter}`;
}

function createApiKeyId(): string {
  return `cmk_${Date.now()}_${++keyIdCounter}`;
}

function sanitizeName(value: string): string {
  return value.trim();
}

function sanitizeApiKey(value: string): string {
  return value.trim();
}

function normalizeWireApi(value: unknown): CodexProviderWireApi | undefined {
  return value === 'responses' || value === 'chat_completions' ? value : undefined;
}

function normalizeSupportsWebsockets(
  value: unknown,
  wireApi?: CodexProviderWireApi,
  baseUrl?: string,
): boolean {
  return (
    wireApi === 'responses' &&
    value === true &&
    resolveCodexApiProviderPresetId(baseUrl ?? '') !== 'openai_official'
  );
}

function normalizeEnableModePreference(
  value: unknown,
): CodexProviderEnableModePreference | undefined {
  return value === 'auto' || value === 'direct' || value === 'gateway' ? value : undefined;
}

function normalizeModelCatalog(value: unknown): string[] | undefined {
  if (!Array.isArray(value)) return undefined;
  const seen = new Set<string>();
  const models: string[] = [];
  for (const item of value) {
    const model = String(item ?? '').trim();
    const key = model.toLowerCase();
    if (!model || seen.has(key)) continue;
    seen.add(key);
    models.push(model);
  }
  return models.length > 0 ? models : undefined;
}

export function normalizeModelContextWindows(
  value: unknown,
  catalog: string[] = [],
): Record<string, number> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  const source = value as Record<string, unknown>;
  const next: Record<string, number> = {};
  const keys = catalog.length > 0 ? catalog : Object.keys(source);
  for (const model of keys) {
    const trimmed = model.trim();
    if (!trimmed) continue;
    const raw =
      source[trimmed] ??
      Object.entries(source).find(([name]) => name.trim().toLowerCase() === trimmed.toLowerCase())?.[1];
    const parsed = typeof raw === 'number' ? raw : Number(String(raw ?? '').trim());
    if (Number.isInteger(parsed) && parsed > 0) {
      next[trimmed] = parsed;
    }
  }
  return Object.keys(next).length > 0 ? next : undefined;
}

function normalizeModelCapabilities(
  value: unknown,
): Record<string, { supportsVision?: boolean }> | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  const result: Record<string, { supportsVision?: boolean }> = {};
  for (const [rawModel, rawCapability] of Object.entries(value as Record<string, unknown>)) {
    const model = rawModel.trim().toLowerCase();
    if (!model || !rawCapability || typeof rawCapability !== 'object') continue;
    const supportsVision = (rawCapability as { supportsVision?: unknown }).supportsVision;
    if (typeof supportsVision === 'boolean') {
      result[model] = { supportsVision };
    }
  }
  return Object.keys(result).length > 0 ? result : undefined;
}

function normalizeBoundInstanceId(value: unknown): string | undefined {
  const id = String(value ?? '').trim();
  return id || undefined;
}

function normalizeBoundOauthAccountId(value: unknown): string | undefined {
  const id = String(value ?? '').trim();
  return id || undefined;
}

function hasOwnProperty(value: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function normalizeIntegrationType(value: unknown): 'sub2api' | 'new_api' | undefined {
  return value === 'sub2api' || value === 'new_api' ? value : undefined;
}

function migrateApiKeyFunProviderWireApi(
  providers: CodexModelProvider[],
): { providers: CodexModelProvider[]; changed: boolean } {
  let changed = false;
  const next = providers.map((provider) => {
    if (
      isApiKeyFunProviderBaseUrl(provider.baseUrl) &&
      provider.wireApi === 'chat_completions'
    ) {
      changed = true;
      return {
        ...provider,
        wireApi: 'responses' as CodexProviderWireApi,
        enableModePreference:
          provider.enableModePreference === 'gateway' ? 'direct' : provider.enableModePreference,
        updatedAt: Date.now(),
      };
    }
    return provider;
  });
  return { providers: next, changed };
}

/**
 * Normalize official DeepSeek providers without locking Chat Completions out.
 * - Missing wireApi defaults to Responses.
 * - Explicit chat_completions is preserved.
 * - Responses mode writes official catalog slugs and talks to the official API directly.
 */
function enforceDeepSeekProvider(provider: CodexModelProvider): boolean {
  if (resolveCodexApiProviderPresetId(provider.baseUrl) !== DEEPSEEK_API_PROVIDER_ID) {
    return false;
  }
  const modelCatalog = [...DEEPSEEK_CODEX_MODEL_CATALOG];
  let changed = false;

  if (provider.baseUrl !== DEEPSEEK_API_BASE_URL) {
    provider.baseUrl = DEEPSEEK_API_BASE_URL;
    changed = true;
  }

  const wireApi = provider.wireApi;
  if (wireApi !== 'responses' && wireApi !== 'chat_completions') {
    provider.wireApi = 'responses';
    changed = true;
  }

  if (provider.wireApi === 'responses') {
    if (provider.supportsWebsockets) {
      provider.supportsWebsockets = false;
      changed = true;
    }
    if (provider.supportsVision === true) {
      provider.supportsVision = false;
      changed = true;
    }
    if (provider.visionRoutingModel !== undefined) {
      provider.visionRoutingModel = undefined;
      changed = true;
    }
    if (provider.modelCapabilities !== undefined) {
      provider.modelCapabilities = undefined;
      changed = true;
    }
    if (
      provider.modelCatalog?.length !== modelCatalog.length ||
      modelCatalog.some((model, index) => provider.modelCatalog?.[index] !== model)
    ) {
      provider.modelCatalog = modelCatalog;
      changed = true;
    }
  } else if (provider.wireApi === 'chat_completions') {
    // Chat Completions stays on gateway path.
    if (provider.enableModePreference === 'direct') {
      provider.enableModePreference = 'gateway';
      changed = true;
    }
    if (provider.supportsWebsockets) {
      provider.supportsWebsockets = false;
      changed = true;
    }
  }

  if (changed) {
    provider.updatedAt = Date.now();
  }
  return changed;
}

function presetModelCatalogForBaseUrl(baseUrl: string): string[] | undefined {
  if (isApiKeyFunProviderBaseUrl(baseUrl)) {
    return normalizeModelCatalog(APIKEY_FUN_DEFAULT_MODEL_CATALOG);
  }
  return normalizeModelCatalog(
    findCodexApiProviderPresetById(resolveCodexApiProviderPresetId(baseUrl))
      ?.modelCatalog,
  );
}

export function normalizeCodexModelProviderBaseUrl(value: string): string | null {
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

function normalizeBaseUrlForStore(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return '';
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return trimmed;
    return `${parsed.origin}${parsed.pathname}`.replace(/\/+$/, '');
  } catch {
    return trimmed;
  }
}

function deriveProviderNameFromBaseUrl(baseUrl: string): string {
  try {
    const host = new URL(baseUrl).hostname.toLowerCase();
    return host.replace(/^www\./, '') || 'Custom Provider';
  } catch {
    return 'Custom Provider';
  }
}

function cloneProviders(providers: CodexModelProvider[]): CodexModelProvider[] {
  return providers.map((provider) => ({
    ...provider,
    modelCapabilities: provider.modelCapabilities
      ? Object.fromEntries(
          Object.entries(provider.modelCapabilities).map(([model, capability]) => [
            model,
            { ...capability },
          ]),
        )
      : undefined,
    visionRoutingModel: sanitizeName(provider.visionRoutingModel ?? '') || undefined,
    apiKeys: provider.apiKeys.map((apiKey) => ({ ...apiKey })),
  }));
}

function toValidApiKeys(value: unknown, now: number): CodexModelProviderApiKey[] {
  if (!Array.isArray(value)) return [];
  const result: CodexModelProviderApiKey[] = [];
  for (const item of value) {
    if (!item || typeof item !== 'object') continue;
    const rawKey = sanitizeApiKey(String((item as { apiKey?: unknown }).apiKey ?? ''));
    if (!rawKey) continue;
    result.push({
      id: String((item as { id?: unknown }).id ?? createApiKeyId()),
      name: sanitizeName(String((item as { name?: unknown }).name ?? '')),
      apiKey: rawKey,
      createdAt: Number((item as { createdAt?: unknown }).createdAt ?? now),
      updatedAt: Number((item as { updatedAt?: unknown }).updatedAt ?? now),
    });
  }
  return result;
}

function toValidProviderList(raw: unknown): CodexModelProvider[] {
  if (!Array.isArray(raw)) return [];
  const now = Date.now();
  const providers: CodexModelProvider[] = [];
  const seenBaseUrls = new Set<string>();
  for (const item of raw) {
    if (!item || typeof item !== 'object') continue;
    const name = sanitizeName(String((item as { name?: unknown }).name ?? ''));
    const baseUrl = normalizeBaseUrlForStore(String((item as { baseUrl?: unknown }).baseUrl ?? ''));
    const normalizedBaseUrl = normalizeCodexModelProviderBaseUrl(baseUrl);
    if (!name || !baseUrl || !normalizedBaseUrl) continue;
    if (seenBaseUrls.has(normalizedBaseUrl)) continue;
    seenBaseUrls.add(normalizedBaseUrl);
    const boundOauthAccountId = normalizeBoundOauthAccountId(
      (item as { boundOauthAccountId?: unknown }).boundOauthAccountId,
    );
    const wireApi = normalizeWireApi((item as { wireApi?: unknown }).wireApi);
    providers.push({
      id: String((item as { id?: unknown }).id ?? createProviderId()),
      name,
      baseUrl,
      sourceTag: sanitizeName(String((item as { sourceTag?: unknown }).sourceTag ?? '')) || undefined,
      integrationType: normalizeIntegrationType(
        (item as { integrationType?: unknown }).integrationType,
      ),
      modelCatalog:
        normalizeModelCatalog((item as { modelCatalog?: unknown }).modelCatalog) ??
        presetModelCatalogForBaseUrl(baseUrl),
      modelContextWindows: normalizeModelContextWindows(
        (item as { modelContextWindows?: unknown }).modelContextWindows,
        normalizeModelCatalog((item as { modelCatalog?: unknown }).modelCatalog) ??
          presetModelCatalogForBaseUrl(baseUrl) ??
          [],
      ),
      supportsVision: (item as { supportsVision?: unknown }).supportsVision === true,
      modelCapabilities: normalizeModelCapabilities(
        (item as { modelCapabilities?: unknown }).modelCapabilities,
      ),
      visionRoutingModel:
        sanitizeName(String((item as { visionRoutingModel?: unknown }).visionRoutingModel ?? '')) ||
        undefined,
      boundInstanceId: normalizeBoundInstanceId(
        (item as { boundInstanceId?: unknown }).boundInstanceId,
      ),
      website: sanitizeName(String((item as { website?: unknown }).website ?? '')) || undefined,
      apiKeyUrl: sanitizeName(String((item as { apiKeyUrl?: unknown }).apiKeyUrl ?? '')) || undefined,
      wireApi,
      supportsWebsockets: normalizeSupportsWebsockets(
        (item as { supportsWebsockets?: unknown }).supportsWebsockets,
        wireApi,
        baseUrl,
      ),
      enableModePreference: normalizeEnableModePreference(
        (item as { enableModePreference?: unknown }).enableModePreference,
      ),
      boundOauthAccountId,
      apiKeys: toValidApiKeys((item as { apiKeys?: unknown }).apiKeys, now),
      createdAt: Number((item as { createdAt?: unknown }).createdAt ?? now),
      updatedAt: Number((item as { updatedAt?: unknown }).updatedAt ?? now),
    });
  }
  return providers.sort((a, b) => a.createdAt - b.createdAt);
}

function hasRemovedImageGenerationSetting(raw: unknown): boolean {
  if (!Array.isArray(raw)) return false;
  return raw.some(
    (item) =>
      Boolean(item) &&
      typeof item === 'object' &&
      hasOwnProperty(item, 'boundOauthUseLocalGateway'),
  );
}

function hasLegacySupportsWebsocketsProvider(raw: unknown): boolean {
  if (!Array.isArray(raw)) return false;
  return raw.some((item) => {
    if (!item || typeof item !== 'object') return false;
    return !hasOwnProperty(item, 'supportsWebsockets');
  });
}

async function loadProvidersFromDisk(): Promise<{
  providers: CodexModelProvider[];
  removedImageGenerationSetting: boolean;
  migratedSupportsWebsockets: boolean;
}> {
  const raw = await invoke<string>('load_codex_model_providers');
  const parsed = JSON.parse(raw);
  return {
    providers: toValidProviderList(parsed),
    removedImageGenerationSetting: hasRemovedImageGenerationSetting(parsed),
    migratedSupportsWebsockets: hasLegacySupportsWebsocketsProvider(parsed),
  };
}

async function saveProvidersToDisk(providers: CodexModelProvider[]): Promise<void> {
  await invoke('save_codex_model_providers', {
    data: JSON.stringify(providers, null, 2),
  });
}

async function ensureProvidersLoaded(): Promise<CodexModelProvider[]> {
  if (cachedProviders !== null) return cloneProviders(cachedProviders);
  const loadResult = await loadProvidersFromDisk().catch(() => ({
    providers: [],
    removedImageGenerationSetting: false,
    migratedSupportsWebsockets: false,
  }));
  const loadedProviders = loadResult.providers;
  let loaded = loadedProviders.filter((provider) => {
    // 兼容清理：移除旧版本自动注入但未配置 API Key 的默认预设项
    if (provider.id.startsWith('preset_') && provider.apiKeys.length === 0) {
      return false;
    }
    return true;
  });
  const migration = migrateApiKeyFunProviderWireApi(loaded);
  loaded = migration.providers;
  let migratedDeepSeek = false;
  for (const provider of loaded) {
    migratedDeepSeek = enforceDeepSeekProvider(provider) || migratedDeepSeek;
  }
  if (
    loaded.length !== loadedProviders.length ||
    migration.changed ||
    migratedDeepSeek ||
    loadResult.removedImageGenerationSetting ||
    loadResult.migratedSupportsWebsockets
  ) {
    await saveProvidersToDisk(loaded).catch(() => { });
  }
  cachedProviders = loaded;
  return cloneProviders(cachedProviders);
}

async function writeProviders(providers: CodexModelProvider[]): Promise<void> {
  const next = cloneProviders(providers);
  cachedProviders = next;
  await saveProvidersToDisk(next);
}

export async function listCodexModelProviders(): Promise<CodexModelProvider[]> {
  return ensureProvidersLoaded();
}

export function invalidateCodexModelProviderCache(): void {
  cachedProviders = null;
}

export function findCodexModelProviderById(
  providers: CodexModelProvider[],
  providerId?: string | null,
): CodexModelProvider | null {
  if (!providerId) return null;
  return providers.find((provider) => provider.id === providerId) ?? null;
}

export function findCodexModelProviderByBaseUrl(
  providers: CodexModelProvider[],
  baseUrl: string,
): CodexModelProvider | null {
  const normalized = normalizeCodexModelProviderBaseUrl(baseUrl);
  if (!normalized) return null;
  return (
    providers.find(
      (provider) => normalizeCodexModelProviderBaseUrl(provider.baseUrl) === normalized,
    ) ?? null
  );
}

function ensureApiKeyOnProvider(
  provider: CodexModelProvider,
  apiKey: string,
  apiKeyName?: string | null,
): void {
  const normalized = sanitizeApiKey(apiKey);
  if (!normalized) return;
  const now = Date.now();
  const existing = provider.apiKeys.find((item) => sanitizeApiKey(item.apiKey) === normalized);
  if (existing) {
    // #1510: reusing the same key must not clobber a saved display name.
    existing.updatedAt = now;
    return;
  }
  provider.apiKeys.push({
    id: createApiKeyId(),
    name: sanitizeName(apiKeyName ?? ''),
    apiKey: normalized,
    createdAt: now,
    updatedAt: now,
  });
}

export async function createCodexModelProvider(input: {
  name: string;
  baseUrl: string;
  sourceTag?: string;
  modelCatalog?: string[];
  modelContextWindows?: Record<string, number>;
  supportsVision?: boolean;
  modelCapabilities?: Record<string, { supportsVision?: boolean }>;
  visionRoutingModel?: string;
  boundInstanceId?: string;
  website?: string;
  apiKeyUrl?: string;
  wireApi?: CodexProviderWireApi;
  supportsWebsockets?: boolean;
  enableModePreference?: CodexProviderEnableModePreference;
  integrationType?: 'sub2api' | 'new_api';
  boundOauthAccountId?: string | null;
  initialApiKey?: string;
  initialApiKeyName?: string;
}): Promise<CodexModelProvider> {
  const name = sanitizeName(input.name);
  const baseUrl = normalizeBaseUrlForStore(input.baseUrl);
  const normalizedBaseUrl = normalizeCodexModelProviderBaseUrl(baseUrl);
  if (!name) throw new Error('PROVIDER_NAME_REQUIRED');
  if (!normalizedBaseUrl) throw new Error('PROVIDER_BASE_URL_INVALID');
  const providers = await ensureProvidersLoaded();
  if (providers.some((item) => normalizeCodexModelProviderBaseUrl(item.baseUrl) === normalizedBaseUrl)) {
    throw new Error('PROVIDER_BASE_URL_EXISTS');
  }
  const now = Date.now();
  const wireApi = normalizeWireApi(input.wireApi);
  const provider: CodexModelProvider = {
    id: createProviderId(),
    name,
    baseUrl,
    sourceTag: sanitizeName(input.sourceTag ?? '') || undefined,
    integrationType: normalizeIntegrationType(input.integrationType),
    modelCatalog:
      normalizeModelCatalog(input.modelCatalog) ??
      presetModelCatalogForBaseUrl(baseUrl),
    modelContextWindows: normalizeModelContextWindows(
      input.modelContextWindows,
      normalizeModelCatalog(input.modelCatalog) ??
        presetModelCatalogForBaseUrl(baseUrl) ??
        [],
    ),
    supportsVision: input.supportsVision === true,
    modelCapabilities: normalizeModelCapabilities(input.modelCapabilities),
    visionRoutingModel: sanitizeName(input.visionRoutingModel ?? '') || undefined,
    boundInstanceId: normalizeBoundInstanceId(input.boundInstanceId),
    website: sanitizeName(input.website ?? '') || undefined,
    apiKeyUrl: sanitizeName(input.apiKeyUrl ?? '') || undefined,
    wireApi,
    supportsWebsockets: normalizeSupportsWebsockets(input.supportsWebsockets, wireApi, baseUrl),
    enableModePreference: normalizeEnableModePreference(input.enableModePreference),
    boundOauthAccountId: normalizeBoundOauthAccountId(input.boundOauthAccountId),
    apiKeys: [],
    createdAt: now,
    updatedAt: now,
  };
  enforceDeepSeekProvider(provider);
  if (input.initialApiKey) {
    ensureApiKeyOnProvider(provider, input.initialApiKey, input.initialApiKeyName);
  }
  providers.push(provider);
  await writeProviders(providers);
  return { ...provider, apiKeys: provider.apiKeys.map((apiKey) => ({ ...apiKey })) };
}

export async function updateCodexModelProvider(
  providerId: string,
  patch: {
    name?: string;
    baseUrl?: string;
    sourceTag?: string | null;
    modelCatalog?: string[] | null;
    modelContextWindows?: Record<string, number> | null;
    supportsVision?: boolean;
    modelCapabilities?: Record<string, { supportsVision?: boolean }> | null;
    visionRoutingModel?: string | null;
    boundInstanceId?: string | null;
    website?: string;
    apiKeyUrl?: string;
    wireApi?: CodexProviderWireApi | null;
    supportsWebsockets?: boolean;
    enableModePreference?: CodexProviderEnableModePreference | null;
    integrationType?: 'sub2api' | 'new_api' | null;
    boundOauthAccountId?: string | null;
  },
): Promise<CodexModelProvider> {
  const providers = await ensureProvidersLoaded();
  const provider = providers.find((item) => item.id === providerId);
  if (!provider) throw new Error('PROVIDER_NOT_FOUND');

  const nextName = patch.name === undefined ? provider.name : sanitizeName(patch.name);
  const nextBaseUrl =
    patch.baseUrl === undefined
      ? provider.baseUrl
      : normalizeBaseUrlForStore(patch.baseUrl);
  const normalizedBaseUrl = normalizeCodexModelProviderBaseUrl(nextBaseUrl);
  if (!nextName) throw new Error('PROVIDER_NAME_REQUIRED');
  if (!normalizedBaseUrl) throw new Error('PROVIDER_BASE_URL_INVALID');

  const duplicated = providers.find(
    (item) =>
      item.id !== providerId &&
      normalizeCodexModelProviderBaseUrl(item.baseUrl) === normalizedBaseUrl,
  );
  if (duplicated) throw new Error('PROVIDER_BASE_URL_EXISTS');

  provider.name = nextName;
  provider.baseUrl = nextBaseUrl;
  if (patch.sourceTag !== undefined) {
    provider.sourceTag =
      patch.sourceTag === null ? undefined : sanitizeName(patch.sourceTag) || undefined;
  }
  if (patch.modelCatalog !== undefined) {
    provider.modelCatalog =
      patch.modelCatalog === null
        ? presetModelCatalogForBaseUrl(nextBaseUrl)
        : normalizeModelCatalog(patch.modelCatalog);
  } else if (!provider.modelCatalog || provider.modelCatalog.length === 0) {
    provider.modelCatalog = presetModelCatalogForBaseUrl(nextBaseUrl);
  }
  if (patch.modelContextWindows !== undefined) {
    provider.modelContextWindows = normalizeModelContextWindows(
      patch.modelContextWindows,
      provider.modelCatalog ?? [],
    );
  } else {
    provider.modelContextWindows = normalizeModelContextWindows(
      provider.modelContextWindows,
      provider.modelCatalog ?? [],
    );
  }
  if (patch.supportsVision !== undefined) {
    provider.supportsVision = patch.supportsVision === true;
  }
  if (patch.modelCapabilities !== undefined) {
    provider.modelCapabilities =
      patch.modelCapabilities === null
        ? undefined
        : normalizeModelCapabilities(patch.modelCapabilities);
  }
  if (patch.visionRoutingModel !== undefined) {
    provider.visionRoutingModel =
      patch.visionRoutingModel === null
        ? undefined
        : sanitizeName(patch.visionRoutingModel) || undefined;
  }
  if (patch.boundInstanceId !== undefined) {
    provider.boundInstanceId =
      patch.boundInstanceId === null
        ? undefined
        : normalizeBoundInstanceId(patch.boundInstanceId);
  }
  if (patch.website !== undefined) {
    provider.website = sanitizeName(patch.website) || undefined;
  }
  if (patch.apiKeyUrl !== undefined) {
    provider.apiKeyUrl = sanitizeName(patch.apiKeyUrl) || undefined;
  }
  if (patch.wireApi !== undefined) {
    provider.wireApi =
      patch.wireApi === null ? undefined : normalizeWireApi(patch.wireApi);
  }
  if (patch.supportsWebsockets !== undefined || patch.wireApi !== undefined) {
    provider.supportsWebsockets = normalizeSupportsWebsockets(
      patch.supportsWebsockets ?? provider.supportsWebsockets,
      provider.wireApi ?? undefined,
      provider.baseUrl,
    );
  }
  if (patch.enableModePreference !== undefined) {
    provider.enableModePreference =
      patch.enableModePreference === null
        ? undefined
        : normalizeEnableModePreference(patch.enableModePreference);
  }
  if (patch.integrationType !== undefined) {
    provider.integrationType = normalizeIntegrationType(patch.integrationType);
  }
  if (patch.boundOauthAccountId !== undefined) {
    provider.boundOauthAccountId =
      patch.boundOauthAccountId === null
        ? undefined
        : normalizeBoundOauthAccountId(patch.boundOauthAccountId);
  }
  enforceDeepSeekProvider(provider);
  provider.updatedAt = Date.now();
  await writeProviders(providers);
  return { ...provider, apiKeys: provider.apiKeys.map((apiKey) => ({ ...apiKey })) };
}

export async function addApiKeyToCodexModelProvider(
  providerId: string,
  apiKey: string,
  apiKeyName?: string,
): Promise<CodexModelProvider> {
  const providers = await ensureProvidersLoaded();
  const provider = providers.find((item) => item.id === providerId);
  if (!provider) throw new Error('PROVIDER_NOT_FOUND');
  ensureApiKeyOnProvider(provider, apiKey, apiKeyName);
  provider.updatedAt = Date.now();
  await writeProviders(providers);
  return { ...provider, apiKeys: provider.apiKeys.map((item) => ({ ...item })) };
}

export async function removeApiKeyFromCodexModelProvider(
  providerId: string,
  apiKeyId: string,
): Promise<CodexModelProvider> {
  const providers = await ensureProvidersLoaded();
  const provider = providers.find((item) => item.id === providerId);
  if (!provider) throw new Error('PROVIDER_NOT_FOUND');
  const nextApiKeys = provider.apiKeys.filter((item) => item.id !== apiKeyId);
  if (nextApiKeys.length === provider.apiKeys.length) {
    return { ...provider, apiKeys: provider.apiKeys.map((item) => ({ ...item })) };
  }
  provider.apiKeys = nextApiKeys;
  provider.updatedAt = Date.now();
  await writeProviders(providers);
  return { ...provider, apiKeys: provider.apiKeys.map((item) => ({ ...item })) };
}

/** Explicit rename for an existing provider API key (#1510). Does not rewrite key material. */
export async function renameApiKeyOnCodexModelProvider(
  providerId: string,
  apiKeyId: string,
  name: string,
): Promise<CodexModelProvider> {
  const providers = await ensureProvidersLoaded();
  const provider = providers.find((item) => item.id === providerId);
  if (!provider) throw new Error('PROVIDER_NOT_FOUND');
  const apiKey = provider.apiKeys.find((item) => item.id === apiKeyId);
  if (!apiKey) throw new Error('API_KEY_NOT_FOUND');

  const now = Date.now();
  apiKey.name = sanitizeName(name);
  apiKey.updatedAt = now;
  provider.updatedAt = now;
  await writeProviders(providers);
  return { ...provider, apiKeys: provider.apiKeys.map((item) => ({ ...item })) };
}

/** Replace the secret for an existing provider API key without changing its id. */
export async function updateApiKeyOnCodexModelProvider(
  providerId: string,
  apiKeyId: string,
  apiKey: string,
  name?: string,
): Promise<CodexModelProvider> {
  const normalizedApiKey = sanitizeApiKey(apiKey);
  if (!normalizedApiKey) throw new Error("API_KEY_REQUIRED");

  const providers = await ensureProvidersLoaded();
  const provider = providers.find((item) => item.id === providerId);
  if (!provider) throw new Error("PROVIDER_NOT_FOUND");
  const existing = provider.apiKeys.find((item) => item.id === apiKeyId);
  if (!existing) throw new Error("API_KEY_NOT_FOUND");

  const duplicate = provider.apiKeys.some(
    (item) => item.id !== apiKeyId && sanitizeApiKey(item.apiKey) === normalizedApiKey,
  );
  if (duplicate) throw new Error("API_KEY_EXISTS");

  const now = Date.now();
  existing.apiKey = normalizedApiKey;
  if (name !== undefined) {
    existing.name = sanitizeName(name);
  }
  existing.updatedAt = now;
  provider.updatedAt = now;
  await writeProviders(providers);
  return { ...provider, apiKeys: provider.apiKeys.map((item) => ({ ...item })) };
}

export async function testCodexModelProviderConnection(input: {
  baseUrl: string;
  apiKey: string;
  wireApi?: CodexProviderWireApi | null;
}): Promise<CodexLocalAccessTestResult> {
  return await invoke('codex_test_model_provider_connection', {
    baseUrl: input.baseUrl,
    apiKey: input.apiKey,
    wireApi: input.wireApi ?? null,
  });
}

export interface CodexModelProviderChatTestTarget {
  providerId: string;
  providerName: string;
  baseUrl: string;
  apiKeyId?: string | null;
  apiKeyName?: string | null;
  apiKey: string;
  wireApi?: CodexProviderWireApi | null;
  modelCatalog?: string[];
}

export interface CodexModelProviderChatTestRecord {
  providerId: string;
  providerName: string;
  apiKeyId?: string | null;
  apiKeyName?: string | null;
  wireApi: CodexProviderWireApi;
  accessMode: 'direct' | 'gateway';
  modelId?: string | null;
  success: boolean;
  prompt: string;
  reply?: string | null;
  error?: string | null;
  durationMs?: number | null;
  timestamp: number;
}

export interface CodexModelProviderChatTestBatchResult {
  runId: string;
  records: CodexModelProviderChatTestRecord[];
  successCount: number;
  failureCount: number;
}

export interface CodexModelProviderChatTestProgressPayload {
  runId: string;
  total: number;
  completed: number;
  successCount: number;
  failureCount: number;
  running: boolean;
  phase:
    | 'batch_started'
    | 'provider_started'
    | 'provider_completed'
    | 'batch_completed'
    | 'batch_cancelled';
  currentProviderId?: string | null;
  item?: CodexModelProviderChatTestRecord | null;
}

export async function testCodexModelProviderChatBatch(input: {
  targets: CodexModelProviderChatTestTarget[];
  prompt?: string | null;
  model?: string | null;
  runId?: string | null;
}): Promise<CodexModelProviderChatTestBatchResult> {
  return await invoke('codex_model_provider_chat_test_batch', {
    targets: input.targets,
    prompt: input.prompt ?? null,
    model: input.model ?? null,
    runId: input.runId ?? null,
  });
}

export async function cancelCodexModelProviderChatTest(runId: string): Promise<boolean> {
  return await invoke('codex_cancel_model_provider_chat_test', { runId });
}

export async function queryCodexModelProviderUsage(input: {
  baseUrl: string;
  apiKey: string;
  integrationType?: 'sub2api' | 'new_api' | null;
}): Promise<CodexModelProviderUsageSummary> {
  return await queryModelProviderUsage(input);
}

export async function saveCodexModelProviderDetectedIntegrationType(
  providerId: string,
  integrationType: 'sub2api' | 'new_api',
): Promise<CodexModelProvider> {
  return updateCodexModelProvider(providerId, { integrationType });
}

export async function deleteCodexModelProvider(providerId: string): Promise<void> {
  const providers = await ensureProvidersLoaded();
  const next = providers.filter((item) => item.id !== providerId);
  if (next.length === providers.length) return;
  await writeProviders(next);
}

export async function upsertCodexModelProviderFromCredential(
  input: UpsertFromCredentialInput,
): Promise<CodexModelProvider> {
  const apiBaseUrl = normalizeBaseUrlForStore(input.apiBaseUrl);
  const normalizedBaseUrl = normalizeCodexModelProviderBaseUrl(apiBaseUrl);
  const apiKey = sanitizeApiKey(input.apiKey);
  if (!normalizedBaseUrl || !apiKey) {
    throw new Error('PROVIDER_CREDENTIAL_INVALID');
  }
  const providers = await ensureProvidersLoaded();
  let provider = findCodexModelProviderById(providers, input.providerId);
  if (!provider) {
    provider = findCodexModelProviderByBaseUrl(providers, apiBaseUrl);
  }

  if (!provider) {
    const now = Date.now();
    const wireApi = normalizeWireApi(input.wireApi);
    provider = {
      id: createProviderId(),
      name:
        sanitizeName(input.providerName ?? '') ||
        deriveProviderNameFromBaseUrl(apiBaseUrl),
      baseUrl: apiBaseUrl,
      sourceTag: sanitizeName(input.sourceTag ?? '') || undefined,
      modelCatalog:
        normalizeModelCatalog(input.modelCatalog) ??
        presetModelCatalogForBaseUrl(apiBaseUrl),
      supportsVision: input.supportsVision === true,
      modelCapabilities: normalizeModelCapabilities(input.modelCapabilities),
      visionRoutingModel: sanitizeName(input.visionRoutingModel ?? '') || undefined,
      integrationType: normalizeIntegrationType(input.integrationType),
      website: sanitizeName(input.website ?? '') || undefined,
      apiKeyUrl: sanitizeName(input.apiKeyUrl ?? '') || undefined,
      wireApi,
      supportsWebsockets: normalizeSupportsWebsockets(input.supportsWebsockets, wireApi, apiBaseUrl),
      enableModePreference: 'auto',
      boundOauthAccountId: undefined,
      apiKeys: [],
      createdAt: now,
      updatedAt: now,
    };
    providers.push(provider);
  } else if (input.providerName && sanitizeName(input.providerName)) {
    provider.name = sanitizeName(input.providerName);
    provider.updatedAt = Date.now();
  }

  if (input.sourceTag !== undefined) {
    provider.sourceTag = sanitizeName(input.sourceTag ?? '') || undefined;
  }

  const moveResult = moveCodexProviderApiKey(
    providers,
    input.previousProviderId,
    provider.id,
    apiKey,
  );
  if (moveResult === 'not_moved' || moveResult === 'name_conflict') {
    ensureApiKeyOnProvider(provider, apiKey, input.apiKeyName);
  }
  provider.baseUrl = apiBaseUrl;
  provider.modelCatalog =
    normalizeModelCatalog(input.modelCatalog) ??
    provider.modelCatalog ??
    presetModelCatalogForBaseUrl(apiBaseUrl);
  if (input.modelContextWindows !== undefined) {
    provider.modelContextWindows = normalizeModelContextWindows(
      input.modelContextWindows,
      provider.modelCatalog ?? [],
    );
  } else {
    provider.modelContextWindows = normalizeModelContextWindows(
      provider.modelContextWindows,
      provider.modelCatalog ?? [],
    );
  }
  if (input.supportsVision !== undefined) {
    provider.supportsVision = input.supportsVision === true;
  }
  if (input.modelCapabilities !== undefined) {
    provider.modelCapabilities = normalizeModelCapabilities(input.modelCapabilities);
  }
  if (input.visionRoutingModel !== undefined) {
    provider.visionRoutingModel = sanitizeName(input.visionRoutingModel ?? '') || undefined;
  }
  if (input.website !== undefined) {
    provider.website = sanitizeName(input.website ?? '') || undefined;
  }
  if (input.apiKeyUrl !== undefined) {
    provider.apiKeyUrl = sanitizeName(input.apiKeyUrl ?? '') || undefined;
  }
  if (input.wireApi !== undefined) {
    provider.wireApi = normalizeWireApi(input.wireApi);
  }
  if (input.supportsWebsockets !== undefined || input.wireApi !== undefined) {
    provider.supportsWebsockets = normalizeSupportsWebsockets(
      input.supportsWebsockets ?? provider.supportsWebsockets,
      provider.wireApi ?? undefined,
      provider.baseUrl,
    );
  }
  if (input.integrationType !== undefined) {
    provider.integrationType = normalizeIntegrationType(input.integrationType);
  }
  enforceDeepSeekProvider(provider);
  provider.updatedAt = Date.now();
  await writeProviders(providers);
  return { ...provider, apiKeys: provider.apiKeys.map((item) => ({ ...item })) };
}

function normalizeOptionalForCompare(value?: string | null): string {
  return value?.trim().toLowerCase() ?? '';
}

export function countCodexModelProviderReferences(
  provider: CodexModelProvider,
  accounts: CodexAccount[],
): number {
  const normalizedBaseUrl = normalizeCodexModelProviderBaseUrl(provider.baseUrl);
  if (!normalizedBaseUrl) return 0;
  return accounts.filter((account) => {
    if ((account.auth_mode ?? '').toLowerCase() !== 'apikey') return false;
    const accountBaseUrl = normalizeCodexModelProviderBaseUrl(account.api_base_url ?? '');
    if (!accountBaseUrl || accountBaseUrl !== normalizedBaseUrl) return false;
    return normalizeOptionalForCompare(account.openai_api_key).length > 0;
  }).length;
}
