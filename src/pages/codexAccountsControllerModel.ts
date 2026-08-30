import { isCodexPendingOAuthAccount, type CodexApiProviderMode, type CodexBatchDeleteJobStatus } from "../types/codex";
import type { CodexAccount } from "../types/codex";
import type { CodexLocalAccessAddressKind, CodexLocalAccessAccountHealth } from "../types/codexLocalAccess";
import { CODEX_OVERVIEW_FILTER_FIELDS, CODEX_OVERVIEW_FILTER_SCOPE } from "../utils/codexAccountOverview";
import { findCodexApiProviderPresetById } from "../utils/codexProviderPresets";
import { normalizeApiKeyFunOfficialUrl, resolveApiKeyFunWireApi } from "../utils/apikeyFunLinks";
import {
  type CodexModelProvider,
  type CodexModelProviderIntegrationType,
  type CodexModelProviderUsageSummary,
} from "../services/codexModelProviderService";
import type { Sponsor } from "../types/sponsor";
import { type MfaRecord } from "../utils/mfaVault";
import { type MailVerificationCodePreview } from "../utils/mailVerificationCode";









export const CODEX_TOKEN_SINGLE_EXAMPLE = `{
  "tokens": {
    "id_token": "eyJ...",
    "access_token": "eyJ...",
    "refresh_token": "rt_..."
  }
}`;
export const CODEX_TOKEN_SESSION_EXAMPLE = `{
  "user": {
    "email": "user@example.com"
  },
  "account": {
    "id": "account-id"
  },
  "accessToken": "eyJ...",
  "authProvider": "openai"
}

{
  "refresh_token": "rt_..."
}

at-your-personal-access-token

{
  "personal_access_token": "at-...",
  "account_id": "workspace-uuid"
}`;
export const CODEX_TOKEN_BATCH_EXAMPLE = `[
  {
    "id": "codex_demo_1",
    "email": "user@example.com",
    "tokens": {
      "id_token": "eyJ...",
      "access_token": "eyJ...",
      "refresh_token": "rt_..."
    },
    "created_at": 1730000000,
    "last_used": 1730000000
  }
]`;
export const OPENAI_OFFICIAL_PRESET_ID = "openai_official";
export const OPENAI_OFFICIAL_BASE_URL = "https://api.openai.com/v1";
export function parseOAuthQuotaReservePercent(value: string): number | null {
  const normalized = value.trim();
  if (!/^(?:[1-9]\d?|100)$/.test(normalized)) {
    return null;
  }
  return Number(normalized);
}

export function normalizeCodexApiBaseUrl(rawValue?: string | null): string {
  return normalizeHttpBaseUrl(rawValue ?? "") ?? "";
}

export function inferCodexAccountProviderMode(
  account: CodexAccount,
): CodexApiProviderMode {
  if (
    account.api_provider_mode === "custom" ||
    account.api_provider_mode === "openai_builtin"
  ) {
    return account.api_provider_mode;
  }
  const normalizedBaseUrl = normalizeCodexApiBaseUrl(account.api_base_url);
  if (!normalizedBaseUrl || normalizedBaseUrl === "https://api.openai.com/v1") {
    return "openai_builtin";
  }
  return "custom";
}
export const CODEX_OVERVIEW_LAYOUT_MODE_KEY =
  "agtools.codex.accounts.overview_layout_mode";
export const CODEX_HIDE_RELAY_QUOTA_LEGACY_KEY =
  "agtools.codex.accounts.hide_relay_quota.v1";
export const CODEX_LOCAL_ACCESS_EXPANDED_KEY =
  "agtools.codex.local_access_entry_expanded.v1";
export const CODEX_LOCAL_ACCESS_ADDRESS_KIND_KEY =
  "agtools.codex.local_access_address_kind.v1";
