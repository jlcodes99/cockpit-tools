import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import {
  AlertTriangle,
  Check,
  CheckCircle,
  ChevronDown,
  CalendarDays,
  CircleAlert,
  Clock3,
  Copy,
  Database,
  Download,
  ExternalLink,
  Eye,
  EyeOff,
  FileText,
  FileJson,
  FolderOpen,
  Globe,
  KeyRound,
  Layers,
  LayoutGrid,
  List,
  Monitor,
  Pencil,
  Play,
  Plus,
  RefreshCw,
  RotateCw,
  Search,
  Star,
  Tag,
  Terminal,
  Trash2,
  Upload,
  X,
} from 'lucide-react';
import { open as openFileDialog } from '@tauri-apps/plugin-dialog';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import type { TFunction } from 'i18next';
import md5 from 'blueimp-md5';
import { ModalErrorMessage, useModalErrorState } from '../components/ModalErrorMessage';
import { ExportJsonModal } from '../components/ExportJsonModal';
import { ManualHelpIconButton } from '../components/ManualHelpIconButton';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import { SingleSelectDropdown } from '../components/SingleSelectDropdown';
import { TagEditModal } from '../components/TagEditModal';
import { ClaudeIcon } from '../components/icons/ClaudeIcon';
import { ModelProviderUsagePanel } from '../components/model-provider/ModelProviderUsagePanel';
import { PlatformGroupSwitcher } from '../components/platform/PlatformGroupSwitcher';
import { useEscClose } from '../hooks/useEscClose';
import { useExportJsonModal } from '../hooks/useExportJsonModal';
import { useLaunchTerminalOptions } from '../hooks/useLaunchTerminalOptions';
import { getProviderCurrentAccountId, type ProviderCurrentPlatform } from '../services/providerCurrentAccountService';
import {
  isModelProviderUsageUnavailableError,
  queryModelProviderUsage,
  type ModelProviderUsageSummary,
} from '../services/modelProviderUsageService';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';
import { isPrivacyModeEnabledByDefault, maskSensitiveValue, persistPrivacyModeEnabled } from '../utils/privacy';
import * as claudeService from '../services/claudeService';
import * as claudeInstanceService from '../services/claudeInstanceService';
import { useClaudeAccountStore } from '../stores/useClaudeAccountStore';
import { useClaudeInstanceStore } from '../stores/useClaudeInstanceStore';
import {
  findGroupByPlatform,
  resolveGroupChildName,
  usePlatformLayoutStore,
} from '../stores/usePlatformLayoutStore';
import { useRemoteConfigStore } from '../stores/useRemoteConfigStore';
import type {
  ClaudeAccount,
  ClaudeDesktopGatewayConnectionMode,
  ClaudeDesktopGatewayModelMapping,
  ClaudeDesktopLoginStartResponse,
  ClaudeDesktopLoginProgressPayload,
  ClaudeOAuthStartResponse,
} from '../types/claude';
import { isMenuVisiblePlatform, type PlatformId } from '../types/platform';
import {
  formatClaudeResetTime,
  getClaudeAccountDisplayEmail,
  getClaudeApiProviderLabel,
  getClaudeAuthModeLabel,
  getClaudePlanBadge,
  getClaudePlanBadgeClass,
  getClaudeQuotaClass,
  isClaudeDesktopGatewayAccount,
  isClaudeDesktopOAuthAccount,
  isClaudeDesktopRuntimeAccount,
  normalizeClaudeAuthMode,
} from '../types/claude';
import {
  CLAUDE_APIKEY_FUN_BASE_URL,
  CLAUDE_APIKEY_FUN_PROVIDER_ID,
  CLAUDE_API_PROVIDER_CUSTOM_ID,
  CLAUDE_API_PROVIDER_PRESETS,
  getDefaultClaudeApiProviderPresetId,
  findClaudeApiProviderPresetById,
  inferClaudeApiKeyField,
  normalizeClaudeApiProviderBaseUrl,
  type ClaudeApiProviderPreset,
} from '../utils/claudeProviderPresets';
import {
  CLAUDE_DESKTOP_GATEWAY_DEFAULT_MODELS,
  CLAUDE_DESKTOP_GATEWAY_PROVIDER_CUSTOM_ID,
  CLAUDE_DESKTOP_GATEWAY_PROVIDER_PRESETS,
  findClaudeDesktopGatewayProviderPresetById,
  getDefaultClaudeDesktopGatewayProviderPresetId,
  inferClaudeDesktopGatewayApiKeyField,
} from '../utils/claudeDesktopProviderPresets';
import {
  APIKEY_FUN_PREFILL_EVENT,
  consumeApiKeyFunPrefill,
  type ApiKeyFunPrefillPayload,
} from '../utils/apiKeyFunPrefill';
import { getPlatformLabel } from '../utils/platformMeta';
import { ClaudeInstancesContent } from './ClaudeInstancesPage';
import type { InstanceProfile } from '../types/instance';
import { AggregateQuotaProjectionChart } from '../components/AggregateQuotaProjectionChart';

const CLAUDE_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.claude.flow_notice_collapsed';
const CLAUDE_ACCOUNTS_VIEW_MODE_KEY = 'agtools.claude.accounts_view_mode';
const CLAUDE_API_KEY_USAGE_CACHE_KEY = 'agtools.claude.apiKeyUsage.cache.v1';
const CLAUDE_CLI_LAST_WORKING_DIR_KEY = 'agtools.claude.cli.last_working_dir';
const CLAUDE_API_KEY_USAGE_REFRESH_THROTTLE_MS = 10 * 1000;
const CLAUDE_DESKTOP_LOGIN_PROGRESS_EVENT = 'claude:desktop-login-progress';
const claudeApiKeyUsageInFlight = new Set<string>();
const claudeApiKeyUsageManualRefreshAt: Record<string, number> = {};

function isClaudeCloudflareError(message?: string | null): boolean {
  const normalized = message?.toLowerCase() ?? '';
  return (
    normalized.includes('cloudflare') ||
    normalized.includes('just a moment') ||
    normalized.includes('cf-ray') ||
    normalized.includes('challenge-platform') ||
    normalized.includes('verify you are human') ||
    normalized.includes('checking your browser')
  );
}

function createClaudeDesktopLoginProgressId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `claude-login-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function clampProgressPercent(value?: number | null): number {
  if (typeof value !== 'number' || Number.isNaN(value)) return 0;
  return Math.max(0, Math.min(100, Math.round(value)));
}

function formatProgressBytes(value?: number | null): string | null {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) return null;
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB`;
  if (value >= 1024) return `${Math.round(value / 1024)} KB`;
  return `${Math.round(value)} B`;
}

function getClaudeDesktopLoginProgressLabel(
  t: TFunction,
  progress?: ClaudeDesktopLoginProgressPayload | null,
): string {
  switch (progress?.phase) {
    case 'start':
      return t('claude.desktopOAuth.progress.start', '准备登录任务');
    case 'profile':
      return t('claude.desktopOAuth.progress.profile', '创建隔离 profile');
    case 'resolve-runtime':
      return t('claude.desktopOAuth.progress.resolveRuntime', '查找登录组件');
    case 'check-cache':
      return t('claude.desktopOAuth.progress.checkCache', '检查本地组件缓存');
    case 'cached':
      return t('claude.desktopOAuth.progress.cached', '使用本地组件缓存');
    case 'download-start':
      return t('claude.desktopOAuth.progress.downloadStart', '开始下载登录组件');
    case 'downloading':
      return t('claude.desktopOAuth.progress.downloading', '下载登录组件');
    case 'downloaded':
      return t('claude.desktopOAuth.progress.downloaded', '登录组件下载完成');
    case 'verify':
      return t('claude.desktopOAuth.progress.verify', '校验组件完整性');
    case 'extract':
      return t('claude.desktopOAuth.progress.extract', '解压登录组件');
    case 'runtime-ready':
      return t('claude.desktopOAuth.progress.runtimeReady', '登录组件已准备');
    case 'launch':
      return t('claude.desktopOAuth.progress.launch', '启动 Claude 登录窗口');
    case 'ready':
      return t('claude.desktopOAuth.progress.ready', '登录窗口已打开');
    default:
      return t('claude.desktopOAuth.progress.default', '准备登录组件');
  }
}

function getClaudeDesktopLoginProgressDetail(
  t: TFunction,
  progress?: ClaudeDesktopLoginProgressPayload | null,
): string {
  const downloaded = formatProgressBytes(progress?.downloadedBytes);
  const total = formatProgressBytes(progress?.totalBytes);
  if (progress?.phase === 'downloading' || progress?.phase === 'downloaded') {
    if (downloaded && total) {
      return t('claude.desktopOAuth.progress.bytesWithTotal', '已下载 {{downloaded}}，共 {{total}}', {
        downloaded,
        total,
      });
    }
    if (downloaded) {
      return t('claude.desktopOAuth.progress.bytesOnly', '已下载 {{downloaded}}', { downloaded });
    }
  }
  return t('claude.desktopOAuth.progress.hint', '首次准备完成后，后续会直接复用本地缓存。');
}

type ViewMode = 'grid' | 'list';
type AddTab = 'desktop' | 'desktopGateway' | 'oauth' | 'apikey' | 'import';
type ClaudeSubPlatform = 'desktop' | 'cli';
type ClaudePageSection = ClaudeSubPlatform | 'instances';
const DEFAULT_CLAUDE_API_PROVIDER_ID = getDefaultClaudeApiProviderPresetId();
const DEFAULT_CLAUDE_API_PROVIDER = findClaudeApiProviderPresetById(DEFAULT_CLAUDE_API_PROVIDER_ID);
const DEFAULT_CLAUDE_DESKTOP_GATEWAY_PROVIDER_ID = getDefaultClaudeDesktopGatewayProviderPresetId();
const DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS = [...CLAUDE_DESKTOP_GATEWAY_DEFAULT_MODELS];
const CLAUDE_DESKTOP_GATEWAY_GENERATED_ROUTE_BASE = 'claude-sonnet-4-6';
const CLAUDE_DESKTOP_GATEWAY_CUSTOM_DESKTOP_MODEL = '__custom_desktop_model__';

type ClaudeApiKeyUsageState = {
  loading: boolean;
  summary?: ModelProviderUsageSummary;
  error?: string;
  unavailable?: boolean;
  updatedAt?: number;
};

interface ClaudeAccountsPageProps {
  subPlatform?: ClaudeSubPlatform;
}

interface DeleteConfirmState {
  accountIds: string[];
  email: string;
}

interface ClaudeCliLaunchModalState {
  accountId: string;
  accountEmail: string;
  instanceId: string | null;
  workingDir: string;
  instanceName: string;
  launchCommand: string;
  preparing: boolean;
  copied: boolean;
  executing: boolean;
  executeMessage: string | null;
  executeError: string | null;
}

function joinFilePath(directory: string, fileName: string): string {
  if (!directory) return fileName;
  const separator = directory.includes('\\') ? '\\' : '/';
  return directory.endsWith(separator) ? `${directory}${fileName}` : `${directory}${separator}${fileName}`;
}

function normalizePathForCompare(value?: string | null): string {
  return (value || '').trim();
}

function sanitizeClaudeCliInstanceName(value: string): string {
  return value.replace(/[\\/:*?"<>|]/g, ' ').replace(/\s+/g, ' ').trim() || 'Claude CLI';
}

function readLastClaudeCliWorkingDir(): string {
  try {
    return localStorage.getItem(CLAUDE_CLI_LAST_WORKING_DIR_KEY)?.trim() || '';
  } catch {
    return '';
  }
}

function persistLastClaudeCliWorkingDir(value: string): void {
  const trimmed = value.trim();
  if (!trimmed) return;
  try {
    localStorage.setItem(CLAUDE_CLI_LAST_WORKING_DIR_KEY, trimmed);
  } catch {
    // Ignore storage errors. The selected workspace is only a UI convenience.
  }
}

function getClaudeApiProviderTemplateInitialValues(
  preset?: ClaudeApiProviderPreset | null,
): Record<string, string> {
  return Object.fromEntries(
    Object.entries(preset?.templateValues ?? {}).map(([key, config]) => [
      key,
      config.editorValue ?? config.defaultValue ?? '',
    ]),
  );
}

function applyClaudeApiProviderTemplateValue(
  value: string,
  templateValues: Record<string, string>,
): string {
  return value.replace(/\$\{([A-Z0-9_]+)\}/g, (matched, key: string) => templateValues[key] ?? matched);
}

function resolveClaudeApiProviderExtraEnv(
  preset: ClaudeApiProviderPreset | null | undefined,
  templateValues: Record<string, string>,
): Record<string, string> | null {
  const entries = Object.entries(preset?.extraEnv ?? {}).map(([key, value]) => [
    key,
    applyClaudeApiProviderTemplateValue(value, templateValues),
  ]);
  return entries.length > 0 ? Object.fromEntries(entries) : null;
}

function formatDate(timestamp: number): string {
  if (!timestamp) return '-';
  const value = timestamp > 10_000_000_000 ? timestamp : timestamp * 1000;
  return new Date(value).toLocaleString();
}

function readInitialViewMode(): ViewMode {
  try {
    const value = localStorage.getItem(CLAUDE_ACCOUNTS_VIEW_MODE_KEY);
    if (value === 'grid' || value === 'list') return value;
    if (value === 'compact') return 'list';
  } catch {
    // ignore storage failures
  }
  return 'grid';
}

function readClaudeApiKeyUsageCache(): Record<string, ClaudeApiKeyUsageState> {
  try {
    const raw = localStorage.getItem(CLAUDE_API_KEY_USAGE_CACHE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    if (!parsed || typeof parsed !== 'object') return {};
    const next: Record<string, ClaudeApiKeyUsageState> = {};
    Object.entries(parsed).forEach(([accountId, value]) => {
      if (!value || typeof value !== 'object') return;
      const item = value as {
        summary?: ModelProviderUsageSummary;
        error?: string;
        unavailable?: boolean;
        updatedAt?: number;
      };
      next[accountId] = {
        loading: false,
        summary: item.summary,
        error: typeof item.error === 'string' ? item.error : undefined,
        unavailable: item.unavailable === true,
        updatedAt:
          typeof item.updatedAt === 'number' && Number.isFinite(item.updatedAt)
            ? item.updatedAt
            : undefined,
      };
    });
    return next;
  } catch {
    return {};
  }
}

function writeClaudeApiKeyUsageCache(value: Record<string, ClaudeApiKeyUsageState>): void {
  try {
    localStorage.setItem(
      CLAUDE_API_KEY_USAGE_CACHE_KEY,
      JSON.stringify(
        Object.fromEntries(
          Object.entries(value).map(([accountId, item]) => [
            accountId,
            {
              summary: item.summary,
              error: item.error,
              unavailable: item.unavailable === true,
              updatedAt: item.updatedAt,
            },
          ]),
        ),
      ),
    );
  } catch {
    // ignore storage failures
  }
}

function normalizeClaudeApiKeyUsageBaseUrl(value?: string | null): string {
  const raw = value?.trim();
  if (!raw) return '';
  try {
    const url = new URL(raw);
    url.hash = '';
    url.search = '';
    const pathname = url.pathname.replace(/\/+$/g, '');
    return `${url.protocol}//${url.host}${pathname && pathname !== '/' ? pathname : ''}`;
  } catch {
    return raw.replace(/\/+$/g, '').toLowerCase();
  }
}

function hashClaudeApiKeyUsageIdentity(value: string): string {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(36);
}

function getClaudeApiKeyUsageStableCacheKey(account: ClaudeAccount): string | null {
  const apiKey = account.api_key?.trim();
  const baseUrl = normalizeClaudeApiKeyUsageBaseUrl(account.api_base_url);
  if (!apiKey || !baseUrl) return null;
  return `api_key:${hashClaudeApiKeyUsageIdentity(baseUrl)}:${hashClaudeApiKeyUsageIdentity(apiKey)}`;
}

function getClaudeApiKeyUsageRequestKey(account: ClaudeAccount): string {
  return getClaudeApiKeyUsageStableCacheKey(account) ?? account.id;
}

function getClaudeApiKeyUsageCacheKeys(account: ClaudeAccount): string[] {
  const stableKey = getClaudeApiKeyUsageStableCacheKey(account);
  return stableKey && stableKey !== account.id ? [account.id, stableKey] : [account.id];
}

function mergeClaudeApiKeyUsageStates(
  states: Array<ClaudeApiKeyUsageState | undefined>,
): ClaudeApiKeyUsageState | undefined {
  const availableStates = states.filter((state): state is ClaudeApiKeyUsageState => Boolean(state));
  if (availableStates.length === 0) return undefined;

  const newestState = [...availableStates].sort(
    (a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0),
  )[0];
  const newestSummaryState = [...availableStates]
    .filter((state) => Boolean(state.summary))
    .sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0))[0];
  const newestUpdatedAt = Math.max(
    ...availableStates.map((state) => state.updatedAt ?? 0),
  );

  return {
    loading: availableStates.some((state) => state.loading === true),
    summary: newestSummaryState?.summary,
    error: newestState.error,
    unavailable: newestState.unavailable === true,
    updatedAt: newestUpdatedAt > 0 ? newestUpdatedAt : undefined,
  };
}

function getClaudeApiKeyUsageState(
  value: Record<string, ClaudeApiKeyUsageState>,
  account: ClaudeAccount,
): ClaudeApiKeyUsageState | undefined {
  return mergeClaudeApiKeyUsageStates(
    getClaudeApiKeyUsageCacheKeys(account).map((key) => value[key]),
  );
}

function areClaudeApiKeyUsageStatesEqual(
  left?: ClaudeApiKeyUsageState,
  right?: ClaudeApiKeyUsageState,
): boolean {
  return (
    left?.loading === right?.loading &&
    left?.summary === right?.summary &&
    left?.error === right?.error &&
    left?.unavailable === right?.unavailable &&
    left?.updatedAt === right?.updatedAt
  );
}

function setClaudeApiKeyUsageStateForAccount(
  value: Record<string, ClaudeApiKeyUsageState>,
  account: ClaudeAccount,
  state: ClaudeApiKeyUsageState,
): Record<string, ClaudeApiKeyUsageState> {
  let changed = false;
  const next = { ...value };
  getClaudeApiKeyUsageCacheKeys(account).forEach((key) => {
    if (!areClaudeApiKeyUsageStatesEqual(next[key], state)) {
      next[key] = state;
      changed = true;
    }
  });
  return changed ? next : value;
}

function getClaudePlanBadgeLabel(account: ClaudeAccount, t: TFunction): string {
  const plan = getClaudePlanBadge(account);
  if (plan) return plan;
  if (isClaudeDesktopOAuthAccount(account)) {
    return t('claude.desktopOAuth.planUnknown', '订阅未知');
  }
  return t('accounts.plan.personal', 'Personal');
}

interface ClaudeQuotaSummaryItem {
  key: string;
  label: string;
  percentage: number;
  resetTime?: number | null;
}

function clampQuotaPercentage(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, Math.round(value)));
}

function maskClaudeApiKey(value?: string | null): string {
  const raw = value?.trim();
  if (!raw) return '-';
  if (raw.length <= 10) return `${raw.slice(0, 3)}***${raw.slice(-2)}`;
  return `${raw.slice(0, 4)}*****${raw.slice(-4)}`;
}

