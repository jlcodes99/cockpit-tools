import { invoke } from '@tauri-apps/api/core';
import type { CodexAccount } from '../types/codex';

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
  website?: string;
  apiKeyUrl?: string;
  apiKeys: CodexModelProviderApiKey[];
  createdAt: number;
  updatedAt: number;
}

interface UpsertFromCredentialInput {
  providerId?: string | null;
  providerName?: string | null;
  apiBaseUrl: string;
  apiKey: string;
  apiKeyName?: string | null;
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
    providers.push({
      id: String((item as { id?: unknown }).id ?? createProviderId()),
      name,
      baseUrl,
      website: sanitizeName(String((item as { website?: unknown }).website ?? '')) || undefined,
      apiKeyUrl: sanitizeName(String((item as { apiKeyUrl?: unknown }).apiKeyUrl ?? '')) || undefined,
      apiKeys: toValidApiKeys((item as { apiKeys?: unknown }).apiKeys, now),
      createdAt: Number((item as { createdAt?: unknown }).createdAt ?? now),
      updatedAt: Number((item as { updatedAt?: unknown }).updatedAt ?? now),
    });
  }
  return providers.sort((a, b) => a.createdAt - b.createdAt);
}

async function loadProvidersFromDisk(): Promise<CodexModelProvider[]> {
  const raw = await invoke<string>('load_codex_model_providers');
  const parsed = JSON.parse(raw);
  return toValidProviderList(parsed);
}

async function saveProvidersToDisk(providers: CodexModelProvider[]): Promise<void> {
  await invoke('save_codex_model_providers', {
    data: JSON.stringify(providers, null, 2),
  });
}

async function ensureProvidersLoaded(): Promise<CodexModelProvider[]> {
  if (cachedProviders !== null) return cloneProviders(cachedProviders);
  const loadedProviders = await loadProvidersFromDisk().catch(() => []);
  const loaded = loadedProviders.filter((provider) => {
    // 兼容清理：移除旧版本自动注入但未配置 API Key 的默认预设项
    if (provider.id.startsWith('preset_') && provider.apiKeys.length === 0) {
      return false;
    }
    return true;
  });
  if (loaded.length !== loadedProviders.length) {
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
    if (apiKeyName && sanitizeName(apiKeyName)) {
      existing.name = sanitizeName(apiKeyName);
    }
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
  website?: string;
  apiKeyUrl?: string;
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
  const provider: CodexModelProvider = {
    id: createProviderId(),
    name,
    baseUrl,
    website: sanitizeName(input.website ?? '') || undefined,
    apiKeyUrl: sanitizeName(input.apiKeyUrl ?? '') || undefined,
    apiKeys: [],
    createdAt: now,
    updatedAt: now,
  };
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
    website?: string;
    apiKeyUrl?: string;
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
  if (patch.website !== undefined) {
    provider.website = sanitizeName(patch.website) || undefined;
  }
  if (patch.apiKeyUrl !== undefined) {
    provider.apiKeyUrl = sanitizeName(patch.apiKeyUrl) || undefined;
  }
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
    provider = {
      id: createProviderId(),
      name:
        sanitizeName(input.providerName ?? '') ||
        deriveProviderNameFromBaseUrl(apiBaseUrl),
      baseUrl: apiBaseUrl,
      apiKeys: [],
      createdAt: now,
      updatedAt: now,
    };
    providers.push(provider);
  } else if (input.providerName && sanitizeName(input.providerName)) {
    provider.name = sanitizeName(input.providerName);
    provider.updatedAt = Date.now();
  }

  ensureApiKeyOnProvider(provider, apiKey, input.apiKeyName);
  provider.baseUrl = apiBaseUrl;
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