export const DEFAULT_CODEX_API_PROVIDER_ID = OPENAI_OFFICIAL_PRESET_ID;
export const DEFAULT_CODEX_API_BASE_URL = OPENAI_OFFICIAL_BASE_URL;
export const CODEX_LOCAL_ACCESS_FALLBACK_PORT = 54140;
export const CODEX_LOCAL_ACCESS_FALLBACK_BASE_URL = `http://127.0.0.1:${CODEX_LOCAL_ACCESS_FALLBACK_PORT}/v1`;
export const CODEX_LOCAL_ACCESS_FALLBACK_API_KEY_MASK = "agt_codex_••••••••••••";
export const CODEX_FILTER_PERSISTENCE_SCOPE = CODEX_OVERVIEW_FILTER_SCOPE;
export const SEARCH_QUERY_FIELD = CODEX_OVERVIEW_FILTER_FIELDS.searchQuery;
export const FILTER_TYPES_FIELD = CODEX_OVERVIEW_FILTER_FIELDS.filterTypes;
export const EXPIRY_FILTER_FIELD = CODEX_OVERVIEW_FILTER_FIELDS.expiryFilter;
export const GROUP_FILTER_FIELD = CODEX_OVERVIEW_FILTER_FIELDS.groupFilter;
export const ACTIVE_GROUP_ID_FIELD = CODEX_OVERVIEW_FILTER_FIELDS.activeGroupId;
export const OAUTH_BINDING_PAGE_SIZE_OPTIONS = [10, 20, 50] as const;

export type CodexOverviewLayoutMode = "compact" | "list" | "grid";
export type OAuthBindingTargetKind = "api_key_account" | "local_access";
export type OAuthBindingQuotaReserveFieldErrors = {
  hourlyPercent?: string;
  weeklyPercent?: string;
};
export type CodexAccountNoteFormState = {
  note: string;
  twoFactorSecret: string;
  accountPassword: string;
  phoneNumber: string;
  mailUrl: string;
  chatgptAccountId: string;
};

export type CodexAccountNoteMailPreviewState = MailVerificationCodePreview & {
  fetchedAt: number;
  truncated: boolean;
  status: "initial" | "changed" | "unchanged";
};

export type CodexAccountNoteMailPreviewSnapshot = {
  mailUrl: string;
  code: string;
};

export type CodexAccountNoteFieldErrors = {
  twoFactorSecret?: string;
};

export const EMPTY_CODEX_ACCOUNT_NOTE_FORM: CodexAccountNoteFormState = {
  note: "",
  twoFactorSecret: "",
  accountPassword: "",
  phoneNumber: "",
  mailUrl: "",
  chatgptAccountId: "",
};

export function buildCodexAccountNoteForm(
  account?: CodexAccount | null,
): CodexAccountNoteFormState {
  return {
    note: account?.account_note ?? "",
    twoFactorSecret: account?.two_factor_secret ?? "",
    accountPassword: account?.account_password ?? "",
    phoneNumber: account?.phone_number ?? "",
    mailUrl: account?.mail_url ?? "",
    chatgptAccountId: account?.account_id ?? "",
  };
}

export function hasCodexAccountNoteDetails(account?: CodexAccount | null): boolean {
  return Boolean(
    account?.account_note?.trim() ||
    account?.two_factor_secret?.trim() ||
    account?.account_password?.trim() ||
    account?.phone_number?.trim() ||
    account?.mail_url?.trim(),
  );
}

export function hasCodexAccountNoteFormDetails(
  form?: CodexAccountNoteFormState | null,
): boolean {
  return Boolean(
    form?.note.trim() ||
    form?.twoFactorSecret.trim() ||
    form?.accountPassword.trim() ||
    form?.phoneNumber.trim() ||
    form?.mailUrl.trim(),
  );
}

export function getCodexAccountNoteTitle(
  account: CodexAccount,
  fallback: string,
): string {
  return (
    account.account_note?.trim() ||
    account.two_factor_secret?.trim() ||
    account.account_password?.trim() ||
    account.phone_number?.trim() ||
    account.mail_url?.trim() ||
    fallback
  );
}

export function formatCodexAccountNoteMailPreviewTime(timestamp: number): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}

export function formatMfaRecordOption(record: MfaRecord, fallback: string): string {
  const accountName = record.accountName.trim();
  if (accountName) return accountName;
  const secret = record.secret.trim();
  if (!secret) return fallback;
  if (secret.length <= 14) return secret;
  return `${secret.slice(0, 6)}...${secret.slice(-4)}`;
}

export function formatMfaSecretPreview(secret: string): string {
  const trimmed = secret.trim();
  if (!trimmed) return "";
  if (trimmed.length <= 12) return trimmed;
  return `${trimmed.slice(0, 5)}...${trimmed.slice(-4)}`;
}

export function isPendingOAuthCodexAccount(account?: CodexAccount | null): boolean {
  return isCodexPendingOAuthAccount(account);
}