function parseClaudeDesktopGatewayModels(value: string): string[] {
  const seen = new Set<string>();
  const models: string[] = [];
  value
    .split(/\r?\n|,/)
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

function isClaudeDesktopGatewayRouteModel(value: string): boolean {
  const model = value.trim().toLowerCase();
  return model.startsWith('claude-') || model.startsWith('anthropic/claude-');
}

function getNextClaudeDesktopGatewaySafeModel(usedModels: Iterable<string>): string {
  const used = new Set(
    Array.from(usedModels)
      .map((model) => model.trim().toLowerCase())
      .filter(Boolean),
  );
  const defaultModel = DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS.find(
    (model) => !used.has(model.toLowerCase()),
  );
  if (defaultModel) return defaultModel;

  let index = 2;
  while (used.has(`${CLAUDE_DESKTOP_GATEWAY_GENERATED_ROUTE_BASE}-r${index}`)) {
    index += 1;
  }
  return `${CLAUDE_DESKTOP_GATEWAY_GENERATED_ROUTE_BASE}-r${index}`;
}

function normalizeClaudeDesktopGatewayMode(
  value?: string | null,
): ClaudeDesktopGatewayConnectionMode {
  return value === 'local_mapping' ? 'local_mapping' : 'direct';
}

function buildClaudeDesktopGatewayMappings(
  desktopModels: string[],
  upstreamModels: string[],
): ClaudeDesktopGatewayModelMapping[] {
  const normalizedDesktopModels = desktopModels.map((model) => model.trim()).filter(Boolean);
  const normalizedUpstreamModels = upstreamModels.map((model) => model.trim()).filter(Boolean);
  if (normalizedUpstreamModels.length > 0) {
    const usedDesktopModels = new Set<string>();
    return normalizedUpstreamModels.map((upstreamModel, index) => {
      const preferredModel = normalizedDesktopModels[index] ?? '';
      const preferredKey = preferredModel.toLowerCase();
      const desktopModel = preferredModel
        && isClaudeDesktopGatewayRouteModel(preferredModel)
        && !usedDesktopModels.has(preferredKey)
        ? preferredModel
        : getNextClaudeDesktopGatewaySafeModel(usedDesktopModels);
      usedDesktopModels.add(desktopModel.toLowerCase());
      return {
        desktopModel,
        upstreamModel,
        labelOverride: upstreamModel,
      };
    });
  }
  return normalizedDesktopModels
    .filter(Boolean)
    .map((desktopModel, index) => ({
      desktopModel,
      upstreamModel: upstreamModels[index]?.trim() || '',
    }));
}

function normalizeClaudeDesktopGatewayMappings(
  mappings: ClaudeDesktopGatewayModelMapping[],
): ClaudeDesktopGatewayModelMapping[] {
  const seen = new Set<string>();
  const result: ClaudeDesktopGatewayModelMapping[] = [];
  mappings.forEach((mapping) => {
    const desktopModel = mapping.desktopModel.trim();
    const upstreamModel = mapping.upstreamModel.trim();
    if (!desktopModel || !upstreamModel) return;
    const key = desktopModel.toLowerCase();
    if (seen.has(key)) return;
    seen.add(key);
    result.push({
      desktopModel,
      upstreamModel,
      labelOverride: upstreamModel,
      ...(mapping.supports1m === true ? { supports1m: true } : {}),
    });
  });
  return result;
}

function cloneClaudeDesktopGatewayMappings(
  mappings?: readonly ClaudeDesktopGatewayModelMapping[] | null,
): ClaudeDesktopGatewayModelMapping[] {
  return (mappings ?? []).map((mapping) => ({
    desktopModel: mapping.desktopModel,
    upstreamModel: mapping.upstreamModel,
    labelOverride: mapping.labelOverride ?? null,
    supports1m: mapping.supports1m === true,
  }));
}

function getClaudeDesktopGatewayUpstreamModels(
  mappings: ClaudeDesktopGatewayModelMapping[],
): string[] {
  const seen = new Set<string>();
  const models: string[] = [];
  mappings.forEach((mapping) => {
    const upstreamModel = mapping.upstreamModel.trim();
    const key = upstreamModel.toLowerCase();
    if (!upstreamModel || seen.has(key)) return;
    seen.add(key);
    models.push(upstreamModel);
  });
  return models;
}

function getClaudeDesktopGatewayDuplicateModel(
  mappings: ClaudeDesktopGatewayModelMapping[],
): string | null {
  const seen = new Set<string>();
  for (const mapping of mappings) {
    const desktopModel = mapping.desktopModel.trim();
    if (!desktopModel) continue;
    const key = desktopModel.toLowerCase();
    if (seen.has(key)) return desktopModel;
    seen.add(key);
  }
  return null;
}

function buildClaudeDesktopGatewayDesktopModelOptions(
  customLabel: string,
) {
  const seen = new Set<string>();
  const options = DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS
    .map((model) => model.trim())
    .filter(Boolean)
    .filter((model) => {
      const key = model.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .map((model) => ({ value: model, label: model }));
  return [
    ...options,
    { value: CLAUDE_DESKTOP_GATEWAY_CUSTOM_DESKTOP_MODEL, label: customLabel },
  ];
}

function isClaudeProviderApiKeyAccount(account: ClaudeAccount): boolean {
  return normalizeClaudeAuthMode(account.auth_mode) === 'api_key' || isClaudeDesktopGatewayAccount(account);
}

function isClaudeApiKeyFunAccount(account: ClaudeAccount): boolean {
  const providerId = account.api_provider_id?.trim().toLowerCase();
  const sourceTag = account.api_provider_source_tag?.trim().toLowerCase();
  const providerName = account.api_provider_name?.trim().toLowerCase();
  const baseUrl = account.api_base_url?.trim().toLowerCase();
  return (
    providerId === CLAUDE_APIKEY_FUN_PROVIDER_ID ||
    sourceTag === 'apikey_fun' ||
    providerName === 'apikey.fun' ||
    Boolean(baseUrl && /(^https?:\/\/)?([^/]+\.)?apikey\.fun(\/|$)/i.test(baseUrl))
  );
}

function buildClaudeQuotaSummaryItems(account: ClaudeAccount, t: TFunction): ClaudeQuotaSummaryItem[] {
  const quota = account.quota;
  if (!quota) return [];
  const items: ClaudeQuotaSummaryItem[] = [
    {
      key: 'five-hour',
      label: t('claude.quota.fiveHour', 'Current session'),
      percentage: quota.five_hour_percentage,
      resetTime: quota.five_hour_reset_time,
    },
    {
      key: 'seven-day',
      label: t('claude.quota.sevenDay', 'Current week (all models)'),
      percentage: quota.seven_day_percentage,
      resetTime: quota.seven_day_reset_time,
    },
  ];
  return items;
}

function isClaudeOAuthAuthorizeInput(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;
  try {
    const url = new URL(trimmed);
    return /(^|\.)claude\.com$/i.test(url.hostname) && url.pathname === '/cai/oauth/authorize';
  } catch {
    return trimmed.includes('claude.com/cai/oauth/authorize') || trimmed.includes('code=true&client_id=');
  }
}

function getClaudeCurrentPlatform(subPlatform: ClaudeSubPlatform): ProviderCurrentPlatform {
  return subPlatform === 'desktop' ? 'claude_desktop_account' : 'claude_code_account';
}

function isClaudeJsonExportableAccount(account: ClaudeAccount): boolean {
  return !isClaudeDesktopOAuthAccount(account);
}

export function ClaudeAccountsPage({ subPlatform = 'desktop' }: ClaudeAccountsPageProps) {
  const { t } = useTranslation();
  const claudePlatformId: PlatformId = 'claude_manager';
  const routeInitialSection: ClaudePageSection = subPlatform === 'cli' ? 'cli' : 'desktop';
  const [activeSection, setActiveSection] = useState<ClaudePageSection>(routeInitialSection);
  const activeSubPlatform: ClaudeSubPlatform = activeSection === 'cli' ? 'cli' : 'desktop';
  const { terminalOptions, selectedTerminal, setSelectedTerminal } =
    useLaunchTerminalOptions(activeSubPlatform === 'cli');
  const store = useClaudeAccountStore();
  const claudeInstanceStore = useClaudeInstanceStore();
  const { platformGroups } = usePlatformLayoutStore();
  const remoteHiddenPlatformIds = useRemoteConfigStore((state) => state.hiddenPlatformIds);
  const remoteHiddenPlatformSet = useMemo(
    () => new Set(remoteHiddenPlatformIds),
    [remoteHiddenPlatformIds],
  );
  const currentPlatformGroup = useMemo(
    () => findGroupByPlatform(platformGroups, claudePlatformId),
    [platformGroups, claudePlatformId],
  );
  const claudePlatformLabel = t('nav.claude', 'Claude');
  const switchablePlatforms = useMemo(() => {
    const source = currentPlatformGroup ? currentPlatformGroup.platformIds : [claudePlatformId];
    const visible = source.filter((platformId) =>
      platformId === claudePlatformId ||
      (isMenuVisiblePlatform(platformId) && !remoteHiddenPlatformSet.has(platformId)),
    );
    return visible.includes(claudePlatformId)
      ? visible
      : [claudePlatformId, ...visible];
  }, [currentPlatformGroup, claudePlatformId, remoteHiddenPlatformSet]);
  const platformSwitchOptions = useMemo(
    () =>
      switchablePlatforms.map((platformId) => ({
        platformId,
        label:
          platformId === claudePlatformId
            ? claudePlatformLabel
            : currentPlatformGroup
              ? resolveGroupChildName(
                  currentPlatformGroup,
                  platformId,
                  getPlatformLabel(platformId, t),
                )
              : getPlatformLabel(platformId, t),
      })),
    [claudePlatformId, claudePlatformLabel, currentPlatformGroup, switchablePlatforms, t],
  );
  const [viewMode, setViewMode] = useState<ViewMode>(readInitialViewMode);
  const [searchQuery, setSearchQuery] = useState('');
  const [privacyModeEnabled, setPrivacyModeEnabled] = useState(isPrivacyModeEnabledByDefault);
  const [message, setMessage] = useState<{ text: string; tone?: 'error' | 'success' } | null>(null);
  const [isFlowNoticeCollapsed, setIsFlowNoticeCollapsed] = useState(() => {
    try {
      return localStorage.getItem(CLAUDE_FLOW_NOTICE_COLLAPSED_KEY) === 'true';
    } catch {
      return false;
    }
  });
  const [showAddModal, setShowAddModal] = useState(false);
  const [addTab, setAddTab] = useState<AddTab>('desktop');
  const [jsonInput, setJsonInput] = useState('');
  const [importing, setImporting] = useState(false);
  const [apiKeyInput, setApiKeyInput] = useState('');
  const [apiKeyNameInput, setApiKeyNameInput] = useState('');
  const [apiKeyInputVisible, setApiKeyInputVisible] = useState(false);
  const [apiKeyImporting, setApiKeyImporting] = useState(false);
  const [apiProviderPresetId, setApiProviderPresetId] = useState(DEFAULT_CLAUDE_API_PROVIDER_ID);
  const [apiBaseUrlInput, setApiBaseUrlInput] = useState(DEFAULT_CLAUDE_API_PROVIDER?.baseUrls[0] ?? '');
  const [apiProviderTemplateValues, setApiProviderTemplateValues] = useState<Record<string, string>>(
    () => getClaudeApiProviderTemplateInitialValues(DEFAULT_CLAUDE_API_PROVIDER),
  );
  const [apiKeyModelCatalogOverride, setApiKeyModelCatalogOverride] = useState<string[] | null>(null);
  const [desktopGatewayAuthScheme, setDesktopGatewayAuthScheme] = useState('bearer');
  const [desktopGatewayModelsInput, setDesktopGatewayModelsInput] = useState('');
  const [desktopGatewayConnectionMode, setDesktopGatewayConnectionMode] =
    useState<ClaudeDesktopGatewayConnectionMode>('direct');
  const [desktopGatewayUpstreamModels, setDesktopGatewayUpstreamModels] = useState<string[]>([]);
  const [desktopGatewayModelMappings, setDesktopGatewayModelMappings] = useState<ClaudeDesktopGatewayModelMapping[]>([]);
  const [desktopGatewayMappingsExpanded, setDesktopGatewayMappingsExpanded] = useState(false);
  const [desktopGatewayModelsLoading, setDesktopGatewayModelsLoading] = useState(false);
  const [desktopGatewayModelsError, setDesktopGatewayModelsError] = useState<string | null>(null);
  const [desktopGatewayModelsMessage, setDesktopGatewayModelsMessage] = useState<string | null>(null);
  const [editingDesktopGatewayAccountId, setEditingDesktopGatewayAccountId] = useState<string | null>(null);
  const desktopGatewayModelsFetchSignatureRef = useRef('');
  const desktopGatewayModelsFetchRequestRef = useRef(0);
  const [desktopLogin, setDesktopLogin] = useState<ClaudeDesktopLoginStartResponse | null>(null);
  const [desktopLoginProgress, setDesktopLoginProgress] = useState<ClaudeDesktopLoginProgressPayload | null>(null);
  const [desktopAccountNameInput, setDesktopAccountNameInput] = useState('');
  const [desktopStarting, setDesktopStarting] = useState(false);
  const [desktopCompleting, setDesktopCompleting] = useState(false);
  const [cliImportingLocal, setCliImportingLocal] = useState(false);
  const [oauthLogin, setOauthLogin] = useState<ClaudeOAuthStartResponse | null>(null);
  const [oauthCallbackInput, setOauthCallbackInput] = useState('');
  const [oauthEmailHint, setOauthEmailHint] = useState('');
  const [oauthStarting, setOauthStarting] = useState(false);
  const [oauthCompleting, setOauthCompleting] = useState(false);
  const [oauthCopied, setOauthCopied] = useState(false);
  const [refreshing, setRefreshing] = useState<string | null>(null);
  const [refreshingAll, setRefreshingAll] = useState(false);
  const [verifyingAccountId, setVerifyingAccountId] = useState<string | null>(null);
  const [apiKeyUsageMap, setApiKeyUsageMap] = useState<Record<string, ClaudeApiKeyUsageState>>(
    () => readClaudeApiKeyUsageCache(),
  );
  const apiKeyUsageInFlightRef = useRef<Set<string>>(claudeApiKeyUsageInFlight);
  const apiKeyUsageManualRefreshAtRef = useRef<Record<string, number>>(claudeApiKeyUsageManualRefreshAt);
  const desktopLoginProgressIdRef = useRef<string | null>(null);
  const desktopLoginProgressUnlistenRef = useRef<UnlistenFn | null>(null);
  const oauthPrepareAttemptedRef = useRef(false);
  const [switching, setSwitching] = useState<string | null>(null);
  const [cliLaunchingAccountId, setCliLaunchingAccountId] = useState<string | null>(null);
  const [cliLaunchModal, setCliLaunchModal] = useState<ClaudeCliLaunchModalState | null>(null);
  const [currentAccountId, setCurrentAccountId] = useState<string | null>(null);
  const [tagAccountId, setTagAccountId] = useState<string | null>(null);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());
  const [editingAccountNoteId, setEditingAccountNoteId] = useState<string | null>(null);
  const [editingAccountNoteValue, setEditingAccountNoteValue] = useState('');
  const [savingAccountNote, setSavingAccountNote] = useState(false);
  const [deleteConfirm, setDeleteConfirm] = useState<DeleteConfirmState | null>(null);
  const [deleting, setDeleting] = useState(false);
  const importFileInputRef = useRef<HTMLInputElement | null>(null);
  const {
    message: addModalError,
    scrollKey: addModalErrorScrollKey,
    set: setAddModalError,
  } = useModalErrorState();
  const {
    message: deleteError,
    scrollKey: deleteErrorScrollKey,
    set: setDeleteError,
  } = useModalErrorState();
  const {
    message: accountNoteError,
    scrollKey: accountNoteErrorScrollKey,
    set: setAccountNoteError,
  } = useModalErrorState();

  const exportModal = useExportJsonModal({
    exportFilePrefix: activeSubPlatform === 'desktop' ? 'claude_desktop_accounts' : 'claude_cli_accounts',
    exportJsonByIds: claudeService.exportClaudeAccounts,
    onError: (error) => {
      setMessage({
        text: t('messages.exportFailed', {
          defaultValue: '导出失败：{{error}}',
          error: String(error).replace(/^Error:\s*/, ''),
        }),
        tone: 'error',
      });
    },
  });

  const getDefaultAddTab = useCallback(
    (platform: ClaudeSubPlatform = activeSubPlatform): AddTab =>
      platform === 'desktop' ? 'desktop' : 'oauth',
    [activeSubPlatform],
  );

  const cleanupDesktopLoginProgressListener = useCallback(() => {
    desktopLoginProgressIdRef.current = null;
    const unlisten = desktopLoginProgressUnlistenRef.current;
    desktopLoginProgressUnlistenRef.current = null;
    if (unlisten) {
      try {
        unlisten();
      } catch {
        // ignore listener cleanup failures
      }
    }
  }, []);

  useEffect(() => cleanupDesktopLoginProgressListener, [cleanupDesktopLoginProgressListener]);

  const resetAddModalState = useCallback((platform: ClaudeSubPlatform = activeSubPlatform) => {
    cleanupDesktopLoginProgressListener();
    setAddTab(getDefaultAddTab(platform));
    setJsonInput('');
    setApiKeyInput('');
    setApiKeyNameInput('');
    setApiKeyInputVisible(false);
    setApiProviderPresetId(DEFAULT_CLAUDE_API_PROVIDER_ID);
    setApiBaseUrlInput(DEFAULT_CLAUDE_API_PROVIDER?.baseUrls[0] ?? '');
    setApiProviderTemplateValues(getClaudeApiProviderTemplateInitialValues(DEFAULT_CLAUDE_API_PROVIDER));
    setApiKeyModelCatalogOverride(null);
    setDesktopGatewayAuthScheme('bearer');
    setDesktopGatewayModelsInput('');
    setDesktopGatewayConnectionMode('direct');
    setDesktopGatewayUpstreamModels([]);
    setDesktopGatewayModelMappings([]);
    setDesktopGatewayMappingsExpanded(false);
    setDesktopGatewayModelsLoading(false);
    setDesktopGatewayModelsError(null);
    setDesktopGatewayModelsMessage(null);
    setEditingDesktopGatewayAccountId(null);
    desktopGatewayModelsFetchSignatureRef.current = '';
    desktopGatewayModelsFetchRequestRef.current += 1;
    setDesktopLogin(null);
    setDesktopLoginProgress(null);
    setDesktopAccountNameInput('');
    setOauthLogin(null);
    setOauthCallbackInput('');
    setOauthEmailHint('');
    setOauthCopied(false);
    oauthPrepareAttemptedRef.current = false;
    setAddModalError(null);
  }, [activeSubPlatform, cleanupDesktopLoginProgressListener, getDefaultAddTab, setAddModalError]);

  const closeAddModal = useCallback(() => {
    if (desktopLogin?.loginId) {
      void claudeService.claudeDesktopLoginCancel(desktopLogin.loginId);
    }
    if (oauthLogin?.loginId) {
      void claudeService.claudeOauthLoginCancel(oauthLogin.loginId);
    }
    resetAddModalState();
    setShowAddModal(false);
  }, [desktopLogin?.loginId, oauthLogin?.loginId, resetAddModalState]);

  useEscClose(showAddModal, closeAddModal);
  useEscClose(Boolean(cliLaunchModal), () => setCliLaunchModal(null));
  useEscClose(Boolean(deleteConfirm), () => setDeleteConfirm(null));

  const refreshCurrentAccountId = useCallback(
    async (platform: ClaudeSubPlatform = activeSubPlatform) => {
      try {
        const accountId = await getProviderCurrentAccountId(getClaudeCurrentPlatform(platform));
        setCurrentAccountId(accountId);
      } catch {
        setCurrentAccountId(null);
      }
    },
    [activeSubPlatform],
  );

  useEffect(() => {
    void store.fetchAccounts();
    void refreshCurrentAccountId();
  }, [refreshCurrentAccountId, store.fetchAccounts]);

  useEffect(() => {
    try {
      localStorage.setItem(CLAUDE_FLOW_NOTICE_COLLAPSED_KEY, String(isFlowNoticeCollapsed));
    } catch {
      // ignore storage failures
    }
  }, [isFlowNoticeCollapsed]);

  useEffect(() => {
    try {
      localStorage.setItem(CLAUDE_ACCOUNTS_VIEW_MODE_KEY, viewMode);
    } catch {
      // ignore storage failures
    }
  }, [viewMode]);

  useEffect(() => {
    setActiveSection(routeInitialSection);
  }, [routeInitialSection]);

  useEffect(() => {
    setSelectedIds(new Set());
  }, [activeSubPlatform]);

  const maskAccountText = useCallback(
    (value?: string | null) => maskSensitiveValue(value, privacyModeEnabled),
    [privacyModeEnabled],
  );

  const togglePrivacyMode = () => {
    setPrivacyModeEnabled((prev) => {
      const next = !prev;
      persistPrivacyModeEnabled(next);
      return next;
    });
  };

  const desktopAccounts = useMemo(
    () => store.accounts.filter(isClaudeDesktopRuntimeAccount),
    [store.accounts],
  );

  const cliAccounts = useMemo(
    () => store.accounts.filter((account) => !isClaudeDesktopRuntimeAccount(account)),
    [store.accounts],
  );

  const currentSubPlatformAccounts = activeSubPlatform === 'desktop' ? desktopAccounts : cliAccounts;

  const exportableAccountIds = useMemo(
    () => new Set(currentSubPlatformAccounts.filter(isClaudeJsonExportableAccount).map((account) => account.id)),
    [currentSubPlatformAccounts],
  );

  useEffect(() => {
    writeClaudeApiKeyUsageCache(apiKeyUsageMap);
  }, [apiKeyUsageMap]);

  const selectedApiProviderPreset = useMemo(
    () => findClaudeApiProviderPresetById(apiProviderPresetId),
    [apiProviderPresetId],
  );
  const selectedDesktopGatewayProviderPreset = useMemo(
    () => findClaudeDesktopGatewayProviderPresetById(apiProviderPresetId),
    [apiProviderPresetId],
  );
  const selectedFormProviderPreset = addTab === 'desktopGateway'
    ? selectedDesktopGatewayProviderPreset
    : selectedApiProviderPreset;
  const resolvedApiBaseUrlInput = useMemo(
    () => applyClaudeApiProviderTemplateValue(apiBaseUrlInput, apiProviderTemplateValues),
    [apiBaseUrlInput, apiProviderTemplateValues],
  );
  const inferredApiKeyField = useMemo(
    () => inferClaudeApiKeyField(selectedApiProviderPreset, resolvedApiBaseUrlInput),
    [selectedApiProviderPreset, resolvedApiBaseUrlInput],
  );
  const resolvedApiProviderExtraEnv = useMemo(
    () => resolveClaudeApiProviderExtraEnv(selectedApiProviderPreset, apiProviderTemplateValues),
    [selectedApiProviderPreset, apiProviderTemplateValues],
  );

  const availableTags = useMemo(() => {
    const tags = new Set<string>();
    currentSubPlatformAccounts.forEach((account) => {
      (account.tags || []).forEach((tag) => {
        const normalized = tag.trim();
        if (normalized) tags.add(normalized);
      });
    });
    return Array.from(tags).sort((a, b) => a.localeCompare(b));
  }, [currentSubPlatformAccounts]);

  const tagAccount = useMemo(
    () => store.accounts.find((account) => account.id === tagAccountId) ?? null,
    [store.accounts, tagAccountId],
  );

  const editingAccountNoteAccount = useMemo(
    () => store.accounts.find((account) => account.id === editingAccountNoteId) ?? null,
    [store.accounts, editingAccountNoteId],
  );

  const openAccountNoteModal = useCallback(
    (account: ClaudeAccount) => {
      setEditingAccountNoteId(account.id);
      setEditingAccountNoteValue(account.account_note || '');
      setAccountNoteError(null);
    },
    [setAccountNoteError],
  );

  const closeAccountNoteModal = useCallback(() => {
    if (savingAccountNote) return;
    setEditingAccountNoteId(null);
    setEditingAccountNoteValue('');
    setAccountNoteError(null);
  }, [savingAccountNote, setAccountNoteError]);

  useEscClose(Boolean(editingAccountNoteAccount), closeAccountNoteModal);

  const filteredAccounts = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    return [...currentSubPlatformAccounts]
      .filter((account) => {
        if (!query) return true;
        return [
          getClaudeAccountDisplayEmail(account),
          getClaudeApiProviderLabel(account),
          account.api_base_url ?? '',
          account.account_uuid ?? '',
          account.id,
          account.account_note ?? '',
          ...(account.tags || []),
        ].some((value) => value.toLowerCase().includes(query));
      })
      .sort((a, b) => {
        const currentFirstDiff = compareCurrentAccountFirst(
          a.id,
          b.id,
          currentAccountId,
        );
        if (currentFirstDiff !== 0) return currentFirstDiff;
        return (b.last_used || b.created_at) - (a.last_used || a.created_at);
      });
  }, [searchQuery, currentSubPlatformAccounts, currentAccountId]);

  const filteredIds = useMemo(
    () => filteredAccounts.map((account) => account.id),
    [filteredAccounts],
  );

  const selectedVisibleIds = useMemo(
    () => filteredIds.filter((id) => selectedIds.has(id)),
    [filteredIds, selectedIds],
  );

  const filteredExportableIds = useMemo(
    () => filteredIds.filter((id) => exportableAccountIds.has(id)),
    [exportableAccountIds, filteredIds],
  );

  const selectedExportableIds = useMemo(
    () => selectedVisibleIds.filter((id) => exportableAccountIds.has(id)),
    [exportableAccountIds, selectedVisibleIds],
  );

  const selectedDeletableIds = useMemo(
    () => selectedVisibleIds.filter((id) => id !== currentAccountId),
    [selectedVisibleIds, currentAccountId],
  );

  const isAllFilteredSelected = useMemo(
    () => filteredIds.length > 0 && filteredIds.every((id) => selectedIds.has(id)),
    [filteredIds, selectedIds],
  );

  useEffect(() => {
    const existingIds = new Set(currentSubPlatformAccounts.map((account) => account.id));
    setSelectedIds((prev) => {
      let changed = false;
      const next = new Set<string>();
      prev.forEach((id) => {
        if (existingIds.has(id)) {
          next.add(id);
        } else {
          changed = true;
        }
      });
      return changed ? next : prev;
    });
  }, [currentSubPlatformAccounts]);

  useEffect(() => {
    const apiKeyAccounts = currentSubPlatformAccounts.filter(
      (account) => isClaudeProviderApiKeyAccount(account),
    );
    if (apiKeyAccounts.length === 0) return;
    setApiKeyUsageMap((previous) => {
      let changed = false;
      let next: Record<string, ClaudeApiKeyUsageState> = previous;
      apiKeyAccounts.forEach((account) => {
        const mergedState = getClaudeApiKeyUsageState(previous, account);
        if (!mergedState) return;
        const synced = setClaudeApiKeyUsageStateForAccount(next, account, mergedState);
        if (synced !== next) {
          next = synced;
          changed = true;
        }
      });
      return changed ? next : previous;
    });
  }, [currentSubPlatformAccounts]);

  const accountsForInstances = store.accounts;

  const toggleAccountSelection = useCallback((accountId: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(accountId)) {
        next.delete(accountId);
      } else {
        next.add(accountId);
      }
      return next;
    });
  }, []);

  const toggleSelectAllFiltered = useCallback(() => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (filteredIds.length > 0 && filteredIds.every((id) => next.has(id))) {
        filteredIds.forEach((id) => next.delete(id));
      } else {
        filteredIds.forEach((id) => next.add(id));
      }
      return next;
    });
  }, [filteredIds]);

  const clearSelection = useCallback(() => {
    setSelectedIds(new Set());
  }, []);

  const openAddModal = () => {
    resetAddModalState(activeSubPlatform);
    setShowAddModal(true);
  };

  function clearDesktopGatewayModelDiscoveryStatus() {
    setDesktopGatewayModelsLoading(false);
    setDesktopGatewayModelsError(null);
    setDesktopGatewayModelsMessage(null);
    desktopGatewayModelsFetchSignatureRef.current = '';
    desktopGatewayModelsFetchRequestRef.current += 1;
  }

  function applyDesktopGatewayProviderPreset(providerId: string) {
    setApiProviderPresetId(providerId);
    setApiKeyModelCatalogOverride(null);
    setApiProviderTemplateValues({});
    clearDesktopGatewayModelDiscoveryStatus();
    setAddModalError(null);

    if (providerId === CLAUDE_DESKTOP_GATEWAY_PROVIDER_CUSTOM_ID) {
      return;
    }

    const preset = findClaudeDesktopGatewayProviderPresetById(providerId);
    if (!preset) return;
    const mappings = cloneClaudeDesktopGatewayMappings(preset.modelMappings);
    setApiBaseUrlInput(preset.baseUrls[0] ?? '');
    setDesktopGatewayAuthScheme(preset.authScheme);
    setDesktopGatewayConnectionMode(preset.connectionMode);
    setDesktopGatewayUpstreamModels(getClaudeDesktopGatewayUpstreamModels(mappings));
    setDesktopGatewayMappingsExpanded(false);
    if (preset.connectionMode === 'local_mapping') {
      const nextMappings = mappings.length > 0
        ? mappings
        : buildClaudeDesktopGatewayMappings(DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS, []);
      setDesktopGatewayModelMappings(nextMappings);
      setDesktopGatewayModelsInput(
        (nextMappings.length > 0
          ? nextMappings.map((mapping) => mapping.desktopModel)
          : DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS
        ).join('\n'),
      );
    } else {
      const directModels = mappings.map((mapping) => mapping.desktopModel).filter(Boolean);
      setDesktopGatewayModelMappings(mappings);
      setDesktopGatewayModelsInput(
        (directModels.length > 0 ? directModels : DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS).join('\n'),
      );
    }
    setApiKeyNameInput(preset.name);
  }

  const openEditDesktopGatewayModal = (account: ClaudeAccount) => {
    if (!isClaudeDesktopGatewayAccount(account)) return;
    resetAddModalState('desktop');
    const providerName = account.api_provider_name?.trim()
      || account.organization_name?.trim()
      || getClaudeApiProviderLabel(account)
      || account.email?.trim()
      || t('claude.desktopGateway.label', 'Gateway');
    const apiKey = account.api_key?.trim() || '';
    const baseUrl = account.api_base_url?.trim() || '';
    const authScheme = account.desktop_gateway_auth_scheme?.trim() || 'bearer';
    setEditingDesktopGatewayAccountId(account.id);
    setShowAddModal(true);
    setAddTab('desktopGateway');
    setApiProviderPresetId(
      findClaudeDesktopGatewayProviderPresetById(account.api_provider_id)?.id
        ?? CLAUDE_DESKTOP_GATEWAY_PROVIDER_CUSTOM_ID,
    );
    setApiProviderTemplateValues({});
    setApiKeyModelCatalogOverride(null);
    setApiKeyNameInput(providerName);
    setApiBaseUrlInput(baseUrl);
    setApiKeyInput(apiKey);
    setApiKeyInputVisible(false);
    setDesktopGatewayAuthScheme(authScheme);
    const savedMode = normalizeClaudeDesktopGatewayMode(account.desktop_gateway_connection_mode);
    const savedModels = (account.desktop_gateway_models || []).map((model) => model.trim()).filter(Boolean);
    const savedModelsAreClaude = savedModels.length > 0 && savedModels.every(isClaudeDesktopGatewayRouteModel);
    const mode: ClaudeDesktopGatewayConnectionMode = savedMode === 'local_mapping' || !savedModelsAreClaude
      ? 'local_mapping'
      : 'direct';
    const upstreamModels = (
      account.desktop_gateway_upstream_models?.length
        ? account.desktop_gateway_upstream_models
        : savedModelsAreClaude
          ? []
          : savedModels
    ).map((model) => model.trim()).filter(Boolean);
    const mappings = normalizeClaudeDesktopGatewayMappings(
      account.desktop_gateway_model_mappings || buildClaudeDesktopGatewayMappings(
        mode === 'local_mapping'
          ? DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS
          : savedModels.length > 0
            ? savedModels
            : DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS,
        upstreamModels,
      ),
    );
    setDesktopGatewayConnectionMode(mode);
    setDesktopGatewayUpstreamModels(upstreamModels);
    setDesktopGatewayModelMappings(
      mappings.length > 0
        ? mappings
        : mode === 'local_mapping'
          ? buildClaudeDesktopGatewayMappings(DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS, upstreamModels)
          : [],
    );
    setDesktopGatewayMappingsExpanded(true);
    setDesktopGatewayModelsInput(
      mode === 'local_mapping'
        ? DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS.join('\n')
        : savedModels.join('\n'),
    );
    setDesktopGatewayModelsError(null);
    setDesktopGatewayModelsMessage(null);
    setAddModalError(null);
    const normalizedBaseUrl = normalizeClaudeApiProviderBaseUrl(baseUrl);
    desktopGatewayModelsFetchSignatureRef.current = apiKey && normalizedBaseUrl
      ? `${apiKey}\n${normalizedBaseUrl}\n${authScheme}`
      : '';
  };

  const selectAddTab = (tab: AddTab) => {
    if (editingDesktopGatewayAccountId) return;
    setAddModalError(null);
    if (tab !== 'desktop' && desktopLogin?.loginId) {
      void claudeService.claudeDesktopLoginCancel(desktopLogin.loginId);
      setDesktopLogin(null);
    }
    if (tab !== 'oauth' && oauthLogin?.loginId) {
      void claudeService.claudeOauthLoginCancel(oauthLogin.loginId);
      setOauthLogin(null);
      setOauthCallbackInput('');
      setOauthCopied(false);
      oauthPrepareAttemptedRef.current = false;
    }
    if (tab === 'oauth' && addTab !== 'oauth') {
      oauthPrepareAttemptedRef.current = false;
    }
    if (tab === 'desktopGateway' && addTab !== 'desktopGateway') {
      applyDesktopGatewayProviderPreset(DEFAULT_CLAUDE_DESKTOP_GATEWAY_PROVIDER_ID);
    }
    setAddTab(tab);
  };

  const applyApiKeyFunPrefill = useCallback(
    (request: ApiKeyFunPrefillPayload) => {
      const key = request.apiKey.trim();
      if (!key) return;

      if (request.target === 'claude_desktop') {
        const models = (request.modelCatalog ?? []).map((model) => model.trim()).filter(Boolean);
        const claudeModels = models.filter(isClaudeDesktopGatewayRouteModel);
        const baseUrl = request.baseUrl?.trim() || CLAUDE_APIKEY_FUN_BASE_URL;
        const normalizedBaseUrl = normalizeClaudeApiProviderBaseUrl(baseUrl) || baseUrl;
        resetAddModalState('desktop');
        setActiveSection('desktop');
        setShowAddModal(true);
        setAddTab('desktopGateway');
        setApiProviderPresetId(CLAUDE_APIKEY_FUN_PROVIDER_ID);
        setApiBaseUrlInput(baseUrl);
        setApiProviderTemplateValues({});
        setApiKeyModelCatalogOverride(null);
        setApiKeyNameInput(request.apiKeyName?.trim() || request.providerName?.trim() || 'APIKEY.FUN');
        setApiKeyInput(key);
        setApiKeyInputVisible(false);
        setDesktopGatewayAuthScheme('bearer');
        setDesktopGatewayUpstreamModels(models);
        if (claudeModels.length > 0) {
          setDesktopGatewayConnectionMode('direct');
          setDesktopGatewayModelsInput(claudeModels.join('\n'));
          setDesktopGatewayModelMappings([]);
          setDesktopGatewayMappingsExpanded(false);
        } else {
          setDesktopGatewayConnectionMode('local_mapping');
          setDesktopGatewayModelsInput(DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS.join('\n'));
          setDesktopGatewayModelMappings(buildClaudeDesktopGatewayMappings(
            DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS,
            models,
          ));
          setDesktopGatewayMappingsExpanded(false);
        }
        setDesktopGatewayModelsError(null);
        setDesktopGatewayModelsMessage(t(
          'apiKeyFun.prefill.claudeDesktopReady',
          '已带入 APIKEY.FUN 配置，请确认后添加到 Claude。',
        ));
        desktopGatewayModelsFetchSignatureRef.current = normalizedBaseUrl
          ? `${key}\n${normalizedBaseUrl}\nbearer`
          : '';
        setAddModalError(null);
        return;
      }

      if (request.target === 'claude_cli') {
        resetAddModalState('cli');
        setActiveSection('cli');
        setShowAddModal(true);
        setAddTab('apikey');
        setApiProviderPresetId(CLAUDE_APIKEY_FUN_PROVIDER_ID);
        setApiBaseUrlInput(request.baseUrl?.trim() || CLAUDE_APIKEY_FUN_BASE_URL);
        setApiProviderTemplateValues({});
        setApiKeyModelCatalogOverride(request.modelCatalog ?? null);
        setApiKeyNameInput(request.apiKeyName?.trim() || request.providerName?.trim() || 'APIKEY.FUN');
        setApiKeyInput(key);
        setApiKeyInputVisible(false);
        setAddModalError(null);
      }
    },
    [resetAddModalState, setAddModalError, t],
  );

  useEffect(() => {
    const consumePrefill = () => {
      const request =
        consumeApiKeyFunPrefill('claude_desktop') ||
        consumeApiKeyFunPrefill('claude_cli');
      if (request) {
        applyApiKeyFunPrefill(request);
      }
    };
    consumePrefill();
    window.addEventListener(APIKEY_FUN_PREFILL_EVENT, consumePrefill);
    return () => {
      window.removeEventListener(APIKEY_FUN_PREFILL_EVENT, consumePrefill);
    };
  }, [applyApiKeyFunPrefill]);

  const importJsonContent = async (content: string) => {
    const trimmed = content.trim();
    if (!trimmed) {
      setAddModalError(t('common.shared.token.empty', '请输入 Token 或 JSON'));
      return;
    }
    setImporting(true);
    setAddModalError(null);
    try {
      const accounts = await claudeService.importClaudeFromJson(trimmed);
      await store.fetchAccounts();
      setMessage({
        text: t('common.shared.token.importSuccessMsg', {
          count: accounts.length,
          defaultValue: '成功导入 {{count}} 个账号',
        }),
      });
      closeAddModal();
    } catch (error) {
      setAddModalError(String(error).replace(/^Error:\s*/, ''));
    } finally {
      setImporting(false);
    }
  };

  const handleImportCliFromLocal = async () => {
    setCliImportingLocal(true);
    setAddModalError(null);
    try {
      const account = await claudeService.importClaudeCliFromLocal();
      await store.fetchAccounts();
      setMessage({
        text: t('claude.cli.localSuccess', '已导入本机 Claude Code 登录态：{{name}}', {
          name: account.email,
        }),
      });
      closeAddModal();
    } catch (error) {
      setAddModalError(String(error).replace(/^Error:\s*/, ''));
    } finally {
      setCliImportingLocal(false);
    }
  };

  const handleStartDesktopLogin = async () => {
    cleanupDesktopLoginProgressListener();
    const progressId = createClaudeDesktopLoginProgressId();
    desktopLoginProgressIdRef.current = progressId;
    setDesktopStarting(true);
    setAddModalError(null);
    setDesktopLoginProgress({
      progressId,
      phase: 'start',
      percent: 0,
    });
    try {
      desktopLoginProgressUnlistenRef.current = await listen<ClaudeDesktopLoginProgressPayload>(
        CLAUDE_DESKTOP_LOGIN_PROGRESS_EVENT,
        (event) => {
          const payload = event.payload;
          if (!payload || payload.progressId !== desktopLoginProgressIdRef.current) return;
          setDesktopLoginProgress((previous) => ({
            ...payload,
            percent: payload.percent ?? previous?.percent ?? null,
            downloadedBytes: payload.downloadedBytes ?? previous?.downloadedBytes ?? null,
            totalBytes: payload.totalBytes ?? previous?.totalBytes ?? null,
          }));
        },
      );
      const login = await claudeService.claudeDesktopLoginStart(progressId);
      setDesktopLogin(login);
      setDesktopLoginProgress((previous) => ({
        progressId,
        phase: 'ready',
        percent: 100,
        downloadedBytes: previous?.downloadedBytes ?? null,
        totalBytes: previous?.totalBytes ?? null,
      }));
    } catch (error) {
      setAddModalError(String(error).replace(/^Error:\s*/, ''));
    } finally {
      cleanupDesktopLoginProgressListener();
      setDesktopStarting(false);
    }
  };

  const handleCompleteDesktopLogin = async () => {
    if (!desktopLogin) return;
    setDesktopCompleting(true);
    setAddModalError(null);
    try {
      const account = await claudeService.claudeDesktopLoginComplete(
        desktopLogin.loginId,
        desktopAccountNameInput,
      );
      await store.fetchAccounts();
      setMessage({
        text: t('claude.desktopOAuth.importSuccess', 'Claude 登录态已导入：{{name}}', {
          name: account.email,
        }),
      });
      closeAddModal();
    } catch (error) {
      setAddModalError(String(error).replace(/^Error:\s*/, ''));
    } finally {
      setDesktopCompleting(false);
    }
  };

  const prepareOAuthLogin = useCallback(async (): Promise<ClaudeOAuthStartResponse | null> => {
    if (oauthLogin) return oauthLogin;
    if (oauthStarting) return null;
    setOauthStarting(true);
    setAddModalError(null);
    try {
      const login = await claudeService.claudeOauthLoginPrepare();
      setOauthLogin(login);
      setOauthCallbackInput('');
      setOauthCopied(false);
      return login;
    } catch (error) {
      setAddModalError(String(error).replace(/^Error:\s*/, ''));
      return null;
    } finally {
      setOauthStarting(false);
    }
  }, [oauthLogin, oauthStarting, setAddModalError]);

  useEffect(() => {
    if (!showAddModal || addTab !== 'oauth' || oauthLogin || oauthStarting || oauthPrepareAttemptedRef.current) {
      return;
    }
    oauthPrepareAttemptedRef.current = true;
    void prepareOAuthLogin();
  }, [addTab, oauthLogin, oauthStarting, prepareOAuthLogin, showAddModal]);

  const handleOpenOAuthUrl = async () => {
    const login = oauthLogin ?? await prepareOAuthLogin();
    if (!login?.verificationUri) return;
    try {
      await openUrl(login.verificationUri);
    } catch (error) {
      setAddModalError(
        t('claude.oauth.openFailed', '打开授权链接失败：{{error}}', {
          error: String(error).replace(/^Error:\s*/, ''),
        }),
      );
    }
  };

  const handleCopyOAuthUrl = async () => {
    if (!oauthLogin?.verificationUri) return;
    await navigator.clipboard.writeText(oauthLogin.verificationUri);
    setOauthCopied(true);
    window.setTimeout(() => setOauthCopied(false), 1200);
  };

  const handleCompleteOAuth = async () => {
    if (!oauthLogin) return;
    const callbackOrCode = oauthCallbackInput.trim();
    if (!callbackOrCode) {
      setAddModalError(t('claude.oauth.callbackRequired', '请粘贴授权完成后的回调链接或 code'));
      return;
    }
    if (isClaudeOAuthAuthorizeInput(callbackOrCode)) {
      setAddModalError(
        t(
          'claude.oauth.authorizeUrlNotCallback',
          '这里粘贴的是上方授权入口链接，不是授权完成后的 code。请先在浏览器完成授权，然后复制最终页面地址或页面显示的 code。',
        ),
      );
      return;
    }
    setOauthCompleting(true);
    setAddModalError(null);
    try {
      const account = await claudeService.claudeOauthLoginComplete(
        oauthLogin.loginId,
        callbackOrCode,
        oauthEmailHint,
      );
      await store.fetchAccounts();
      setMessage({
        text: t('claude.oauth.importSuccess', 'Claude OAuth 授权导入成功：{{name}}', {
          name: account.email,
        }),
      });
      closeAddModal();
    } catch (error) {
      setAddModalError(String(error).replace(/^Error:\s*/, ''));
    } finally {
      setOauthCompleting(false);
    }
  };

  const handleSelectApiProviderPreset = (providerId: string) => {
    if (addTab === 'desktopGateway') {
      applyDesktopGatewayProviderPreset(providerId);
      return;
    }
    setApiProviderPresetId(providerId);
    setApiKeyModelCatalogOverride(null);
    setDesktopGatewayModelsError(null);
    setDesktopGatewayModelsMessage(null);
    if (providerId === CLAUDE_API_PROVIDER_CUSTOM_ID) {
      setApiProviderTemplateValues({});
      return;
    }
    const preset = findClaudeApiProviderPresetById(providerId);
    if (!preset) return;
    const templateValues = getClaudeApiProviderTemplateInitialValues(preset);
    setApiProviderTemplateValues(templateValues);
    setApiBaseUrlInput(
      applyClaudeApiProviderTemplateValue(preset.baseUrls[0] ?? '', templateValues),
    );
    setApiKeyNameInput(preset.name);
    setAddModalError(null);
  };

  const handleFetchDesktopGatewayModels = async () => {
    const apiKey = apiKeyInput.trim();
    if (!apiKey) {
      setDesktopGatewayModelsError(t('claude.apiKey.required', '请输入 API Key'));
      return;
    }
    const normalizedBaseUrl = normalizeClaudeApiProviderBaseUrl(resolvedApiBaseUrlInput);
    if (!normalizedBaseUrl) {
      setDesktopGatewayModelsError(t('claude.desktopGateway.baseUrlRequired', '请输入 Gateway Base URL'));
      return;
    }
    const signature = `${apiKey}\n${normalizedBaseUrl}\n${desktopGatewayAuthScheme}`;
    if (desktopGatewayModelsFetchSignatureRef.current === signature) {
      return;
    }
    desktopGatewayModelsFetchSignatureRef.current = signature;
    const requestId = desktopGatewayModelsFetchRequestRef.current + 1;
    desktopGatewayModelsFetchRequestRef.current = requestId;
    setDesktopGatewayModelsLoading(true);
    setDesktopGatewayModelsError(null);
    setDesktopGatewayModelsMessage(null);
    const fallbackMappings = (() => {
      const presetMappings = cloneClaudeDesktopGatewayMappings(selectedDesktopGatewayProviderPreset?.modelMappings);
      if (presetMappings.length > 0) return presetMappings;
      if (desktopGatewayModelMappings.length > 0) return cloneClaudeDesktopGatewayMappings(desktopGatewayModelMappings);
      return buildClaudeDesktopGatewayMappings(DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS, []);
    })();
    const applyFallbackMappings = () => {
      setDesktopGatewayConnectionMode('local_mapping');
      setDesktopGatewayUpstreamModels(getClaudeDesktopGatewayUpstreamModels(fallbackMappings));
      setDesktopGatewayModelsInput(
        (fallbackMappings.length > 0
          ? fallbackMappings.map((mapping) => mapping.desktopModel)
          : DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS
        ).join('\n'),
      );
      setDesktopGatewayModelMappings(fallbackMappings);
      setDesktopGatewayModelsError(null);
      if (fallbackMappings.some((mapping) => mapping.upstreamModel.trim())) {
        setDesktopGatewayMappingsExpanded(false);
        setDesktopGatewayModelsMessage(t('claude.desktopGateway.usingPresetMappings', '已使用预设映射，可按需修改。'));
      } else {
        setDesktopGatewayModelsMessage(null);
        setDesktopGatewayMappingsExpanded(true);
      }
    };
    try {
      const result = await claudeService.listClaudeDesktopGatewayModels({
        apiKey,
        apiBaseUrl: normalizedBaseUrl,
        authScheme: desktopGatewayAuthScheme,
      });
      if (desktopGatewayModelsFetchRequestRef.current !== requestId) {
        return;
      }
      const models = result.models.map((model) => model.id.trim()).filter(Boolean);
      if (models.length === 0) {
        applyFallbackMappings();
        return;
      }
      const claudeModels = models.filter(isClaudeDesktopGatewayRouteModel);
      if (claudeModels.length > 0) {
        setDesktopGatewayConnectionMode('direct');
        setDesktopGatewayModelsInput(claudeModels.join('\n'));
        setDesktopGatewayUpstreamModels(models);
        setDesktopGatewayModelMappings(buildClaudeDesktopGatewayMappings(claudeModels, []));
        setDesktopGatewayMappingsExpanded(false);
      } else {
        setDesktopGatewayConnectionMode('local_mapping');
        setDesktopGatewayUpstreamModels(models);
        setDesktopGatewayModelsInput(DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS.join('\n'));
        setDesktopGatewayModelMappings(buildClaudeDesktopGatewayMappings(DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS, models));
        setDesktopGatewayMappingsExpanded(false);
      }
      setDesktopGatewayModelsMessage(t('claude.desktopGateway.modelsLoaded', '已获取 {{count}} 个模型，可按需修改。', {
        count: models.length,
      }));
    } catch {
      if (desktopGatewayModelsFetchRequestRef.current !== requestId) {
        return;
      }
      applyFallbackMappings();
    } finally {
      if (desktopGatewayModelsFetchRequestRef.current === requestId) {
        setDesktopGatewayModelsLoading(false);
      }
    }
  };

  useEffect(() => {
    if (!showAddModal || addTab !== 'desktopGateway') {
      return;
    }
    const apiKey = apiKeyInput.trim();
    const normalizedBaseUrl = normalizeClaudeApiProviderBaseUrl(resolvedApiBaseUrlInput);
    if (!apiKey || !normalizedBaseUrl) {
      setDesktopGatewayModelsLoading(false);
      desktopGatewayModelsFetchRequestRef.current += 1;
      return;
    }
    const signature = `${apiKey}\n${normalizedBaseUrl}\n${desktopGatewayAuthScheme}`;
    if (desktopGatewayModelsFetchSignatureRef.current === signature) {
      return;
    }
    desktopGatewayModelsFetchRequestRef.current += 1;
    const timer = window.setTimeout(() => {
      void handleFetchDesktopGatewayModels();
    }, 600);
    return () => window.clearTimeout(timer);
  }, [
    addTab,
    apiKeyInput,
    desktopGatewayAuthScheme,
    resolvedApiBaseUrlInput,
    showAddModal,
  ]);

  const handleImportApiKey = async () => {
    const apiKey = apiKeyInput.trim();
    if (!apiKey) {
      setAddModalError(t('claude.apiKey.required', '请输入 API Key'));
      return;
    }
    const missingTemplateValue = addTab === 'desktopGateway'
      ? undefined
      : Object.entries(selectedApiProviderPreset?.templateValues ?? {}).find(
        ([key]) => !(apiProviderTemplateValues[key] ?? '').trim(),
      );
    if (missingTemplateValue) {
      setAddModalError(t('claude.apiKey.templateRequired', '请填写 {{label}}', {
        label: missingTemplateValue[1].label,
      }));
      return;
    }
    const normalizedBaseUrl = normalizeClaudeApiProviderBaseUrl(resolvedApiBaseUrlInput);
    if (normalizedBaseUrl === null) {
      setAddModalError(t('claude.apiKey.baseUrlInvalid', 'Base URL 不是有效 URL'));
      return;
    }
    if (addTab === 'desktopGateway' && !normalizedBaseUrl) {
      setAddModalError(t('claude.desktopGateway.baseUrlRequired', '请输入 Gateway Base URL'));
      return;
    }
    const directDesktopGatewayModels = parseClaudeDesktopGatewayModels(desktopGatewayModelsInput);
    const desktopGatewayMappingRows = desktopGatewayModelMappings
      .map((mapping) => ({
        desktopModel: mapping.desktopModel.trim(),
        upstreamModel: mapping.upstreamModel.trim(),
        supports1m: mapping.supports1m === true,
      }))
      .filter((mapping) => mapping.desktopModel || mapping.upstreamModel);
    const desktopGatewayMappings = normalizeClaudeDesktopGatewayMappings(desktopGatewayMappingRows);
    const desktopGatewayModels = addTab === 'desktopGateway'
      ? desktopGatewayConnectionMode === 'local_mapping'
        ? desktopGatewayMappings.map((mapping) => mapping.desktopModel)
        : directDesktopGatewayModels
      : [];
    const directDesktopGatewayModelSet = new Set(
      directDesktopGatewayModels.map((model) => model.toLowerCase()),
    );
    const directDesktopGatewayModelMappings = normalizeClaudeDesktopGatewayMappings(
      desktopGatewayModelMappings
        .filter((mapping) => directDesktopGatewayModelSet.has(mapping.desktopModel.trim().toLowerCase()))
        .map((mapping) => ({
          desktopModel: mapping.desktopModel,
          upstreamModel: mapping.upstreamModel || mapping.desktopModel,
          supports1m: mapping.supports1m === true,
        })),
    );
    if (addTab === 'desktopGateway') {
      if (desktopGatewayConnectionMode === 'local_mapping') {
        if (desktopGatewayMappings.length === 0) {
          setAddModalError(t('claude.desktopGateway.mappingsRequired', '请配置模型映射'));
          return;
        }
        if (desktopGatewayMappingRows.some((mapping) => !mapping.desktopModel || !mapping.upstreamModel)) {
          setAddModalError(t('claude.desktopGateway.mappingsIncomplete', '请补全模型映射'));
          return;
        }
        if (desktopGatewayMappings.some((mapping) => !isClaudeDesktopGatewayRouteModel(mapping.desktopModel))) {
          setAddModalError(t('claude.desktopGateway.mappingDesktopModelInvalid', '映射左侧必须填写 Claude 可识别的 Claude 模型名'));
          return;
        }
        const duplicatedModel = getClaudeDesktopGatewayDuplicateModel(desktopGatewayMappingRows);
        if (duplicatedModel) {
          setAddModalError(t('claude.desktopGateway.mappingDesktopModelDuplicated', 'Claude 模型名不能重复：{{model}}', {
            model: duplicatedModel,
          }));
          return;
        }
      } else {
        if (desktopGatewayModels.length === 0) {
          setAddModalError(t('claude.desktopGateway.modelsRequired', '请填写模型目录'));
          return;
        }
        if (desktopGatewayModels.some((model) => !isClaudeDesktopGatewayRouteModel(model))) {
          setAddModalError(t('claude.desktopGateway.directModelsInvalid', '直连模式只支持 Claude 可识别的 Claude 模型名'));
          return;
        }
      }
    }
    setApiKeyImporting(true);
    setAddModalError(null);
    try {
      const providerPresetForPayload = addTab === 'desktopGateway'
        ? selectedDesktopGatewayProviderPreset
        : selectedApiProviderPreset;
      const providerPayload = {
        apiBaseUrl: normalizedBaseUrl,
        apiProviderId: providerPresetForPayload?.id ?? null,
        apiProviderName: providerPresetForPayload?.name || apiKeyNameInput || null,
        apiProviderSourceTag: providerPresetForPayload?.sourceTag ?? null,
        apiProviderWebsite: providerPresetForPayload?.website ?? null,
        apiProviderApiKeyUrl: providerPresetForPayload?.apiKeyUrl ?? null,
        apiKeyField: addTab === 'desktopGateway'
          ? inferClaudeDesktopGatewayApiKeyField(selectedDesktopGatewayProviderPreset, desktopGatewayAuthScheme)
          : null,
        apiModelCatalog: addTab === 'desktopGateway'
          ? null
          : apiKeyModelCatalogOverride ?? selectedApiProviderPreset?.modelCatalog ?? null,
        apiExtraEnv: addTab === 'desktopGateway' ? null : resolvedApiProviderExtraEnv,
      };
      const gatewayPayload = {
        ...providerPayload,
        authScheme: desktopGatewayAuthScheme,
        desktopGatewayModels,
        desktopGatewayConnectionMode,
        desktopGatewayUpstreamModels,
        desktopGatewayModelMappings: desktopGatewayConnectionMode === 'local_mapping'
          ? desktopGatewayMappings
          : directDesktopGatewayModelMappings.length > 0
            ? directDesktopGatewayModelMappings
            : null,
      };
      const account = addTab === 'desktopGateway'
        ? editingDesktopGatewayAccountId
          ? await claudeService.updateClaudeDesktopGateway(
            editingDesktopGatewayAccountId,
            apiKey,
            apiKeyNameInput,
            gatewayPayload,
          )
          : await claudeService.importClaudeDesktopGateway(apiKey, apiKeyNameInput, gatewayPayload)
        : await claudeService.importClaudeApiKey(apiKey, apiKeyNameInput, {
          ...providerPayload,
          apiKeyField: inferredApiKeyField,
        });
      await store.fetchAccounts();
      setMessage({
        text: t(
          addTab === 'desktopGateway'
            ? editingDesktopGatewayAccountId
              ? 'claude.desktopGateway.updateSuccess'
              : 'claude.desktopGateway.importSuccess'
            : 'claude.apiKey.importSuccess',
          addTab === 'desktopGateway'
            ? editingDesktopGatewayAccountId
              ? 'Claude Gateway 账号已更新：{{name}}'
              : 'Claude Gateway 账号已导入：{{name}}'
            : 'Claude API Key 账号已导入：{{name}}',
          {
          name: account.email,
          },
        ),
      });
      closeAddModal();
    } catch (error) {
      setAddModalError(String(error).replace(/^Error:\s*/, ''));
    } finally {
      setApiKeyImporting(false);
    }
  };

  const handleImportFile = async (file: File) => {
    const text = await file.text();
    await importJsonContent(text);
  };

  const handleSwitch = async (account: ClaudeAccount) => {
    setSwitching(account.id);
    setMessage(null);
    try {
      await store.switchAccount(account.id);
      setCurrentAccountId(account.id);
      const displayName = maskAccountText(getClaudeAccountDisplayEmail(account));
      setMessage({
        text: isClaudeDesktopGatewayAccount(account)
          ? t(
            'claude.desktopGateway.switchSuccess',
            '已应用 Claude Desktop 供应商配置：{{name}}。',
            { name: displayName },
          )
          : t('messages.switched', {
            email: displayName,
          }),
      });
    } catch (error) {
      if (String(error ?? '').startsWith('APP_PATH_NOT_FOUND:claude')) {
        window.dispatchEvent(
          new CustomEvent('app-path-missing', {
            detail: {
              app: 'claude',
              retry: { kind: 'switchAccount', accountId: account.id },
            },
          }),
        );
        return;
      }
      setMessage({
        text: t('messages.switchFailed', {
          error: String(error),
        }),
        tone: 'error',
      });
    } finally {
      setSwitching(null);
    }
  };

  const resolveClaudeCliInstanceForAccount = async (
    account: ClaudeAccount,
    workingDir: string,
  ): Promise<InstanceProfile> => {
    const normalizedWorkingDir = normalizePathForCompare(workingDir);
    const instances = await claudeInstanceService.listInstances();
    const existing = instances.find(
      (instance) =>
        !instance.isDefault &&
        (instance.launchMode ?? 'app') === 'cli' &&
        instance.bindAccountId === account.id &&
        normalizePathForCompare(instance.workingDir) === normalizedWorkingDir,
    );
    if (existing) {
      return existing;
    }

    const defaults = await claudeInstanceService.getInstanceDefaults();
    const displayName = getClaudeAccountDisplayEmail(account) || account.email || account.id;
    const instanceHash = md5(`${account.id}|${normalizedWorkingDir}`).substring(0, 12);
    const instanceName = sanitizeClaudeCliInstanceName(
      `${displayName} CLI ${instanceHash.substring(0, 6)}`,
    );
    const userDataDir = joinFilePath(defaults.rootDir, `cli-${instanceHash}`);

    return await claudeInstanceService.createInstance({
      name: instanceName,
      userDataDir,
      workingDir: normalizedWorkingDir,
      extraArgs: '',
      bindAccountId: account.id,
      launchMode: 'cli',
      copySourceInstanceId: '__default__',
      initMode: 'copy',
    });
  };

  const handleLaunchClaudeCli = async (account: ClaudeAccount) => {
    setMessage(null);
    setCliLaunchModal({
      accountId: account.id,
      accountEmail: getClaudeAccountDisplayEmail(account),
      instanceId: null,
      workingDir: readLastClaudeCliWorkingDir(),
      instanceName: t('instances.messages.launchPrepared', '启动命令已准备'),
      launchCommand: '',
      preparing: false,
      copied: false,
      executing: false,
      executeMessage: null,
      executeError: null,
    });
  };

  const prepareClaudeCliLaunch = async (
    modal: ClaudeCliLaunchModalState,
  ): Promise<ClaudeCliLaunchModalState | null> => {
    if (modal.instanceId && modal.launchCommand.trim()) {
      return modal;
    }
    const selected = modal.workingDir.trim();
    if (!selected) {
      setCliLaunchModal((prev) =>
        prev && prev.accountId === modal.accountId
          ? {
              ...prev,
              executeMessage: null,
              executeError: t('claude.cli.selectWorkingDir', '选择 Claude CLI 工作目录'),
            }
          : prev,
      );
      return null;
    }

    const account = useClaudeAccountStore
      .getState()
      .accounts.find((item) => item.id === modal.accountId);
    if (!account) {
      setCliLaunchModal((prev) =>
        prev && prev.accountId === modal.accountId
          ? {
              ...prev,
              executeMessage: null,
              executeError: t('instances.messages.accountMissing', '账号不存在'),
            }
          : prev,
      );
      return null;
    }

    setCliLaunchingAccountId(account.id);
    setCliLaunchModal((prev) =>
      prev && prev.accountId === modal.accountId
        ? {
            ...prev,
            preparing: true,
            executing: false,
            executeMessage: null,
            executeError: null,
            copied: false,
          }
        : prev,
    );
    try {
      const instance = await resolveClaudeCliInstanceForAccount(account, selected);
      const prepared = await claudeInstanceService.startInstance(instance.id);
      const launchInfo = await claudeInstanceService.getClaudeInstanceLaunchCommand(prepared.id);
      await claudeInstanceStore.refreshInstances();
      await store.fetchAccounts();
      setCurrentAccountId(account.id);
      persistLastClaudeCliWorkingDir(prepared.workingDir || selected);
      const nextModal: ClaudeCliLaunchModalState = {
        accountId: account.id,
        accountEmail: getClaudeAccountDisplayEmail(account),
        instanceId: prepared.id,
        workingDir: prepared.workingDir || selected,
        instanceName: prepared.isDefault
          ? t('instances.defaultName', '默认实例')
          : prepared.name || t('instances.defaultName', '默认实例'),
        launchCommand: launchInfo.launchCommand,
        preparing: false,
        copied: false,
        executing: false,
        executeMessage: null,
        executeError: null,
      };
      setCliLaunchModal((prev) => (prev && prev.accountId === modal.accountId ? nextModal : prev));
      return nextModal;
    } catch (error) {
      setCliLaunchModal((prev) =>
        prev && prev.accountId === modal.accountId
          ? {
              ...prev,
              preparing: false,
              executing: false,
              executeMessage: null,
              executeError: String(error).replace(/^Error:\s*/, ''),
            }
          : prev,
      );
      return null;
    } finally {
      setCliLaunchingAccountId(null);
    }
  };

  const updateCliLaunchWorkingDir = (value: string) => {
    setCliLaunchModal((prev) =>
      prev
        ? {
            ...prev,
            workingDir: value,
            instanceId: null,
            instanceName: t('instances.messages.launchPrepared', '启动命令已准备'),
            launchCommand: '',
            copied: false,
            executeMessage: null,
            executeError: null,
          }
        : prev,
    );
  };

  const handleChooseCliWorkingDir = async () => {
    if (!cliLaunchModal || cliLaunchModal.preparing || cliLaunchModal.executing) return;
    const selected = await openFileDialog({
      directory: true,
      multiple: false,
      title: t('claude.cli.selectWorkingDir', '选择 Claude CLI 工作目录'),
    });
    if (!selected || typeof selected !== 'string') return;
    persistLastClaudeCliWorkingDir(selected);
    updateCliLaunchWorkingDir(selected);
  };

  const handleCopyCliLaunchCommand = async () => {
    if (!cliLaunchModal) return;
    const prepared = await prepareClaudeCliLaunch(cliLaunchModal);
    if (!prepared) return;
    try {
      await navigator.clipboard.writeText(prepared.launchCommand);
      setCliLaunchModal((prev) => (prev ? { ...prev, copied: true, executeError: null } : prev));
      window.setTimeout(() => {
        setCliLaunchModal((prev) => (prev ? { ...prev, copied: false } : prev));
      }, 1200);
    } catch {
      setCliLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executeError: t('common.shared.export.copyFailed', '复制失败，请手动复制'),
            }
          : prev,
      );
    }
  };

  const handleExecuteCliInTerminal = async () => {
    if (!cliLaunchModal || cliLaunchModal.executing) return;
    const prepared = await prepareClaudeCliLaunch(cliLaunchModal);
    if (!prepared?.instanceId) return;
    setCliLaunchModal((prev) =>
      prev
        ? {
            ...prev,
            executing: true,
            executeMessage: null,
            executeError: null,
          }
        : prev,
    );
    try {
      const result = await claudeInstanceService.executeClaudeInstanceLaunchCommand(
        prepared.instanceId,
        selectedTerminal,
      );
      await store.fetchAccounts();
      await claudeInstanceStore.refreshInstances();
      setCurrentAccountId(prepared.accountId);
      setCliLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executing: false,
              executeMessage: result || t('claude.cli.launchSuccess', '已启动 Claude CLI'),
            }
          : prev,
      );
    } catch (error) {
      setCliLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executing: false,
              executeError: String(error).replace(/^Error:\s*/, ''),
            }
          : prev,
      );
    }
  };

  const refreshClaudeApiKeyUsage = useCallback(
    async (
      account: ClaudeAccount,
      options?: { showMessage?: boolean },
    ) => {
      const apiKey = account.api_key?.trim() || '';
      const baseUrl = account.api_base_url?.trim() || '';
      const showMessage = options?.showMessage === true;
      if (!apiKey || !baseUrl) {
        if (showMessage) {
          setMessage({
            text: t('codex.modelProviders.usage.noKey', '暂无可查询额度'),
            tone: 'error',
          });
        }
        return;
      }
      const requestKey = getClaudeApiKeyUsageRequestKey(account);
      const now = Date.now();
      const lastRequestedAt = apiKeyUsageManualRefreshAtRef.current[requestKey] ?? 0;
      if (now - lastRequestedAt < CLAUDE_API_KEY_USAGE_REFRESH_THROTTLE_MS) return;
      if (apiKeyUsageInFlightRef.current.has(requestKey)) return;

      apiKeyUsageManualRefreshAtRef.current[requestKey] = now;
      apiKeyUsageInFlightRef.current.add(requestKey);
      setApiKeyUsageMap((previous) =>
        setClaudeApiKeyUsageStateForAccount(previous, account, {
          ...(getClaudeApiKeyUsageState(previous, account) ?? {}),
          loading: true,
          error: undefined,
          unavailable: false,
        }),
      );

      try {
        const summary = await queryModelProviderUsage({
          baseUrl,
          apiKey,
          integrationType: null,
        });
        setApiKeyUsageMap((previous) =>
          setClaudeApiKeyUsageStateForAccount(previous, account, {
            loading: false,
            summary,
            updatedAt: Date.now(),
          }),
        );
        if (showMessage) {
          setMessage({ text: t('claude.quota.refreshSuccess', '额度已刷新') });
        }
      } catch (error) {
        const unavailable = isModelProviderUsageUnavailableError(error);
        const errorText = String(error).replace(/^Error:\s*/, '');
        setApiKeyUsageMap((previous) =>
          setClaudeApiKeyUsageStateForAccount(previous, account, {
            loading: false,
            summary: getClaudeApiKeyUsageState(previous, account)?.summary,
            error: unavailable ? undefined : errorText,
            unavailable,
            updatedAt: Date.now(),
          }),
        );
        if (showMessage) {
          setMessage({
            text: t('claude.quota.refreshFailed', '额度刷新失败：{{error}}', {
              error: unavailable
                ? t('codex.modelProviders.usage.noKey', '暂无可查询额度')
                : errorText,
            }),
            tone: 'error',
          });
        }
      } finally {
        apiKeyUsageInFlightRef.current.delete(requestKey);
      }
    },
    [t],
  );

  const handleRefresh = async (accountId: string) => {
    const targetAccount = useClaudeAccountStore
      .getState()
      .accounts.find((account) => account.id === accountId);
    if (targetAccount && isClaudeProviderApiKeyAccount(targetAccount)) {
      setMessage(null);
      await refreshClaudeApiKeyUsage(targetAccount, { showMessage: true });
      return;
    }

    setRefreshing(accountId);
    setMessage(null);
    try {
      await store.refreshToken(accountId);
      const refreshed = useClaudeAccountStore.getState().accounts.find((account) => account.id === accountId);
      if (refreshed?.quota_error?.message) {
        setMessage({
          text: t('claude.quota.refreshWithWarning', '额度刷新失败：{{error}}', {
            error: refreshed.quota_error.message,
          }),
          tone: 'error',
        });
      } else {
        setMessage({
          text: t('claude.quota.refreshSuccess', '额度已刷新'),
        });
      }
    } catch (error) {
      setMessage({
        text: t('claude.quota.refreshFailed', '额度刷新失败：{{error}}', {
          error: String(error).replace(/^Error:\s*/, ''),
        }),
        tone: 'error',
      });
    } finally {
      setRefreshing(null);
    }
  };

  const handleRefreshAll = async () => {
    setRefreshingAll(true);
    setMessage(null);
    try {
      for (const account of currentSubPlatformAccounts) {
        if (isClaudeProviderApiKeyAccount(account)) {
          await refreshClaudeApiKeyUsage(account);
        } else {
          await store.refreshToken(account.id);
        }
      }
      setMessage({
        text: t('claude.quota.refreshAllDone', '额度刷新完成'),
      });
    } catch (error) {
      setMessage({
        text: t('claude.quota.refreshFailed', '额度刷新失败：{{error}}', {
          error: String(error).replace(/^Error:\s*/, ''),
        }),
        tone: 'error',
      });
    } finally {
      setRefreshingAll(false);
    }
  };

  const handleOpenVerificationWindow = async (accountId: string) => {
    setVerifyingAccountId(accountId);
    setMessage(null);
    try {
      await claudeService.openClaudeVerificationWindow(accountId);
      setMessage({
        text: t(
          'claude.quota.verificationWindowOpened',
          '已打开 Claude 验证窗口，完成验证后请重新刷新额度。',
        ),
      });
    } catch (error) {
      setMessage({
        text: t('claude.quota.verificationWindowFailed', '打开 Claude 验证窗口失败：{{error}}', {
          error: String(error).replace(/^Error:\s*/, ''),
        }),
        tone: 'error',
      });
    } finally {
      setVerifyingAccountId(null);
    }
  };

  const handleExport = async (accountIds: string[]) => {
    const exportIds = accountIds.filter((id) => exportableAccountIds.has(id));
    if (exportIds.length === 0) return;
    const fileNameBase = activeSubPlatform === 'desktop' ? 'claude_desktop_accounts' : 'claude_cli_accounts';
    await exportModal.startExport(exportIds, fileNameBase);
  };

  const handleSubmitAccountNote = async () => {
    if (!editingAccountNoteId || savingAccountNote) return;
    setSavingAccountNote(true);
    setAccountNoteError(null);
    try {
      await claudeService.updateClaudeAccountNote(editingAccountNoteId, editingAccountNoteValue);
      await store.fetchAccounts();
      setMessage({
        text: t('claude.accountNote.saved', '账号备注已保存'),
      });
      setEditingAccountNoteId(null);
      setEditingAccountNoteValue('');
    } catch (error) {
      setAccountNoteError(
        t('claude.accountNote.saveFailed', '保存账号备注失败：{{error}}', {
          error: String(error).replace(/^Error:\s*/, ''),
        }),
      );
    } finally {
      setSavingAccountNote(false);
    }
  };

  const handleSaveTags = async (tags: string[]) => {
    if (!tagAccountId) return;
    await store.updateAccountTags(tagAccountId, tags);
    await store.fetchAccounts();
    setTagAccountId(null);
  };

  const confirmDelete = async () => {
    if (!deleteConfirm || deleting) return;
    setDeleting(true);
    setDeleteError(null);
    try {
      await store.deleteAccounts(deleteConfirm.accountIds);
      await refreshCurrentAccountId();
      setSelectedIds((prev) => {
        const next = new Set(prev);
        deleteConfirm.accountIds.forEach((id) => next.delete(id));
        return next;
      });
      setDeleteConfirm(null);
    } catch (error) {
      setDeleteError(String(error).replace(/^Error:\s*/, ''));
    } finally {
      setDeleting(false);
    }
  };

  const openBatchDeleteConfirm = () => {
    if (selectedDeletableIds.length === 0) return;
    setDeleteError(null);
    setDeleteConfirm({
      accountIds: selectedDeletableIds,
      email: t('claude.deleteSelectedLabel', '{{count}} 个账号', {
        count: selectedDeletableIds.length,
      }),
    });
  };

  const renderAccountNoteButton = (account: ClaudeAccount, className = 'codex-account-note-chip') => {
    const hasNote = Boolean(account.account_note?.trim());
    return (
      <button
        type="button"
        className={`${className} ${hasNote ? 'has-note' : 'empty-note'}`}
        onClick={() => openAccountNoteModal(account)}
        title={hasNote ? account.account_note || '' : t('claude.accountNote.emptyTitle', '填写账号备注')}
        aria-label={t('claude.accountNote.title', '账号备注')}
      >
        <FileText size={className.includes('card-action') || className.includes('action-btn') ? 14 : 12} />
        {!className.includes('card-action') && !className.includes('action-btn') && (
          <span>
            {hasNote
              ? t('claude.accountNote.short', '账号备注')
              : t('claude.accountNote.addShort', '加备注')}
          </span>
        )}
      </button>
    );
  };

  const renderAccountActions = (account: ClaudeAccount, variant: 'card' | 'table' = 'table') => {
    const isCurrent = currentAccountId === account.id;
    const authMode = normalizeClaudeAuthMode(account.auth_mode);
    const isApiKey = authMode === 'api_key';
    const isDesktopGateway = isClaudeDesktopGatewayAccount(account);
    const isProviderAccount = isApiKey || isDesktopGateway;
    const isDesktopRuntime = isClaudeDesktopRuntimeAccount(account);
    const isClaudeCodeOAuth = !isApiKey && !isDesktopRuntime;
    const canExportJson = isClaudeJsonExportableAccount(account);
    const isCliSubPlatform = activeSubPlatform === 'cli';
    const isApiKeyUsageLoading =
      isProviderAccount && getClaudeApiKeyUsageState(apiKeyUsageMap, account)?.loading === true;
    const buttonClass = variant === 'card' ? 'card-action-btn' : 'action-btn';
    return (
      <div className={variant === 'card' ? 'card-actions' : 'action-buttons'}>
        <button
          className={buttonClass}
          onClick={() => setTagAccountId(account.id)}
          title={t('accounts.editTags', '编辑标签')}
        >
          <Tag size={14} />
        </button>
        {renderAccountNoteButton(
          account,
          `${buttonClass} ${account.account_note?.trim() ? 'active' : ''}`,
        )}
        {isDesktopGateway && (
          <button
            className={buttonClass}
            onClick={() => openEditDesktopGatewayModal(account)}
            title={t('common.edit', '编辑')}
          >
            <Pencil size={14} />
          </button>
        )}
        <button
          className={`${buttonClass} ${!isCurrent ? 'success' : ''}`}
          onClick={() => void (isCliSubPlatform ? handleLaunchClaudeCli(account) : handleSwitch(account))}
          disabled={
            isCliSubPlatform
              ? Boolean(cliLaunchingAccountId) || isDesktopRuntime
              : Boolean(switching) || isApiKey
          }
          title={
            isCliSubPlatform
              ? isDesktopRuntime
                  ? t('claude.desktopOAuth.cliUnsupported', 'Claude 账号不能启动 Claude Code CLI')
                  : t('claude.cli.quickLaunch', 'CLI 启动')
              : isApiKey
                ? t('claude.apiKey.switchDisabled', 'API Key 账号不能写入本地登录态')
                : isDesktopGateway
                  ? t('claude.desktopGateway.switchHint', '应用供应商配置并启动 Claude Desktop')
                  : isClaudeCodeOAuth
                    ? t('claude.oauth.switchHint', '切换到本机 Claude Code')
                    : isDesktopRuntime
                      ? t('claude.desktopOAuth.switchHint', '切换到官方 Claude')
                      : t('common.shared.switchAccount', '切换账号')
          }
        >
          {(isCliSubPlatform ? cliLaunchingAccountId : switching) === account.id
            ? <RefreshCw size={14} className="loading-spinner" />
            : <Play size={14} />}
        </button>
        <button
          className={buttonClass}
          onClick={() => void handleRefresh(account.id)}
          disabled={refreshing === account.id || isApiKeyUsageLoading}
          title={t('common.refresh', '刷新')}
        >
          <RotateCw
            size={14}
            className={refreshing === account.id || isApiKeyUsageLoading ? 'loading-spinner' : ''}
          />
        </button>
        {canExportJson && (
          <button
            className={buttonClass}
            onClick={() => void handleExport([account.id])}
            title={t('common.shared.export.title', '导出')}
          >
            <Upload size={14} />
          </button>
        )}
        <button
          className={`${buttonClass} danger`}
          onClick={() =>
            setDeleteConfirm({
              accountIds: [account.id],
              email: getClaudeAccountDisplayEmail(account),
            })
          }
          disabled={isCurrent}
          title={isCurrent ? t('claude.deleteCurrentDisabled', '当前账号不可删除') : t('common.delete', '删除')}
        >
          <Trash2 size={14} />
        </button>
      </div>
    );
  };

  const renderPlanControl = (account: ClaudeAccount) => {
    if (isClaudeDesktopGatewayAccount(account)) return null;
    const planBadge = getClaudePlanBadgeLabel(account, t);
    const planClass = getClaudePlanBadgeClass(account);
    return <span className={`tier-badge ${planClass}`}>{planBadge}</span>;
  };

  const getDesktopGatewayProviderTitle = (account: ClaudeAccount) => (
    account.api_provider_name?.trim()
    || account.organization_name?.trim()
    || getClaudeApiProviderLabel(account)
    || t('claude.desktopGateway.label', 'Gateway')
  );

  const handleCopyApiKey = async (account: ClaudeAccount) => {
    const apiKey = account.api_key?.trim();
    if (!apiKey) return;
    try {
      await navigator.clipboard.writeText(apiKey);
      setMessage({ text: t('common.copied', '已复制') });
    } catch {
      setMessage({
        text: t('common.shared.export.copyFailed', '复制失败，请手动复制'),
        tone: 'error',
      });
    }
  };

  const renderApiKeyLine = (account: ClaudeAccount) => {
    const apiKey = account.api_key?.trim() || '';
    const masked = maskClaudeApiKey(apiKey);
    return (
      <div className="account-sub-line claude-api-key-line">
        <span className="codex-login-subline" title={apiKey ? masked : '-'}>
          {t('claude.apiKey.label', 'API Key')}: {masked}
        </span>
        {apiKey && (
          <button
            type="button"
            className="claude-api-key-copy"
            onClick={() => void handleCopyApiKey(account)}
            title={t('common.copy', '复制')}
            aria-label={t('common.copy', '复制')}
          >
            <Copy size={14} />
          </button>
        )}
      </div>
    );
  };

  const renderApiKeyStatsPanel = (account: ClaudeAccount) => {
    const usageState = getClaudeApiKeyUsageState(apiKeyUsageMap, account);
    return (
      <ModelProviderUsagePanel
        summary={usageState?.summary}
        loading={usageState?.loading === true}
        error={usageState?.error}
        unavailable={usageState?.unavailable === true}
        className="claude-api-key-stats-panel"
      />
    );
  };

  const renderQuotaSummary = (account: ClaudeAccount, variant: 'card' | 'table') => {
    const items = buildClaudeQuotaSummaryItems(account, t);
    const errorMessage = account.quota_error?.message?.trim();
    const isDesktopAccount = isClaudeDesktopOAuthAccount(account);
    const isApiKey = normalizeClaudeAuthMode(account.auth_mode) === 'api_key';
    const canOpenVerificationWindow = isDesktopAccount && isClaudeCloudflareError(errorMessage);

    if (isApiKey && !isDesktopAccount && items.length === 0 && !errorMessage) {
      return variant === 'table' ? (
        <span style={{ color: 'var(--text-muted)', fontSize: 13 }}>{t('claude.quota.unsupported', '不可刷新')}</span>
      ) : null;
    }

    const content = (
      <>
        {items.map((item) => {
          const percentage = clampQuotaPercentage(item.percentage);
          const quotaClass = getClaudeQuotaClass(percentage);
          const resetText = formatClaudeResetTime(item.resetTime);
          const resetDisplay = resetText || '-';
          const Icon = item.key === 'five-hour' ? Clock3 : CalendarDays;
          const title = resetText
            ? t('claude.quota.resetAt', '{{label}} 重置：{{time}}', {
                label: item.label,
                time: resetText,
              })
            : item.label;

          if (variant === 'card') {
            return (
              <div className="quota-item" key={`${account.id}-${item.key}`} title={title}>
                <div className="quota-header">
                  <Icon size={14} />
                  <span className="quota-label">{item.label}</span>
                  <span className={`quota-pct ${quotaClass}`}>{percentage}%</span>
                </div>
                <div className="quota-bar-track">
                  <div className={`quota-bar ${quotaClass}`} style={{ width: `${percentage}%` }} />
                </div>
                <span className="quota-reset">{resetDisplay}</span>
              </div>
            );
          }

          return (
            <div className="quota-item" key={`${account.id}-${item.key}`} title={title}>
              <div className="quota-header">
                <span className="quota-name">{item.label}</span>
                <span className={`quota-value ${quotaClass}`}>{percentage}%</span>
              </div>
              <div className="quota-progress-track">
                <div className={`quota-progress-bar ${quotaClass}`} style={{ width: `${percentage}%` }} />
              </div>
              <div className="quota-footer">
                <span className="quota-reset">{resetDisplay}</span>
              </div>
            </div>
          );
        })}
        {items.length === 0 && (
          <div className={variant === 'card' ? 'quota-empty' : ''} style={variant === 'table' ? { color: 'var(--text-muted)', fontSize: 13 } : undefined}>
            {t('claude.quota.empty', '暂无额度')}
          </div>
        )}
        {errorMessage && (
          <div className={`quota-error-inline ${variant === 'table' ? 'table' : ''}`} title={errorMessage}>
            <CircleAlert size={variant === 'table' ? 12 : 14} />
            <span>{errorMessage}</span>
            {canOpenVerificationWindow && (
              <button
                type="button"
                className="btn btn-secondary quota-error-action"
                disabled={verifyingAccountId === account.id}
                onClick={(event) => {
                  event.stopPropagation();
                  void handleOpenVerificationWindow(account.id);
                }}
              >
                <ExternalLink size={12} />
                <span>
                  {verifyingAccountId === account.id
                    ? t('claude.quota.openingVerification', '正在打开...')
                    : t('claude.quota.openVerification', '打开验证窗口')}
                </span>
              </button>
            )}
          </div>
        )}
      </>
    );

    return variant === 'card' ? (
      <div className="codex-quota-section">{content}</div>
    ) : (
      <div className="quota-grid">{content}</div>
    );
  };

  const isDesktopSubPlatform = activeSubPlatform === 'desktop';
  const isInstancesSection = activeSection === 'instances';
  const shouldShowDesktopGatewayRouting =
    addTab === 'desktopGateway' &&
    Boolean(apiKeyInput.trim()) &&
    (
      desktopGatewayModelsLoading ||
      Boolean(desktopGatewayModelsMessage) ||
      Boolean(desktopGatewayModelsError) ||
      Boolean(desktopGatewayModelsInput.trim()) ||
      desktopGatewayModelMappings.length > 0
    );
  const shouldShowDesktopGatewayMappingEditor =
    desktopGatewayMappingsExpanded || desktopGatewayModelMappings.length === 0;
  const desktopGatewayMappingPreview = desktopGatewayModelMappings.slice(0, 3);
  const subPlatformAccountsCount = currentSubPlatformAccounts.length;
  const claudeTopTabs: Array<{
    key: ClaudePageSection;
    label: string;
    icon: ReactNode;
  }> = [
    {
      key: 'desktop',
      label: t('claude.subPlatform.desktop', 'Claude'),
      icon: <ClaudeIcon className="tab-icon" />,
    },
    {
      key: 'cli',
      label: t('claude.subPlatform.cli', 'Claude CLI'),
      icon: <Terminal className="tab-icon" />,
    },
    {
      key: 'instances',
      label: t('instances.title', '应用多开'),
      icon: <Layers className="tab-icon" />,
    },
  ];

  const addModalBusy =
    importing ||
    apiKeyImporting ||
    desktopStarting ||
    desktopCompleting ||
    cliImportingLocal ||
    oauthStarting ||
    oauthCompleting;

  return (
    <div className="ghcp-accounts-page codex-accounts-page claude-accounts-page">
      <div className="page-top-strip">
        <div className="page-top-strip-left">
          <span className="page-top-strip-label">
            {t('settings.general.account', '账号')}
          </span>
          <ManualHelpIconButton className="platform-header-help" />
        </div>
        <div className="page-top-strip-right-placeholder" aria-hidden="true" />
      </div>
      <div className="page-tabs-row page-tabs-center page-tabs-row-with-leading">
        <div className="page-tabs-leading">
          <PlatformGroupSwitcher
            currentPlatformId={claudePlatformId}
            currentLabel={claudePlatformLabel}
            options={platformSwitchOptions}
            currentGroupId={currentPlatformGroup?.id ?? null}
            activePlatformId={claudePlatformId}
          />
        </div>
        <div className="page-tabs filter-tabs claude-page-tabs">
          {claudeTopTabs.map((tab) => (
            <button
              key={tab.key}
              type="button"
              className={`filter-tab${activeSection === tab.key ? ' active' : ''}`}
              onClick={() => setActiveSection(tab.key)}
            >
              {tab.icon}
              <span>{tab.label}</span>
            </button>
          ))}
        </div>
      </div>

      {isInstancesSection ? (
        <ClaudeInstancesContent accountsForSelect={accountsForInstances} />
      ) : (
        <>
          <div className={`ghcp-flow-notice ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} role="note">
            <button
              type="button"
              className="ghcp-flow-notice-toggle"
              onClick={() => setIsFlowNoticeCollapsed((prev) => !prev)}
            >
              <div className="ghcp-flow-notice-title">
                <CircleAlert size={16} />
                <span>{t('claude.flowNotice.title', 'Claude 账号管理说明（点击展开/收起）')}</span>
              </div>
              <ChevronDown size={16} className={`ghcp-flow-notice-arrow ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} />
            </button>
            {!isFlowNoticeCollapsed && (
              <div className="ghcp-flow-notice-body">
                <div className="ghcp-flow-notice-desc">
                  {t(
                    isDesktopSubPlatform ? 'claude.flowNotice.desktopDesc' : 'claude.flowNotice.cliDesc',
                    isDesktopSubPlatform
                      ? '本工具可管理 Claude 登录态。登录会先保存到本地账号库；切号时才写入官方 Claude。'
                      : '本工具可管理 Claude Code OAuth 与多供应商 API Key。OAuth 切号会写入本机 Claude Code 配置；API Key 会写入 Claude Code settings.json 的 env。',
                  )}
                </div>
                <ul className="ghcp-flow-notice-list">
                  <li>
                    {t(
                    isDesktopSubPlatform ? 'claude.flowNotice.desktopPermission' : 'claude.flowNotice.cliPermission',
                    isDesktopSubPlatform
                      ? '权限范围：读取/写入官方 Claude 应用数据目录；Claude 快照保存于本工具本地账号数据。'
                        : '权限范围：读取/写入本机 Claude Code 配置目录与 macOS Keychain 中的 Claude Code 凭据；API Key 账号保存于本工具本地账号数据，并在切换或启动 CLI 时明文写入 settings.json 的 env。',
                    )}
                  </li>
                  <li>
                    {t(
                    isDesktopSubPlatform ? 'claude.flowNotice.desktopNetwork' : 'claude.flowNotice.cliNetwork',
                    isDesktopSubPlatform
                      ? '网络范围：Claude 登录窗口访问 claude.ai；刷新账号会请求 Claude Web 相关接口。'
                        : '网络范围：OAuth 授权访问 Claude 官方授权页和 token/profile/usage 接口；API Key 导入不联网，启动后由 Claude CLI 访问所选供应商。',
                    )}
                  </li>
                </ul>
              </div>
            )}
          </div>

          {message && (
            <div className={`message-bar ${message.tone === 'error' ? 'error' : 'success'}`}>
              {message.text}
              <button onClick={() => setMessage(null)}>
                <X size={14} />
              </button>
            </div>
          )}

          {/* Aggregate capacity projection panel */}
          <AggregateQuotaProjectionChart platform="claude" accounts={filteredAccounts} />

          <div className="toolbar">
            <div className="toolbar-left">
              <div className="search-box">
                <Search size={16} className="search-icon" />
                <input
                  type="text"
                  placeholder={t(
                    isDesktopSubPlatform ? 'claude.searchDesktop' : 'claude.searchCli',
                    isDesktopSubPlatform ? '搜索 Claude 账号...' : '搜索 Claude CLI 账号...',
                  )}
                  value={searchQuery}
                  onChange={(event) => setSearchQuery(event.target.value)}
                />
              </div>
              <div className="view-switcher">
                <button
                  className={`view-btn ${viewMode === 'list' ? 'active' : ''}`}
                  onClick={() => setViewMode('list')}
                  title={t('accounts.view.list', '列表视图')}
                >
                  <List size={16} />
                </button>
                <button
                  className={`view-btn ${viewMode === 'grid' ? 'active' : ''}`}
                  onClick={() => setViewMode('grid')}
                  title={t('accounts.view.grid', '卡片视图')}
                >
                  <LayoutGrid size={16} />
                </button>
              </div>
            </div>
            <div className="toolbar-right">
              <button className="btn btn-primary icon-only" onClick={openAddModal} title={t('common.shared.addAccount', '添加账号')}>
                <Plus size={14} />
              </button>
              <button
                className="btn btn-secondary icon-only"
                onClick={() => void handleRefreshAll()}
                disabled={refreshingAll || subPlatformAccountsCount === 0}
                title={t('common.shared.refreshAll', '刷新全部')}
              >
                <RefreshCw size={14} className={refreshingAll ? 'loading-spinner' : ''} />
              </button>
              <button
                className="btn btn-secondary icon-only"
                onClick={togglePrivacyMode}
                title={privacyModeEnabled ? t('privacy.showSensitive', '显示邮箱') : t('privacy.hideSensitive', '隐藏邮箱')}
              >
                {privacyModeEnabled ? <EyeOff size={14} /> : <Eye size={14} />}
              </button>
              {(selectedVisibleIds.length > 0 ? selectedExportableIds.length > 0 : filteredExportableIds.length > 0) && (
                <button
                  className="btn btn-secondary export-btn icon-only"
                  onClick={() => void handleExport(selectedVisibleIds.length > 0 ? selectedExportableIds : filteredExportableIds)}
                  disabled={exportModal.preparing}
                  title={
                    selectedVisibleIds.length > 0
                      ? `${t('common.shared.export.title', '导出')} (${selectedExportableIds.length})`
                      : t('common.shared.export.title', '导出')
                  }
                >
                  <Upload size={14} />
                </button>
              )}
              {isDesktopSubPlatform && <QuickSettingsPopover type="claude" />}
            </div>
          </div>

          {store.loading && store.accounts.length === 0 ? (
            <div className="loading-container">
              <RefreshCw size={24} className="loading-spinner" />
              <p>{t('common.loading', '加载中...')}</p>
            </div>
          ) : subPlatformAccountsCount === 0 ? (
            <div className="empty-state">
              {isDesktopSubPlatform ? <Monitor size={48} /> : <Terminal size={48} />}
              <h3>{t('common.shared.empty.title', '暂无账号')}</h3>
              <p>
                {t(
                  isDesktopSubPlatform ? 'claude.noDesktopAccounts' : 'claude.noCliAccounts',
                  isDesktopSubPlatform ? '暂无 Claude 账号' : '暂无 Claude CLI 账号',
                )}
              </p>
              <button className="btn btn-primary" onClick={openAddModal}>
                <Plus size={16} />
                {t('common.shared.addAccount', '添加账号')}
              </button>
            </div>
          ) : filteredAccounts.length === 0 ? (
            <div className="empty-state">
              <h3>{t('common.shared.noMatch.title', '没有匹配的账号')}</h3>
              <p>{t('common.shared.noMatch.desc', '请尝试调整搜索或筛选条件')}</p>
            </div>
          ) : (
            <>
              <div className="codex-overview-selection-bar">
                <div className="codex-overview-selection-left">
                  <label className="codex-overview-select-all">
                    <input
                      type="checkbox"
                      checked={isAllFilteredSelected}
                      onChange={toggleSelectAllFiltered}
                    />
                    <span>{t('common.selectAll', '全选')}</span>
                  </label>
                  {selectedVisibleIds.length > 0 && (
                    <>
                      <span className="codex-overview-selected-count">
                        {t('claude.selection.selected', '已选 {{count}}', {
                          count: selectedVisibleIds.length,
                        })}
                      </span>
                      <button
                        type="button"
                        className="codex-overview-clear-selection-btn"
                        onClick={clearSelection}
                      >
                        {t('messages.clearSelection', '取消选择')}
                      </button>
                    </>
                  )}
                </div>
                {selectedVisibleIds.length > 0 && (
                  <div className="codex-overview-selection-actions">
                    {selectedExportableIds.length > 0 && (
                      <button
                        className="btn btn-secondary icon-only"
                        onClick={() => void handleExport(selectedExportableIds)}
                        disabled={exportModal.preparing}
                        title={`${t('common.shared.export.title', '导出')} (${selectedExportableIds.length})`}
                      >
                        <Upload size={14} />
                      </button>
                    )}
                    <button
                      className="btn btn-danger icon-only"
                      onClick={openBatchDeleteConfirm}
                      disabled={selectedDeletableIds.length === 0}
                      title={`${t('common.delete', '删除')} (${selectedDeletableIds.length})`}
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                )}
              </div>

              {viewMode === 'grid' ? (
                <div className="codex-accounts-grid">
                  {filteredAccounts.map((account) => {
                    const displayEmail = getClaudeAccountDisplayEmail(account);
                    const apiProviderLabel = getClaudeApiProviderLabel(account);
                    const authMode = normalizeClaudeAuthMode(account.auth_mode);
                    const isApiKey = authMode === 'api_key';
                    const isDesktopGateway = isClaudeDesktopGatewayAccount(account);
                    const isProviderAccount = isApiKey || isDesktopGateway;
                    const isSponsorApiKeyAccount = isProviderAccount && isClaudeApiKeyFunAccount(account);
                    const isCurrent = currentAccountId === account.id;
                    const isSelected = selectedIds.has(account.id);
                    const tags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
                    const visibleTags = tags.slice(0, 2);
                    const moreTagCount = Math.max(0, tags.length - visibleTags.length);
                    const gatewayProviderTitle = isDesktopGateway ? getDesktopGatewayProviderTitle(account) : '';
                    const cardTitle = isDesktopGateway
                      ? gatewayProviderTitle
                      : isProviderAccount
                      ? apiProviderLabel || displayEmail || t('claude.apiKey.label', 'API Key')
                      : displayEmail;
                    const apiBaseUrlText = account.api_base_url?.trim()
                      || t('claude.apiKey.officialEndpoint', '官方默认');
                    const apiProviderLine = `${t('claude.apiKey.providerLabel', '供应商')}: ${apiProviderLabel || '-'}`;
                    const apiBaseUrlLine = `${t('claude.apiKey.baseUrlLabel', '基础 URL')}: ${apiBaseUrlText}`;
                    return (
                      <div
                        key={account.id}
                        className={`codex-account-card claude-account-card ${isProviderAccount ? 'claude-api-key-card' : ''} ${isSponsorApiKeyAccount ? 'sponsor-api-account' : ''} ${isCurrent ? 'current' : ''} ${isSelected ? 'selected' : ''}`}
                      >
                        <div className="card-top">
                          <div className="card-select">
                            <input
                              type="checkbox"
                              checked={isSelected}
                              onChange={() => toggleAccountSelection(account.id)}
                            />
                          </div>
                          <span className="account-email" title={cardTitle}>
                            {maskAccountText(cardTitle)}
                          </span>
                          {isCurrent && <span className="current-tag">{t('accounts.status.current', '当前')}</span>}
                          {!isProviderAccount && renderPlanControl(account)}
                        </div>
                        {isProviderAccount ? (
                          <>
                            {renderApiKeyLine(account)}
                            <div className="account-sub-line codex-provider-inline-line">
                              <span
                                className="codex-login-subline codex-provider-inline-text"
                                title={apiProviderLine}
                              >
                                {apiProviderLine}
                              </span>
                            </div>
                            <div className="account-sub-line">
                              <span className="codex-login-subline" title={apiBaseUrlLine}>
                                {apiBaseUrlLine}
                              </span>
                            </div>
                            {account.account_note?.trim() && (
                              <div className="account-sub-line">
                                {renderAccountNoteButton(account)}
                              </div>
                            )}
                          </>
                        ) : (
                          <>
                            <div className="account-sub-line">
                              {account.organization_name && (
                                <span className="codex-login-subline" title={account.organization_name}>
                                  {t('claude.account.nickname', '昵称')}: {account.organization_name}
                                </span>
                              )}
                              {renderAccountNoteButton(account)}
                            </div>
                            {account.account_uuid && (
                              <div className="account-sub-line">
                                <span className="codex-login-subline" title={`${t('claude.account.userId', '用户 ID')}: ${account.account_uuid}`}>
                                  {t('claude.account.signedInWith', '使用 {{provider}} 登录', { provider: getClaudeAuthModeLabel(account) })}
                                  {' | '}
                                  {t('claude.account.userId', '用户 ID')}: {maskAccountText(account.account_uuid)}
                                </span>
                              </div>
                            )}
                          </>
                        )}
                        {tags.length > 0 && (
                          <div className="card-tags">
                            {visibleTags.map((tag, index) => (
                              <span key={`${account.id}-${tag}-${index}`} className="tag-pill">{tag}</span>
                            ))}
                            {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
                          </div>
                        )}
                        {isProviderAccount ? (
                          renderApiKeyStatsPanel(account)
                        ) : (
                          renderQuotaSummary(account, 'card')
                        )}
                        <div className="codex-card-bottom">
                          <span className="card-date">{formatDate(account.created_at)}</span>
                          <div className="card-footer">
                            {renderAccountActions(account, 'card')}
                          </div>
                        </div>
                      </div>
                    );
                  })}
                </div>
              ) : (
                <div className="account-table-container claude-account-table-container">
                  <table className="account-table claude-account-table">
                    <thead>
                      <tr>
                        <th style={{ width: 40 }}>
                          <input
                            type="checkbox"
                            checked={isAllFilteredSelected}
                            onChange={toggleSelectAllFiltered}
                          />
                        </th>
                        <th style={{ width: 260 }}>{t('common.shared.columns.account', '账号')}</th>
                        <th style={{ width: 140 }}>{t('accounts.columns.plan', '套餐')}</th>
                        <th>{t('claude.quota.title', '额度')}</th>
                        <th style={{ width: 180 }}>{t('accounts.columns.createdAt', '创建时间')}</th>
                        <th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th>
                      </tr>
                    </thead>
                    <tbody>
                      {filteredAccounts.map((account) => {
                        const displayEmail = getClaudeAccountDisplayEmail(account);
                        const apiProviderLabel = getClaudeApiProviderLabel(account);
                        const authMode = normalizeClaudeAuthMode(account.auth_mode);
                        const isApiKey = authMode === 'api_key';
                        const isDesktopGateway = isClaudeDesktopGatewayAccount(account);
                        const isProviderAccount = isApiKey || isDesktopGateway;
                        const isSponsorApiKeyAccount = isProviderAccount && isClaudeApiKeyFunAccount(account);
                        const isCurrent = currentAccountId === account.id;
                        const isSelected = selectedIds.has(account.id);
                        const tags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
                        const tableTitle = isDesktopGateway
                          ? getDesktopGatewayProviderTitle(account)
                          : isApiKey
                            ? apiProviderLabel || displayEmail
                            : displayEmail;
                        return (
                          <tr key={account.id} className={`${isCurrent ? 'current' : ''} ${isSelected ? 'selected' : ''} ${isSponsorApiKeyAccount ? 'sponsor-api-account' : ''}`}>
                            <td>
                              <input
                                type="checkbox"
                                checked={isSelected}
                                onChange={() => toggleAccountSelection(account.id)}
                              />
                            </td>
                            <td>
                              <div className="account-cell">
                                <div className="account-main-line">
                                  <span className="account-email-text" title={tableTitle}>
                                    {maskAccountText(tableTitle)}
                                  </span>
                                  {isCurrent && <span className="mini-tag current">{t('accounts.status.current', '当前')}</span>}
                                </div>
                                <div className="account-sub-line codex-account-meta-inline">
                                  {account.organization_name && !isProviderAccount && (
                                    <span className="codex-login-subline" title={account.organization_name}>
                                      {t('claude.account.nickname', '昵称')}: {account.organization_name}
                                    </span>
                                  )}
                                  {renderAccountNoteButton(account)}
                                  {tags.slice(0, 2).map((tag, index) => (
                                    <span key={`${account.id}-table-${tag}-${index}`} className="tag-pill">{tag}</span>
                                  ))}
                                </div>
                                {account.account_uuid && !isProviderAccount && (
                                  <div className="account-sub-line">
                                    <span className="codex-login-subline" title={`${t('claude.account.userId', '用户 ID')}: ${account.account_uuid}`}>
                                      {t('claude.account.signedInWith', '使用 {{provider}} 登录', { provider: getClaudeAuthModeLabel(account) })}
                                      {' | '}
                                      {t('claude.account.userId', '用户 ID')}: {maskAccountText(account.account_uuid)}
                                    </span>
                                  </div>
                                )}
                                {isProviderAccount && (
                                  <div className="account-sub-line">
                                    <span className="codex-login-subline" title={account.api_base_url || apiProviderLabel}>
                                      {t('claude.apiKey.providerLabel', '供应商')}: {apiProviderLabel || '-'}
                                      {account.api_base_url ? ` | ${account.api_base_url}` : ''}
                                    </span>
                                  </div>
                                )}
                              </div>
                            </td>
                            <td>{isProviderAccount ? null : renderPlanControl(account)}</td>
                            <td>{isProviderAccount ? renderApiKeyStatsPanel(account) : renderQuotaSummary(account, 'table')}</td>
                            <td>{formatDate(account.created_at)}</td>
                            <td className="sticky-action-cell table-action-cell">
                              {renderAccountActions(account, 'table')}
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              )}
            </>
          )}
        </>
      )}

      {showAddModal && (
        <div className="modal-overlay">
          <div className="modal ghcp-add-modal claude-add-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>
                {t(
                  editingDesktopGatewayAccountId
                    ? 'claude.desktopGateway.editTitle'
                    : isDesktopSubPlatform
                      ? 'claude.addAccount.desktopTitle'
                      : 'claude.addAccount.cliTitle',
                  editingDesktopGatewayAccountId
                    ? '编辑 Claude Gateway'
                    : isDesktopSubPlatform
                      ? '添加 Claude 账号'
                      : '添加 Claude CLI 账号',
                )}
              </h2>
              <button className="modal-close" onClick={closeAddModal} aria-label={t('common.close', '关闭')}>
                <X />
              </button>
            </div>
            {!editingDesktopGatewayAccountId && <div className="modal-tabs">
              {isDesktopSubPlatform ? (
                <>
                  <button
                    className={`modal-tab ${addTab === 'desktop' ? 'active' : ''}`}
                    onClick={() => selectAddTab('desktop')}
                    type="button"
                  >
                    <Monitor size={14} />
                    <span className="modal-tab-label">{t('claude.addTabs.desktop', 'Desktop')}</span>
                  </button>
                  <button
                    className={`modal-tab ${addTab === 'desktopGateway' ? 'active' : ''}`}
                    onClick={() => selectAddTab('desktopGateway')}
                    type="button"
                  >
                    <KeyRound size={14} />
                    <span className="modal-tab-label">{t('claude.addTabs.desktopGateway', 'Gateway')}</span>
                  </button>
                </>
              ) : (
                <>
                  <button
                    className={`modal-tab ${addTab === 'oauth' ? 'active' : ''}`}
                    onClick={() => selectAddTab('oauth')}
                    type="button"
                  >
                    <Globe size={14} />
                    <span className="modal-tab-label">{t('claude.addTabs.oauth', 'OAuth')}</span>
                  </button>
                  <button
                    className={`modal-tab ${addTab === 'apikey' ? 'active' : ''}`}
                    onClick={() => selectAddTab('apikey')}
                    type="button"
                  >
                    <KeyRound size={14} />
                    <span className="modal-tab-label">{t('claude.addTabs.apiKey', 'API Key')}</span>
                  </button>
                </>
              )}
              <button
                className={`modal-tab ${addTab === 'import' ? 'active' : ''}`}
                onClick={() => selectAddTab('import')}
                type="button"
              >
                <Database size={14} />
                <span className="modal-tab-label">
                  {isDesktopSubPlatform
                    ? t('settings.transfer.backup.downloadJsonAction', 'JSON')
                    : t('claude.addTabs.import', '本地/JSON')}
                </span>
              </button>
            </div>}
            <div className="modal-body">
              <ModalErrorMessage message={addModalError} scrollKey={addModalErrorScrollKey} />
              {addTab === 'desktop' && (
                <div className="add-section">
                  <div className="add-method-card">
                    <div className="method-icon">
                      <Monitor size={20} />
                    </div>
                    <div>
                      <h3>{t('claude.desktopOAuth.title', 'Claude 登录')}</h3>
                      <p>
                        {t(
                          'claude.desktopOAuth.desc',
                          '在本工具打开 Claude 登录窗口，支持 Google、Apple、邮箱和 free 账号。',
                        )}
                      </p>
                    </div>
                    <button
                      className="btn btn-primary"
                      onClick={() => void handleStartDesktopLogin()}
                      disabled={addModalBusy || Boolean(desktopLogin)}
                    >
                      {desktopStarting ? <RefreshCw size={14} className="loading-spinner" /> : <ExternalLink size={14} />}
                      {desktopStarting
                        ? t('claude.desktopOAuth.preparingRuntime', '准备登录组件...')
                        : t('claude.desktopOAuth.start', '打开登录')}
                    </button>
                  </div>
                  <p className="oauth-hint">
                    {t(
                      'claude.desktopOAuth.hint',
                      '登录态会先保存到本工具本地账号库，不会立刻写入官方 Claude；切号时才写回 Claude。',
                    )}
                  </p>
                  <p className="oauth-hint">
                    {t(
                      'claude.desktopOAuth.runtimeHint',
                      '首次使用时会下载并校验 Electron 登录组件到本地应用数据目录；安装包不内置，之后复用本地缓存。',
                    )}
                  </p>
                  {desktopStarting && desktopLoginProgress && (
                    <div className="claude-desktop-login-progress" role="status" aria-live="polite">
                      <div className="claude-desktop-login-progress__head">
                        <strong>{getClaudeDesktopLoginProgressLabel(t, desktopLoginProgress)}</strong>
                        <span>{clampProgressPercent(desktopLoginProgress.percent)}%</span>
                      </div>
                      <div className="claude-desktop-login-progress__bar" aria-hidden="true">
                        <span style={{ width: `${clampProgressPercent(desktopLoginProgress.percent)}%` }} />
                      </div>
                      <p>{getClaudeDesktopLoginProgressDetail(t, desktopLoginProgress)}</p>
                    </div>
                  )}
                  <div className="form-group">
                    <label>{t('claude.desktopOAuth.nameLabel', '账号名称')}</label>
                    <input
                      className="form-input"
                      value={desktopAccountNameInput}
                      onChange={(event) => setDesktopAccountNameInput(event.target.value)}
                      placeholder={t('claude.desktopOAuth.namePlaceholder', '可选，例如 Claude Free')}
                    />
                  </div>
                  {desktopLogin && (
                    <div className="oauth-url-section">
                      <p className="section-desc">
                        {t(
                          'claude.desktopOAuth.waiting',
                          '请在已打开的 Claude 授权窗口完成登录。看到聊天页后回到这里点击完成导入。',
                        )}
                      </p>
                      <div className="oauth-url-box">
                        <input
                          value={desktopLogin.userDataDir}
                          readOnly
                          aria-label={t('claude.desktopOAuth.profileDir', '隔离 profile 目录')}
                        />
                      </div>
                      <button
                        type="button"
                        className="btn btn-primary btn-full"
                        onClick={() => void handleCompleteDesktopLogin()}
                        disabled={addModalBusy}
                      >
                        {desktopCompleting ? <RefreshCw size={14} className="loading-spinner" /> : <Download size={14} />}
                        {desktopCompleting ? t('common.loading', '加载中...') : t('claude.desktopOAuth.complete', '完成导入')}
                      </button>
                    </div>
                  )}
                </div>
              )}
              {addTab === 'oauth' && (
                <div className="add-section">
                  <div className="add-method-card">
                    <div className="method-icon">
                      <Globe size={20} />
                    </div>
                    <div>
                      <h3>{t('claude.oauth.title', 'Claude OAuth 授权')}</h3>
                      <p>
                        {t(
                          'claude.oauth.desc',
                          '打开 Claude 官方 OAuth 授权页，完成后粘贴回调链接或 code 导入账号。',
                        )}
                      </p>
                    </div>
                  </div>
                  <p className="oauth-hint">
                    {t(
                      'claude.oauth.proRequiredHint',
                      'Claude 官方 OAuth 通常用于 Claude Code 授权；如果页面停在升级或无权限页面，可改用 Claude 登录。',
                    )}
                  </p>
                  <div className="oauth-url-section">
                    <p className="section-desc">
                      {t(
                        'claude.oauth.openInstruction',
                        '点击下方按钮，在浏览器中完成 Claude OAuth 授权。',
                      )}
                    </p>
                    <label className="oauth-url-label">
                      {t('claude.oauth.authUrl', '授权链接')}
                    </label>
                    <div className="oauth-url-box">
                      <input
                        value={
                          oauthLogin?.verificationUri
                            ?? (oauthStarting ? t('claude.oauth.preparing', '正在生成授权链接...') : '')
                        }
                        readOnly
                        aria-label={t('claude.oauth.authUrl', '授权链接')}
                      />
                      <button
                        type="button"
                        className="oauth-copy-button"
                        onClick={() => void handleCopyOAuthUrl()}
                        disabled={!oauthLogin?.verificationUri}
                      >
                        {oauthCopied ? <Check size={14} /> : <Copy size={14} />}
                        {oauthCopied ? t('common.success', '成功') : t('common.copy', '复制')}
                      </button>
                    </div>
                    <button
                      type="button"
                      className="btn btn-primary btn-full"
                      onClick={() => void handleOpenOAuthUrl()}
                      disabled={addModalBusy || !oauthLogin?.verificationUri}
                    >
                      {oauthStarting ? <RefreshCw size={14} className="loading-spinner" /> : <Globe size={14} />}
                      {oauthStarting ? t('common.loading', '加载中...') : t('claude.oauth.openInBrowser', '在浏览器中打开')}
                    </button>
                    <p className="section-desc">
                      {t(
                        'claude.oauth.waiting',
                        '完成授权后，将最终页面地址或页面显示的 code 粘贴到下方。',
                      )}
                    </p>
                    <div className="oauth-url-box oauth-manual-input">
                      <input
                        value={oauthCallbackInput}
                        onChange={(event) => {
                          setOauthCallbackInput(event.target.value);
                          setAddModalError(null);
                        }}
                        placeholder={t('claude.oauth.callbackPlaceholder', '粘贴回调链接或授权 code')}
                      />
                      <button
                        type="button"
                        className="btn btn-primary"
                        onClick={() => void handleCompleteOAuth()}
                        disabled={addModalBusy || !oauthLogin}
                      >
                        {oauthCompleting ? <RefreshCw size={14} className="loading-spinner" /> : <Download size={14} />}
                        {oauthCompleting ? t('common.loading', '加载中...') : t('claude.oauth.complete', '完成导入')}
                      </button>
                    </div>
                    <div className="oauth-url-box oauth-manual-input">
                      <input
                        value={oauthEmailHint}
                        onChange={(event) => setOauthEmailHint(event.target.value)}
                        placeholder={t('claude.oauth.emailPlaceholder', '邮箱（无法自动识别时填写）')}
                      />
                    </div>
                  </div>
                </div>
              )}
              {(addTab === 'apikey' || addTab === 'desktopGateway') && (
                <div className="add-section">
                  <p className="section-desc">
                    {t(
                      addTab === 'desktopGateway' ? 'claude.desktopGateway.desc' : 'claude.apiKey.desc',
                      addTab === 'desktopGateway'
                        ? '保存 Gateway API Key 作为 Claude 3P 配置；切换时写入受管 profile 并用官方 Claude 启动。'
                        : '保存 Claude CLI API Key 作为独立凭证；切换或启动 CLI 时会写入 Claude Code settings.json 的 env，不会写入 Claude 登录态。',
                    )}
                  </p>
                  <div className="form-group">
                    <label>{t('claude.apiKey.providerLabel', '供应商')}</label>
                    <div className="claude-provider-chip-list">
                      {(addTab === 'desktopGateway'
                        ? CLAUDE_DESKTOP_GATEWAY_PROVIDER_PRESETS
                        : CLAUDE_API_PROVIDER_PRESETS
                      ).map((preset) => (
                        <button
                          key={preset.id}
                          type="button"
                          className={`claude-provider-chip ${preset.isPartner ? 'sponsor' : ''} ${apiProviderPresetId === preset.id ? 'active' : ''}`}
                          onClick={() => handleSelectApiProviderPreset(preset.id)}
                        >
                          <span>{preset.name}</span>
                          {preset.isPartner && <Star size={12} className="api-provider-chip-badge" />}
                        </button>
                      ))}
                      <button
                        type="button"
                        className={`claude-provider-chip ${apiProviderPresetId === (
                          addTab === 'desktopGateway'
                            ? CLAUDE_DESKTOP_GATEWAY_PROVIDER_CUSTOM_ID
                            : CLAUDE_API_PROVIDER_CUSTOM_ID
                        ) ? 'active' : ''}`}
                        onClick={() => handleSelectApiProviderPreset(
                          addTab === 'desktopGateway'
                            ? CLAUDE_DESKTOP_GATEWAY_PROVIDER_CUSTOM_ID
                            : CLAUDE_API_PROVIDER_CUSTOM_ID,
                        )}
                      >
                        <span>{t('claude.apiKey.customProvider', '自定义')}</span>
                      </button>
                    </div>
                  </div>
                  {selectedFormProviderPreset && selectedFormProviderPreset.baseUrls.length > 1 && (
                    <div className="form-group">
                      <label>{t('claude.apiKey.endpointLabel', '供应商端点')}</label>
                      <div className="claude-provider-endpoint-list">
                        {selectedFormProviderPreset.baseUrls.map((baseUrl) => (
                          <button
                            key={baseUrl || 'official'}
                            type="button"
                            className={`claude-provider-endpoint-chip ${apiBaseUrlInput === applyClaudeApiProviderTemplateValue(baseUrl, apiProviderTemplateValues) ? 'active' : ''}`}
                            onClick={() => {
                              setApiBaseUrlInput(applyClaudeApiProviderTemplateValue(baseUrl, apiProviderTemplateValues));
                              if (addTab === 'desktopGateway') {
                                clearDesktopGatewayModelDiscoveryStatus();
                              }
                            }}
                          >
                            {applyClaudeApiProviderTemplateValue(baseUrl, apiProviderTemplateValues) ||
                              t('claude.apiKey.officialEndpoint', '官方默认')}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}
                  {addTab !== 'desktopGateway' && selectedApiProviderPreset?.templateValues &&
                    Object.entries(selectedApiProviderPreset.templateValues).map(([key, config]) => (
                      <div className="form-group" key={key}>
                        <label>{config.label}</label>
                        <input
                          className="form-input"
                          type={key.includes('SECRET') ? 'password' : 'text'}
                          value={apiProviderTemplateValues[key] ?? ''}
                          onChange={(event) => {
                            const nextValues = {
                              ...apiProviderTemplateValues,
                              [key]: event.target.value,
                            };
                            setApiProviderTemplateValues(nextValues);
                            setApiBaseUrlInput(
                              applyClaudeApiProviderTemplateValue(
                                selectedApiProviderPreset.baseUrls[0] ?? '',
                                nextValues,
                              ),
                            );
                            setAddModalError(null);
                          }}
                          placeholder={config.placeholder}
                          autoComplete="off"
                          spellCheck={false}
                        />
                      </div>
                    ))}
                  <div className="form-group">
                    <label>{t('claude.apiKey.baseUrlLabel', 'Base URL')}</label>
                    <input
                      className="form-input"
                        value={apiBaseUrlInput}
                        onChange={(event) => {
                          setApiBaseUrlInput(event.target.value);
                          setApiProviderPresetId(
                            addTab === 'desktopGateway'
                              ? CLAUDE_DESKTOP_GATEWAY_PROVIDER_CUSTOM_ID
                              : CLAUDE_API_PROVIDER_CUSTOM_ID,
                          );
                          setApiProviderTemplateValues({});
                          setApiKeyModelCatalogOverride(null);
                          if (addTab === 'desktopGateway') {
                            clearDesktopGatewayModelDiscoveryStatus();
                          } else {
                            setDesktopGatewayModelsError(null);
                            setDesktopGatewayModelsMessage(null);
                          }
                          setAddModalError(null);
                        }}
                      placeholder={t('claude.apiKey.baseUrlPlaceholder', '留空使用 Anthropic 官方默认地址')}
                    />
                  </div>
                  {addTab === 'desktopGateway' && (
                    <div className="form-group">
                      <label>{t('claude.desktopGateway.authScheme', 'Auth Scheme')}</label>
                      <div className="claude-gateway-segmented claude-gateway-auth-segmented">
                        {['bearer', 'x-api-key', 'auto'].map((scheme) => (
                          <button
                            key={scheme}
                            type="button"
                            className={`claude-provider-endpoint-chip ${desktopGatewayAuthScheme === scheme ? 'active' : ''}`}
                            onClick={() => {
                              setDesktopGatewayAuthScheme(scheme);
                              clearDesktopGatewayModelDiscoveryStatus();
                            }}
                          >
                            {scheme}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}
                  <div className="form-group">
                    <label>{t('claude.apiKey.nameLabel', '账号名称')}</label>
                    <input
                      className="form-input"
                      value={apiKeyNameInput}
                      onChange={(event) => setApiKeyNameInput(event.target.value)}
                      placeholder={t('claude.apiKey.namePlaceholder', '可选，例如 Anthropic API')}
                    />
                  </div>
                  <div className="form-group">
                    <label>{t('claude.apiKey.keyLabel', 'API Key')}</label>
                    <div className="oauth-url-box oauth-manual-input claude-secret-input">
                      <input
                        type={apiKeyInputVisible ? 'text' : 'password'}
                        value={apiKeyInput}
                        onChange={(event) => {
                          setApiKeyInput(event.target.value);
                          if (addTab === 'desktopGateway') {
                            clearDesktopGatewayModelDiscoveryStatus();
                          } else {
                            setDesktopGatewayModelsError(null);
                            setDesktopGatewayModelsMessage(null);
                          }
                          setAddModalError(null);
                        }}
                        placeholder={t('claude.apiKey.placeholder', '粘贴供应商 API Key')}
                        autoComplete="off"
                        spellCheck={false}
                      />
                      <button
                        type="button"
                        className="codex-secret-toggle-btn"
                        onClick={() => setApiKeyInputVisible((visible) => !visible)}
                        title={
                          apiKeyInputVisible
                            ? t('claude.apiKey.hide', '隐藏 API Key')
                            : t('claude.apiKey.show', '显示 API Key')
                        }
                        aria-label={
                          apiKeyInputVisible
                            ? t('claude.apiKey.hide', '隐藏 API Key')
                            : t('claude.apiKey.show', '显示 API Key')
                        }
                      >
                        {apiKeyInputVisible ? <EyeOff size={16} /> : <Eye size={16} />}
                      </button>
                    </div>
                  </div>
                  {shouldShowDesktopGatewayRouting && (
                    <>
                      <div className="form-group">
                        <label>{t('claude.desktopGateway.connectionMode', '连接方式')}</label>
                        <div className="claude-gateway-segmented claude-gateway-mode-segmented">
                          {[
                            {
                              value: 'direct',
                              label: t('claude.desktopGateway.modeDirect', '直连'),
                            },
                            {
                              value: 'local_mapping',
                              label: t('claude.desktopGateway.modeLocalMapping', '本地网关映射'),
                            },
                          ].map((mode) => (
                            <button
                              key={mode.value}
                              type="button"
                              className={`claude-provider-endpoint-chip ${desktopGatewayConnectionMode === mode.value ? 'active' : ''}`}
                              onClick={() => {
                                const nextMode = mode.value as ClaudeDesktopGatewayConnectionMode;
                                setDesktopGatewayConnectionMode(nextMode);
                                if (nextMode === 'local_mapping' && desktopGatewayModelMappings.length === 0) {
                                  const desktopModels = parseClaudeDesktopGatewayModels(desktopGatewayModelsInput);
                                  setDesktopGatewayModelMappings(buildClaudeDesktopGatewayMappings(
                                    desktopModels.length > 0 ? desktopModels : DEFAULT_CLAUDE_DESKTOP_GATEWAY_MODELS,
                                    desktopGatewayUpstreamModels,
                                  ));
                                }
                                setDesktopGatewayModelsError(null);
                                setDesktopGatewayModelsMessage(null);
                                setAddModalError(null);
                              }}
                            >
                              {mode.label}
                            </button>
                          ))}
                        </div>
                      </div>
                      <div className="form-group">
                        <div className="claude-gateway-models-header">
                          <label>{t('claude.desktopGateway.modelsLabel', '模型目录')}</label>
                        </div>
                        {desktopGatewayConnectionMode === 'direct' ? (
                          <textarea
                            className="form-input token-input claude-gateway-models-input"
                            rows={4}
                            value={desktopGatewayModelsInput}
                            onChange={(event) => {
                              setDesktopGatewayModelsInput(event.target.value);
                              setDesktopGatewayModelsError(null);
                              setDesktopGatewayModelsMessage(null);
                            }}
                            placeholder={t('claude.desktopGateway.modelsPlaceholder', '每行一个模型 ID')}
                            spellCheck={false}
                          />
                        ) : (
                          <div className="claude-gateway-mapping-list">
                            {!shouldShowDesktopGatewayMappingEditor && (
                              <div className="claude-gateway-mapping-summary">
                                <div className="claude-gateway-mapping-summary-header">
                                  <div>
                                    <strong>
                                      {t('claude.desktopGateway.mappingSummary', '已配置 {{count}} 个映射', {
                                        count: desktopGatewayModelMappings.length,
                                      })}
                                    </strong>
                                    <span>{t('claude.desktopGateway.mappingSummaryHint', '保存前可展开检查或修改。')}</span>
                                  </div>
                                  <button
                                    type="button"
                                    className="btn btn-secondary"
                                    onClick={() => setDesktopGatewayMappingsExpanded(true)}
                                  >
                                    <Pencil size={14} />
                                    {t('claude.desktopGateway.editMappings', '编辑映射')}
                                  </button>
                                </div>
                                <div className="claude-gateway-mapping-preview-list">
                                  {desktopGatewayMappingPreview.map((mapping, index) => (
                                    <div className="claude-gateway-mapping-preview-item" key={`${index}-${mapping.upstreamModel}-${mapping.desktopModel}`}>
                                      <span title={mapping.upstreamModel}>{mapping.upstreamModel || '-'}</span>
                                      <span>-&gt;</span>
                                      <span title={mapping.desktopModel}>{mapping.desktopModel || '-'}</span>
                                    </div>
                                  ))}
                                  {desktopGatewayModelMappings.length > desktopGatewayMappingPreview.length && (
                                    <div className="claude-gateway-mapping-preview-more">
                                      {t('claude.desktopGateway.moreMappings', '还有 {{count}} 个', {
                                        count: desktopGatewayModelMappings.length - desktopGatewayMappingPreview.length,
                                      })}
                                    </div>
                                  )}
                                </div>
                              </div>
                            )}
                            {shouldShowDesktopGatewayMappingEditor && (
                              <>
                                {desktopGatewayModelMappings.length > 0 && (
                                  <div className="claude-gateway-mapping-editor-toolbar">
                                    <span>{t('claude.desktopGateway.mappingEditorTitle', '模型映射')}</span>
                                    <button
                                      type="button"
                                      className="btn btn-secondary"
                                      onClick={() => setDesktopGatewayMappingsExpanded(false)}
                                    >
                                      {t('claude.desktopGateway.hideMappings', '收起')}
                                    </button>
                                  </div>
                                )}
                                {desktopGatewayModelMappings.map((mapping, index) => {
                                  const desktopModelOptions = buildClaudeDesktopGatewayDesktopModelOptions(
                                    t('claude.apiKey.customProvider', '自定义'),
                                  );
                                  const desktopModelInOptions = desktopModelOptions.some((option) => option.value === mapping.desktopModel);
                                  const desktopDropdownValue = desktopModelInOptions && mapping.desktopModel
                                    ? mapping.desktopModel
                                    : CLAUDE_DESKTOP_GATEWAY_CUSTOM_DESKTOP_MODEL;
                                  const showCustomDesktopInput =
                                    desktopDropdownValue === CLAUDE_DESKTOP_GATEWAY_CUSTOM_DESKTOP_MODEL;
                                  return (
                                    <div className="claude-gateway-mapping-row" key={`${index}-${mapping.upstreamModel}-${mapping.desktopModel}`}>
                                      <input
                                        className="form-input"
                                        value={mapping.upstreamModel}
                                        onChange={(event) => {
                                          const next = [...desktopGatewayModelMappings];
                                          next[index] = { ...mapping, upstreamModel: event.target.value };
                                          setDesktopGatewayModelMappings(next);
                                          setDesktopGatewayModelsError(null);
                                          setAddModalError(null);
                                        }}
                                        placeholder={t('claude.desktopGateway.upstreamModelPlaceholder', '上游真实模型名')}
                                        spellCheck={false}
                                      />
                                      <div className="claude-gateway-mapped-model-field">
                                        <SingleSelectDropdown
                                          value={desktopDropdownValue}
                                          options={desktopModelOptions}
                                          onChange={(value) => {
                                            const next = [...desktopGatewayModelMappings];
                                            next[index] = {
                                              ...mapping,
                                              desktopModel:
                                                value === CLAUDE_DESKTOP_GATEWAY_CUSTOM_DESKTOP_MODEL
                                                  ? desktopModelInOptions
                                                    ? ''
                                                    : mapping.desktopModel
                                                  : value,
                                            };
                                            setDesktopGatewayModelMappings(next);
                                            setDesktopGatewayModelsError(null);
                                            setAddModalError(null);
                                          }}
                                          ariaLabel={t('claude.desktopGateway.desktopModelPlaceholder', 'Claude 模型名')}
                                          placeholder={t('claude.desktopGateway.desktopModelPlaceholder', 'Claude 模型名')}
                                          menuWidth={260}
                                        />
                                        {showCustomDesktopInput && (
                                          <input
                                            className="form-input"
                                            value={mapping.desktopModel}
                                            onChange={(event) => {
                                              const next = [...desktopGatewayModelMappings];
                                              next[index] = { ...mapping, desktopModel: event.target.value };
                                              setDesktopGatewayModelMappings(next);
                                              setDesktopGatewayModelsError(null);
                                              setAddModalError(null);
                                            }}
                                            placeholder={t('claude.desktopGateway.desktopModelPlaceholder', 'Claude 模型名')}
                                            spellCheck={false}
                                          />
                                        )}
                                      </div>
                                      <label
                                        className="claude-gateway-supports1m-toggle"
                                        title={t('claude.desktopGateway.supports1mLabel', '声明支持 1M')}
                                      >
                                        <input
                                          type="checkbox"
                                          checked={mapping.supports1m === true}
                                          onChange={(event) => {
                                            const next = [...desktopGatewayModelMappings];
                                            next[index] = { ...mapping, supports1m: event.target.checked };
                                            setDesktopGatewayModelMappings(next);
                                            setDesktopGatewayModelsError(null);
                                            setAddModalError(null);
                                          }}
                                        />
                                        <span className="claude-gateway-supports1m-box" aria-hidden="true">
                                          {mapping.supports1m === true && <Check size={12} />}
                                        </span>
                                        <span>{t('claude.desktopGateway.supports1mShort', '1M')}</span>
                                      </label>
                                      <button
                                        type="button"
                                        className="btn btn-secondary"
                                        onClick={() => {
                                          setDesktopGatewayModelMappings((prev) => prev.filter((_, itemIndex) => itemIndex !== index));
                                          setAddModalError(null);
                                        }}
                                      >
                                        <Trash2 size={14} />
                                        {t('common.delete', '删除')}
                                      </button>
                                    </div>
                                  );
                                })}
                                <button
                                  type="button"
                                  className="btn btn-secondary"
                                  onClick={() => {
                                    setDesktopGatewayModelMappings((prev) => [
                                      ...prev,
                                      {
                                        desktopModel: getNextClaudeDesktopGatewaySafeModel(
                                          prev.map((mapping) => mapping.desktopModel),
                                        ),
                                        upstreamModel: '',
                                        supports1m: false,
                                      },
                                    ]);
                                    setDesktopGatewayMappingsExpanded(true);
                                    setAddModalError(null);
                                  }}
                                >
                                  <Plus size={14} />
                                  {t('claude.desktopGateway.addMapping', '添加映射')}
                                </button>
                              </>
                            )}
                          </div>
                        )}
                      </div>
                    </>
                  )}
                  {shouldShowDesktopGatewayRouting && (
                    <div className="form-group claude-gateway-model-status-group">
                      {desktopGatewayModelsLoading && (
                        <div className="add-status loading">
                          <RefreshCw size={14} className="loading-spinner" />
                          <span>{t('claude.desktopGateway.modelsLoading', '正在获取模型目录...')}</span>
                        </div>
                      )}
                      {desktopGatewayModelsMessage && (
                        <div className="add-status success">
                          <CheckCircle size={14} />
                          <span>{desktopGatewayModelsMessage}</span>
                        </div>
                      )}
                      {desktopGatewayModelsError && (
                        <div className="add-status error">
                          <AlertTriangle size={14} />
                          <span>{desktopGatewayModelsError}</span>
                        </div>
                      )}
                    </div>
                  )}
                  <p className="oauth-hint">
                    {t(
                      addTab === 'desktopGateway' ? 'claude.desktopGateway.hint' : 'claude.apiKey.hint',
                      addTab === 'desktopGateway'
                        ? 'Gateway 账号不会读取 Claude 订阅信息；API Key 会按官方 3P 配置写入受管 profile，用于启动 Claude。'
                        : 'API Key 账号仅用于 Claude CLI；会以明文 env 写入 Claude Code settings.json，不会写入 Claude 登录态，也不支持订阅额度刷新。',
                    )}
                  </p>
                  <button
                    className="btn btn-primary btn-full"
                    onClick={() => void handleImportApiKey()}
                    disabled={addModalBusy || !apiKeyInput.trim()}
                  >
                    {apiKeyImporting ? <RefreshCw size={14} className="loading-spinner" /> : <KeyRound size={14} />}
                    {apiKeyImporting
                      ? t('common.loading', '加载中...')
                      : t(
                        addTab === 'desktopGateway'
                          ? editingDesktopGatewayAccountId
                            ? 'claude.desktopGateway.updateAction'
                            : 'claude.desktopGateway.importAction'
                          : 'claude.apiKey.importAction',
                        addTab === 'desktopGateway'
                          ? editingDesktopGatewayAccountId
                            ? '保存 Gateway'
                            : '导入 Gateway'
                          : '导入 API Key',
                      )}
                  </button>
                </div>
              )}
              {addTab === 'import' && (
                <div className="add-section">
                  {!isDesktopSubPlatform && (
                    <div className="add-method-card">
                      <div className="method-icon">
                        <Terminal size={20} />
                      </div>
                      <div>
                        <h3>{t('claude.cli.localTitle', '导入当前 Claude Code')}</h3>
                        <p>
                          {t(
                            'claude.cli.localDesc',
                            '读取本机 Claude Code 当前 OAuth 登录态，复制为本工具本地账号快照。',
                          )}
                        </p>
                      </div>
                      <button
                        className="btn btn-secondary"
                        onClick={() => void handleImportCliFromLocal()}
                        disabled={addModalBusy}
                      >
                        {cliImportingLocal ? (
                          <RefreshCw size={14} className="loading-spinner" />
                        ) : (
                          <Download size={14} />
                        )}
                        {cliImportingLocal
                          ? t('common.loading', '加载中...')
                          : t('claude.desktopOAuth.localAction', '导入')}
                      </button>
                    </div>
                  )}
                  <div className="form-group">
                    <label>{t('claude.import.jsonLabel', 'JSON 数据')}</label>
                    <textarea
                      className="form-input"
                      rows={8}
                      value={jsonInput}
                      placeholder={t(
                        isDesktopSubPlatform ? 'claude.import.desktopJsonPlaceholder' : 'claude.import.cliJsonPlaceholder',
                        isDesktopSubPlatform
                          ? '粘贴导出的 Claude Gateway 账号 JSON'
                          : '粘贴导出的 Claude CLI 账号 JSON',
                      )}
                      onChange={(event) => setJsonInput(event.target.value)}
                    />
                  </div>
                  <input
                    ref={importFileInputRef}
                    type="file"
                    accept=".json,application/json"
                    style={{ display: 'none' }}
                    onChange={(event) => {
                      const file = event.target.files?.[0];
                      event.currentTarget.value = '';
                      if (file) void handleImportFile(file);
                    }}
                  />
                </div>
              )}
            </div>
            {addTab === 'import' && (
              <div className="modal-footer">
                <button
                  className="btn btn-secondary"
                  onClick={() => importFileInputRef.current?.click()}
                  disabled={addModalBusy}
                >
                  <FileJson size={14} />
                  {t('common.shared.import.file', '选择文件')}
                </button>
                <button
                  className="btn btn-primary"
                  onClick={() => void importJsonContent(jsonInput)}
                  disabled={addModalBusy}
                >
                  {importing ? <RefreshCw size={14} className="loading-spinner" /> : <Upload size={14} />}
                  {importing ? t('common.loading', '加载中...') : t('common.shared.import.label', '导入')}
                </button>
              </div>
            )}
          </div>
        </div>
      )}

      <ExportJsonModal
        isOpen={exportModal.showModal}
        title={`${t('common.shared.export.title', '导出')} JSON`}
        jsonContent={exportModal.jsonContent}
        hidden={exportModal.hidden}
        copied={exportModal.copied}
        saving={exportModal.saving}
        savedPath={exportModal.savedPath}
        canOpenSavedDirectory={exportModal.canOpenSavedDirectory}
        pathCopied={exportModal.pathCopied}
        onClose={exportModal.closeModal}
        onToggleHidden={exportModal.toggleHidden}
        onCopyJson={exportModal.copyJson}
        onSaveJson={exportModal.saveJson}
        onOpenSavedDirectory={exportModal.openSavedDirectory}
        onCopySavedPath={exportModal.copySavedPath}
      />

      {cliLaunchModal && (
        <div className="modal-overlay">
          <div className="modal modal-lg" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('instances.launchDialog.title', '启动实例')}</h2>
              <button
                className="modal-close"
                onClick={() => setCliLaunchModal(null)}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="add-status success">
                <Check size={16} />
                <span>
                  {t('accounts.switched', '已切换至 {{email}}', {
                    email: maskAccountText(cliLaunchModal.accountEmail),
                  })}
                </span>
              </div>
              <div className="form-group">
                <label>{t('instances.columns.instance', '实例')}</label>
                <input
                  className="form-input"
                  value={cliLaunchModal.instanceName}
                  readOnly
                />
              </div>
              <div className="form-group">
                <label>{t('instances.form.workingDir', '工作目录')}</label>
                <div style={{ display: 'flex', gap: 8 }}>
                  <input
                    className="form-input"
                    value={cliLaunchModal.workingDir}
                    placeholder={t('instances.form.workingDirPlaceholder', '默认当前路径')}
                    onChange={(event) => updateCliLaunchWorkingDir(event.target.value)}
                    disabled={cliLaunchModal.preparing || cliLaunchModal.executing}
                  />
                  <button
                    className="btn btn-secondary"
                    type="button"
                    onClick={() => void handleChooseCliWorkingDir()}
                    disabled={cliLaunchModal.preparing || cliLaunchModal.executing}
                    title={t('claude.cli.selectWorkingDir', '选择 Claude CLI 工作目录')}
                    aria-label={t('claude.cli.selectWorkingDir', '选择 Claude CLI 工作目录')}
                  >
                    <FolderOpen size={16} />
                  </button>
                </div>
                <p className="form-hint">
                  {t('instances.form.workingDirDesc', '启动时将首先切换到此目录')}
                </p>
              </div>
              <div className="form-group">
                <label>{t('instances.launchDialog.command', '启动命令')}</label>
                <textarea
                  className="form-input instance-args-input"
                  value={cliLaunchModal.launchCommand}
                  placeholder={t('claude.cli.selectWorkingDir', '选择 Claude CLI 工作目录')}
                  readOnly
                />
                <p className="form-hint">
                  {t(
                    'instances.launchDialog.hint',
                    '可复制命令手动执行，或点击下方按钮直接在终端执行。',
                  )}
                </p>
              </div>
              <div className="form-group">
                <label>{t('instances.launchDialog.terminal', '终端')}</label>
                <SingleSelectDropdown
                  value={selectedTerminal}
                  onChange={setSelectedTerminal}
                  options={terminalOptions}
                  disabled={cliLaunchModal.preparing || cliLaunchModal.executing}
                  ariaLabel={t('instances.launchDialog.terminal', '终端')}
                />
              </div>
              {cliLaunchModal.executeMessage && (
                <div className="add-status success">
                  <Check size={16} />
                  <span>{cliLaunchModal.executeMessage}</span>
                </div>
              )}
              {cliLaunchModal.executeError && (
                <div className="form-error">{cliLaunchModal.executeError}</div>
              )}
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => void handleCopyCliLaunchCommand()}
                disabled={cliLaunchModal.preparing || cliLaunchModal.executing}
              >
                <Copy size={16} />
                {cliLaunchModal.preparing
                  ? t('common.loading', '加载中...')
                  : cliLaunchModal.copied
                    ? t('common.success', '成功')
                    : t('common.copy', '复制')}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleExecuteCliInTerminal()}
                disabled={cliLaunchModal.preparing || cliLaunchModal.executing}
              >
                {cliLaunchModal.preparing || cliLaunchModal.executing
                  ? <RefreshCw size={16} className="loading-spinner" />
                  : <Play size={16} />}
                {cliLaunchModal.preparing || cliLaunchModal.executing
                  ? t('common.loading', '加载中...')
                  : t('instances.launchDialog.runInTerminal', '终端执行')}
              </button>
            </div>
          </div>
        </div>
      )}

      {deleteConfirm && (
        <div className="modal-overlay">
          <div className="modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('common.delete', '删除')}</h2>
              <button className="modal-close" onClick={() => setDeleteConfirm(null)} aria-label={t('common.close', '关闭')}>
                <X />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={deleteError} scrollKey={deleteErrorScrollKey} />
              <p>
                {t('claude.deleteConfirm', '确定删除 Claude 账号 {{email}} 吗？', {
                  email: deleteConfirm.email,
                })}
              </p>
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={() => setDeleteConfirm(null)}>
                {t('common.cancel', '取消')}
              </button>
              <button className="btn btn-danger" onClick={() => void confirmDelete()} disabled={deleting}>
                <Trash2 size={14} />
                {deleting ? t('common.loading', '加载中...') : t('common.delete', '删除')}
              </button>
            </div>
          </div>
        </div>
      )}

      {editingAccountNoteAccount && (
        <div className="modal-overlay">
          <div className="modal codex-account-note-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('claude.accountNote.title', '账号备注')}</h2>
              <button
                className="modal-close"
                onClick={closeAccountNoteModal}
                aria-label={t('common.close', '关闭')}
                disabled={savingAccountNote}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={accountNoteError} scrollKey={accountNoteErrorScrollKey} />
              <p className="codex-account-note-desc">
                {t('claude.accountNote.desc', '为 {{email}} 添加本地备注，方便切换账号时识别。', {
                  email: maskAccountText(getClaudeAccountDisplayEmail(editingAccountNoteAccount)),
                })}
              </p>
              <label className="codex-account-note-field">
                <span>{t('claude.accountNote.label', '账号备注')}</span>
                <textarea
                  className="codex-account-note-textarea"
                  value={editingAccountNoteValue}
                  onChange={(event) => {
                    setEditingAccountNoteValue(event.target.value);
                    setAccountNoteError(null);
                  }}
                  placeholder={t('claude.accountNote.placeholder', '例如：Free 主号、Max 20x、团队账号')}
                  disabled={savingAccountNote}
                  rows={5}
                />
              </label>
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={closeAccountNoteModal} disabled={savingAccountNote}>
                {t('common.cancel', '取消')}
              </button>
              <button className="btn btn-primary" onClick={() => void handleSubmitAccountNote()} disabled={savingAccountNote}>
                {savingAccountNote ? <RefreshCw size={14} className="loading-spinner" /> : <FileText size={14} />}
                {savingAccountNote ? t('common.loading', '加载中...') : t('common.save', '保存')}
              </button>
            </div>
          </div>
        </div>
      )}

      <TagEditModal
        isOpen={Boolean(tagAccount)}
        initialTags={tagAccount?.tags || []}
        availableTags={availableTags}
        onClose={() => setTagAccountId(null)}
        onSave={handleSaveTags}
      />
    </div>
  );
}