export function isSponsorModelProvider(
  provider: CodexModelProvider | null | undefined,
  sponsorTemplates: SponsorApiProviderTemplate[],
): boolean {
  if (!provider) return false;
  if (provider.sourceTag) {
    return sponsorTemplates.some(
      (template) => template.id === provider.sourceTag,
    );
  }
  const normalizedBaseUrl = normalizeHttpBaseUrl(provider.baseUrl);
  if (!normalizedBaseUrl) return false;
  return sponsorTemplates.some(
    (template) => normalizeHttpBaseUrl(template.baseUrl) === normalizedBaseUrl,
  );
}

export interface LocalAccessAccountPoolHealthSummary {
  total: number;
  available: number;
  abnormal: number;
  cooldown: number;
  missing: number;
  authError: number;
  quotaLimited: number;
  poolUnavailable: number;
}

export const ABNORMAL_LOCAL_ACCESS_ACCOUNT_FAILURE_CATEGORIES = new Set([
  "auth_unavailable",
  "auth_refresh_failed",
  "account_prepare_failed",
]);

export function isAbnormalLocalAccessAccountFailure(
  health?: CodexLocalAccessAccountHealth,
): boolean {
  return Boolean(
    health &&
    ((health.schedulerAvailable === false && !health.cooldowns.length) ||
      (health.consecutiveFailures >= 3 &&
        health.lastFailureCategory &&
        ABNORMAL_LOCAL_ACCESS_ACCOUNT_FAILURE_CATEGORIES.has(
          health.lastFailureCategory,
        ))),
  );
}

export function normalizeLocalAccessAddressKind(
  value: string | null | undefined,
): CodexLocalAccessAddressKind {
  return value === "lan" ? "lan" : "local";
}

export function readStoredLocalAccessAddressKind(): CodexLocalAccessAddressKind {
  try {
    return normalizeLocalAccessAddressKind(
      localStorage.getItem(CODEX_LOCAL_ACCESS_ADDRESS_KIND_KEY),
    );
  } catch {
    return "local";
  }
}

export function persistLocalAccessAddressKind(
  value: CodexLocalAccessAddressKind,
): void {
  try {
    localStorage.setItem(CODEX_LOCAL_ACCESS_ADDRESS_KIND_KEY, value);
  } catch {
    // ignore storage write failures
  }
}

export const CODEX_BATCH_IMPORT_SESSION_STORAGE_KEY =
  "cockpit.codex.batchImport.sessionId";

export type CodexBatchImportFilter = "all" | "ready";

export function shouldAutoHideBatchDeleteJob(
  job: CodexBatchDeleteJobStatus | null,
): job is CodexBatchDeleteJobStatus & { status: "completed" } {
  return job?.status === "completed" && job.failed === 0;
}

export type CockpitApiJsonRecord = Record<string, unknown>;

export function toCockpitApiRecord(value: unknown): CockpitApiJsonRecord | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as CockpitApiJsonRecord)
    : null;
}

export function readCockpitApiString(
  record: CockpitApiJsonRecord | null,
  key: string,
): string {
  const value = record?.[key];
  return typeof value === "string" ? value.trim() : "";
}

export function readCockpitApiNumber(
  record: CockpitApiJsonRecord | null,
  key: string,
): number {
  const value = record?.[key];
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

export function readCockpitApiOptionalNumber(
  record: CockpitApiJsonRecord | null,
  key: string,
): number | null {
  const value = record?.[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function formatCockpitApiInteger(value: number): string {
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(
    Math.max(0, value),
  );
}

export function formatCockpitApiTokenCount(value: number): string {
  const normalized = Math.max(0, value);
  if (normalized >= 100_000_000) {
    return `${(normalized / 100_000_000).toFixed(normalized >= 1_000_000_000 ? 1 : 2).replace(/\.?0+$/, "")}亿`;
  }
  if (normalized >= 10_000) {
    return `${(normalized / 10_000).toFixed(normalized >= 100_000 ? 1 : 2).replace(/\.?0+$/, "")}万`;
  }
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(
    normalized,
  );
}

export function getCockpitApiUsageRecord(
  account: CodexAccount,
): CockpitApiJsonRecord | null {
  const raw = toCockpitApiRecord(account.quota?.raw_data);
  const profile = toCockpitApiRecord(raw?.profile);
  return toCockpitApiRecord(raw?.usage) ?? toCockpitApiRecord(profile?.usage);
}

export function getCockpitApiStatsRecord(
  account: CodexAccount,
): CockpitApiJsonRecord | null {
  const usage = getCockpitApiUsageRecord(account);
  return toCockpitApiRecord(usage?.stats);
}

export function resolveApiKeyUsageMode(
  summary?: CodexModelProviderUsageSummary,
):
  | "new_api"
  | "sub2api"
  | "cockpit_tools"
  | "deepseek"
  | "token_plan"
  | null {
  if (!summary) return null;
  if (
    summary.mode === "new_api" ||
    summary.mode === "sub2api" ||
    summary.mode === "cockpit_tools" ||
    summary.mode === "deepseek" ||
    summary.mode === "token_plan"
  ) {
    return summary.mode;
  }
  if (
    typeof summary.todayRequests === "number" ||
    typeof summary.todayTotalTokens === "number"
  ) {
    return "sub2api";
  }
  const detailKeys = new Set((summary.details ?? []).map((item) => item.key));
  if (
    detailKeys.has("todayRequests") ||
    detailKeys.has("todayTokens") ||
    detailKeys.has("remaining")
  ) {
    return "sub2api";
  }
  if (
    detailKeys.has("totalGranted") ||
    detailKeys.has("totalAvailable") ||
    detailKeys.has("expiresAt")
  ) {
    return "new_api";
  }
  return null;
}

export interface CodexOverviewGeneralConfig {
  codex_local_access_entry_visible?: boolean;
  codex_hide_relay_quota?: boolean;
}

export function normalizeCodexOverviewLayoutMode(
  value: string | null,
): CodexOverviewLayoutMode | null {
  if (value === "compact" || value === "list" || value === "grid") return value;
  return null;
}

export function isHttpLikeUrl(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;
  try {
    const parsed = new URL(trimmed);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    const lower = trimmed.toLowerCase();
    return lower.startsWith("http://") || lower.startsWith("https://");
  }
}

export function normalizeHttpBaseUrl(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:")
      return null;
    return trimmed.replace(/\/+$/, "");
  } catch {
    return null;
  }
}

export function isSameHttpBaseUrl(left: string, right: string): boolean {
  const normalizedLeft = normalizeHttpBaseUrl(left)?.toLowerCase();
  const normalizedRight = normalizeHttpBaseUrl(right)?.toLowerCase();
  return Boolean(
    normalizedLeft && normalizedRight && normalizedLeft === normalizedRight,
  );
}

export function buildExportFileName(baseName: string): string {
  const date = new Date().toISOString().slice(0, 10);
  return `${baseName}_${date}.json`;
}

export function getDirectoryPath(filePath: string): string {
  const slashIndex = Math.max(
    filePath.lastIndexOf("/"),
    filePath.lastIndexOf("\\"),
  );
  if (slashIndex <= 0) {
    return filePath;
  }
  return filePath.slice(0, slashIndex);
}

export function joinFilePath(directory: string, fileName: string): string {
  if (!directory) return fileName;
  const separator = directory.includes("\\") ? "\\" : "/";
  return directory.endsWith("/") || directory.endsWith("\\")
    ? `${directory}${fileName}`
    : `${directory}${separator}${fileName}`;
}

export function normalizePathForCompare(value?: string | null): string {
  return (value || "").trim().replace(/[\\/]+$/, "");
}

export function sanitizeCodexCliInstanceName(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return "Codex CLI";
  return trimmed
    .replace(/[\\/:*?"<>|]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export interface CodexCliLaunchModalState {
  target: "account" | "apiService";
  accountId: string;
  bindAccountId: string;
  accountLabel: string;
  instanceId: string | null;
  instanceDraft: CodexCliInstanceDraft | null;
  instanceName: string;
  workingDir: string;
  workingDirError: string | null;
  launchCommand: string;
  terminalCommand: string;
  runtimePrepared: boolean;
  preparing: boolean;
  copied: boolean;
  executing: boolean;
  executeMessage: string | null;
  executeError: string | null;
}

export interface CodexCliInstanceDraft {
  name: string;
  userDataDir: string;
  workingDir: string;
  extraArgs: string;
  bindAccountId: string;
}

export const CODEX_CLI_LAST_WORKING_DIR_KEY = "cockpit.codex.cli.lastWorkingDir";

export function readLastCodexCliWorkingDir(): string {
  try {
    return localStorage.getItem(CODEX_CLI_LAST_WORKING_DIR_KEY)?.trim() || "";
  } catch {
    return "";
  }
}

export function persistLastCodexCliWorkingDir(value: string): void {
  const workingDir = value.trim();
  if (!workingDir) return;
  try {
    localStorage.setItem(CODEX_CLI_LAST_WORKING_DIR_KEY, workingDir);
  } catch {
    // 工作目录记忆失败不影响 CLI 启动。
  }
}

export function maskCodexApiKey(value: string): string {
  const raw = value.trim();
  if (!raw) return raw;
  if (raw.startsWith("sk-")) return "sk-••••••••••••••••";
  return "••••••••••••••••";
}

/** Distinguish multi-keys under one provider in select UI (name + masked tail). */
export function formatCodexManagedApiKeyOptionLabel(
  apiKey: { name?: string | null; apiKey: string },
  unnamedLabel: string,
): string {
  const name = apiKey.name?.trim() || unnamedLabel;
  const raw = apiKey.apiKey.trim();
  if (!raw) return name;
  if (raw.length <= 8) {
    return `${name}（${raw.slice(0, 2)}****）`;
  }
  return `${name}（${raw.slice(0, 4)}…${raw.slice(-4)}）`;
}

export function parseApiModelCatalogText(value: string): string[] {
  const seen = new Set<string>();
  const models: string[] = [];
  value
    .split(/[\n,]+/)
    .map((item) => item.trim())
    .filter(Boolean)
    .forEach((model) => {
      const key = model.toLowerCase();
      if (seen.has(key)) return;
      seen.add(key);
      models.push(model);
    });
  return models;
}

export interface SponsorApiProviderTemplate {
  id: string;
  sponsor: Sponsor;
  name: string;
  baseUrl: string;
  modelCatalog: string[];
  supportsVision: boolean;
  website: string;
  apiKeyUrl: string;
  wireApi?: "responses" | "chat_completions" | null;
  integrationType?: CodexModelProviderIntegrationType | null;
}

export function normalizeSponsorApiProviderTemplates(
  sponsors: Sponsor[] | undefined,
): SponsorApiProviderTemplate[] {
  const templates: SponsorApiProviderTemplate[] = [];
  for (const sponsor of sponsors ?? []) {
    const integration = sponsor.integration;
    if (
      !integration?.enabled ||
      !integration.quickConfigure ||
      !integration.baseUrl?.trim()
    ) {
      continue;
    }
    templates.push({
      id: `relay:${sponsor.id}`,
      sponsor,
      name: sponsor.name,
      baseUrl: integration.baseUrl.trim(),
      modelCatalog: integration.models ?? [],
      supportsVision: integration.supportsVision === true,
      website: normalizeApiKeyFunOfficialUrl(
        integration.website || sponsor.url,
      ),
      apiKeyUrl: normalizeApiKeyFunOfficialUrl(
        integration.apiKeyUrl || sponsor.url,
      ),
      wireApi: resolveApiKeyFunWireApi(
        integration.baseUrl,
        integration.wireApi ?? null,
      ),
      integrationType: integration.type ?? null,
    });
  }
  return templates.sort((a, b) => {
    const priority = a.sponsor.priority - b.sponsor.priority;
    if (priority !== 0) return priority;
    return a.name.localeCompare(b.name);
  });
}

export function isRelayApiProviderTemplateId(value?: string | null): boolean {
  return Boolean(value?.startsWith("relay:"));
}

export function getDefaultApiProviderPresetId(
  sponsorTemplates: SponsorApiProviderTemplate[],
): string {
  return sponsorTemplates[0]?.id ?? DEFAULT_CODEX_API_PROVIDER_ID;
}

export function resolveApiProviderPresetDefaults(
  providerId: string,
  sponsorTemplates: SponsorApiProviderTemplate[],
): { baseUrl: string; providerName: string } {
  const sponsorTemplate = sponsorTemplates.find(
    (template) => template.id === providerId,
  );
  if (sponsorTemplate) {
    return {
      baseUrl: sponsorTemplate.baseUrl,
      providerName: sponsorTemplate.name,
    };
  }
  const preset = findCodexApiProviderPresetById(providerId);
  return {
    baseUrl: preset?.baseUrls[0] ?? DEFAULT_CODEX_API_BASE_URL,
    providerName: "",
  };
}
