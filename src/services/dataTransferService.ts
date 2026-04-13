import { invoke } from '@tauri-apps/api/core';
import {
  ACCOUNT_TRANSFER_SCHEMA,
  AccountTransferBundle,
  AccountTransferImportProgress,
  AccountTransferImportResult,
  buildAccountTransferBundle,
  importAllAccountsFromTransferJson,
} from './accountTransferService';
import { ALL_PLATFORM_IDS, PlatformId } from '../types/platform';
import { getGroupSettings, GroupSettings, saveGroupSettings } from './groupService';
import {
  AccountGroup,
  getAccountGroups,
  invalidateCache as invalidateAccountGroupCache,
} from './accountGroupService';
import {
  CodexAccountGroup,
  getCodexAccountGroups,
  invalidateCodexGroupCache,
} from './codexAccountGroupService';
import {
  CodexModelProvider,
  invalidateCodexModelProviderCache,
  listCodexModelProviders,
} from './codexModelProviderService';
import {
  getCodexWakeupCliStatus,
  getCodexWakeupState,
  saveCodexWakeupState,
  updateCodexWakeupRuntimeConfig,
} from './codexWakeupService';
import { CodexWakeupModelPreset, CodexWakeupTask } from '../types/codexWakeup';
import {
  CURRENT_ACCOUNT_REFRESH_STORAGE_KEY,
  CurrentAccountRefreshMinutesMap,
  loadCurrentAccountRefreshMinutesMap,
  saveCurrentAccountRefreshMinutesMap,
} from '../utils/currentAccountRefresh';
import {
  DEFAULT_WAKEUP_OFFICIAL_LS_VERSION_MODE,
  WakeupOfficialLsVersionMode,
  WAKEUP_OFFICIAL_LS_VERSION_STORAGE_KEY,
  loadWakeupOfficialLsVersionMode,
  saveWakeupOfficialLsVersionMode,
} from '../utils/wakeupOfficialLsVersion';
import * as accountService from './accountService';
import * as codexService from './codexService';
import * as zedService from './zedService';
import * as githubCopilotService from './githubCopilotService';
import * as windsurfService from './windsurfService';
import * as kiroService from './kiroService';
import * as cursorService from './cursorService';
import * as geminiService from './geminiService';
import * as codebuddyService from './codebuddyService';
import * as codebuddyCnService from './codebuddyCnService';
import * as qoderService from './qoderService';
import * as traeService from './traeService';
import * as workbuddyService from './workbuddyService';
import type { InstanceLaunchMode } from '../types/instance';

const DATA_TRANSFER_SCHEMA = 'cockpit-tools.data-transfer';
const DATA_TRANSFER_VERSION = 1;
const WAKEUP_ENABLED_KEY = 'agtools.wakeup.enabled';
const WAKEUP_TASKS_KEY = 'agtools.wakeup.tasks';

const INSTANCE_PLATFORMS = [
  'antigravity',
  'codex',
  'github-copilot',
  'windsurf',
  'kiro',
  'cursor',
  'gemini',
  'codebuddy',
  'codebuddy_cn',
  'qoder',
  'trae',
  'workbuddy',
] as const;

type InstancePlatform = (typeof INSTANCE_PLATFORMS)[number];
type TransferAccountRecord = Record<string, unknown> & { id: string };
type AccountLoader = () => Promise<TransferAccountRecord[]>;
type LegacyFormat = 'data_bundle' | 'account_bundle' | 'legacy_account_json';
type DataTransferWarningCode = 'accounts_section_missing' | 'config_section_missing';

interface RawUserConfig extends Record<string, unknown> {
  auto_switch_selected_account_ids?: string[];
  codex_auto_switch_selected_account_ids?: string[];
}

interface ExportedUserConfig extends Omit<
  RawUserConfig,
  'auto_switch_selected_account_ids' | 'codex_auto_switch_selected_account_ids'
> {
  auto_switch_selected_account_refs: DataTransferAccountRef[];
  codex_auto_switch_selected_account_refs: DataTransferAccountRef[];
}

interface RawInstanceProfile {
  id: string;
  name: string;
  userDataDir: string;
  workingDir?: string | null;
  extraArgs: string;
  bindAccountId?: string | null;
  launchMode?: InstanceLaunchMode;
  createdAt: number;
  lastLaunchedAt?: number | null;
  lastPid?: number | null;
}

interface RawDefaultInstanceSettings {
  bindAccountId?: string | null;
  extraArgs: string;
  launchMode?: InstanceLaunchMode;
  followLocalAccount?: boolean;
  lastPid?: number | null;
}

interface RawInstanceStore {
  instances: RawInstanceProfile[];
  defaultSettings: RawDefaultInstanceSettings;
}

interface ExportedInstanceProfile {
  id: string;
  name: string;
  userDataDir: string;
  workingDir?: string | null;
  extraArgs: string;
  bindAccountRef: DataTransferAccountRef | null;
  launchMode?: InstanceLaunchMode;
  createdAt: number;
}

interface ExportedDefaultInstanceSettings {
  bindAccountRef: DataTransferAccountRef | null;
  extraArgs: string;
  launchMode?: InstanceLaunchMode;
  followLocalAccount: boolean;
}

interface ExportedInstanceStore {
  defaultSettings: ExportedDefaultInstanceSettings;
  instances: ExportedInstanceProfile[];
}

type GenericRecord = Record<string, unknown>;
type WakeupTaskRecord = GenericRecord & { enabled?: boolean; schedule?: GenericRecord };

interface ExportedAntigravityWakeupState {
  enabled: boolean;
  official_ls_version_mode: WakeupOfficialLsVersionMode;
  tasks: WakeupTaskRecord[];
}

interface ExportedCodexWakeupTask extends Omit<CodexWakeupTask, 'account_ids'> {
  account_refs: DataTransferAccountRef[];
}

interface ExportedCodexWakeupState {
  enabled: boolean;
  tasks: ExportedCodexWakeupTask[];
  model_presets: CodexWakeupModelPreset[];
  runtime: {
    codex_cli_path?: string;
    node_path?: string;
  };
}

interface ExportedAccountGroup extends Omit<AccountGroup, 'accountIds'> {
  accountRefs: DataTransferAccountRef[];
}

interface ExportedCodexAccountGroup extends Omit<CodexAccountGroup, 'accountIds'> {
  accountRefs: DataTransferAccountRef[];
}

export interface DataTransferAccountRef {
  platform: PlatformId;
  email?: string;
  userId?: string;
  accountId?: string;
  authId?: string;
  githubLogin?: string;
  githubId?: number;
  uid?: string;
  organizationId?: string;
  loginProvider?: string;
  apiBaseUrl?: string;
  apiProviderId?: string;
  apiProviderName?: string;
  domain?: string;
}

export interface DataTransferSelection {
  includeAccounts: boolean;
  includeConfig: boolean;
}

export interface DataTransferConfigBundle {
  user_config: ExportedUserConfig;
  group_settings: GroupSettings;
  account_groups: ExportedAccountGroup[];
  codex_account_groups: ExportedCodexAccountGroup[];
  codex_model_providers: CodexModelProvider[];
  instance_stores: Partial<Record<InstancePlatform, ExportedInstanceStore>>;
  antigravity_wakeup: ExportedAntigravityWakeupState;
  codex_wakeup: ExportedCodexWakeupState;
  current_account_refresh_minutes: CurrentAccountRefreshMinutesMap;
}

export interface DataTransferBundle {
  schema: typeof DATA_TRANSFER_SCHEMA;
  version: typeof DATA_TRANSFER_VERSION;
  exported_at: string;
  sections: {
    accounts: boolean;
    config: boolean;
  };
  accounts?: AccountTransferBundle;
  config?: DataTransferConfigBundle;
}

export interface DataTransferConfigImportResult {
  applied: boolean;
  unresolved_account_ref_count: number;
  disabled_task_count: number;
  needs_restart: boolean;
}

export interface DataTransferImportResult {
  detected_format: LegacyFormat;
  legacy_account_platform?: PlatformId | null;
  imported_account_count: number;
  account_result: AccountTransferImportResult | null;
  config_result: DataTransferConfigImportResult | null;
  warnings: DataTransferWarningCode[];
}

export interface DataTransferImportOptions extends DataTransferSelection {
  onAccountProgress?: (progress: AccountTransferImportProgress) => void;
}

interface AccountRegistry {
  byPlatform: Record<PlatformId, TransferAccountRecord[]>;
  byId: Record<PlatformId, Map<string, TransferAccountRecord>>;
}

const ACCOUNT_LOADERS: Record<PlatformId, AccountLoader> = {
  antigravity: async () => (await accountService.listAccounts()) as unknown as TransferAccountRecord[],
  codex: async () => (await codexService.listCodexAccounts()) as unknown as TransferAccountRecord[],
  zed: async () => (await zedService.listZedAccounts()) as unknown as TransferAccountRecord[],
  'github-copilot': async () =>
    (await githubCopilotService.listGitHubCopilotAccounts()) as unknown as TransferAccountRecord[],
  windsurf: async () => (await windsurfService.listWindsurfAccounts()) as unknown as TransferAccountRecord[],
  kiro: async () => (await kiroService.listKiroAccounts()) as unknown as TransferAccountRecord[],
  cursor: async () => (await cursorService.listCursorAccounts()) as unknown as TransferAccountRecord[],
  gemini: async () => (await geminiService.listGeminiAccounts()) as unknown as TransferAccountRecord[],
  codebuddy: async () => (await codebuddyService.listCodebuddyAccounts()) as unknown as TransferAccountRecord[],
  codebuddy_cn: async () =>
    (await codebuddyCnService.listCodebuddyCnAccounts()) as unknown as TransferAccountRecord[],
  qoder: async () => (await qoderService.listQoderAccounts()) as unknown as TransferAccountRecord[],
  trae: async () => (await traeService.listTraeAccounts()) as unknown as TransferAccountRecord[],
  workbuddy: async () => (await workbuddyService.listWorkbuddyAccounts()) as unknown as TransferAccountRecord[],
};

const LEGACY_IMPORTERS: Record<PlatformId, ((jsonContent: string) => Promise<unknown[]>) | undefined> = {
  antigravity: accountService.importFromJson,
  codex: codexService.importCodexFromJson,
  zed: zedService.importZedFromJson,
  'github-copilot': githubCopilotService.importGitHubCopilotFromJson,
  windsurf: windsurfService.importWindsurfFromJson,
  kiro: kiroService.importKiroFromJson,
  cursor: cursorService.importCursorFromJson,
  gemini: geminiService.importGeminiFromJson,
  codebuddy: codebuddyService.importCodebuddyFromJson,
  codebuddy_cn: codebuddyCnService.importCodebuddyCnFromJson,
  qoder: qoderService.importQoderFromJson,
  trae: traeService.importTraeFromJson,
  workbuddy: workbuddyService.importWorkbuddyFromJson,
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function normalizeString(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
}

function normalizeNumber(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value;
  }
  if (typeof value === 'string') {
    const parsed = Number(value.trim());
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function normalizeBoolean(value: unknown): boolean | null {
  if (typeof value === 'boolean') return value;
  return null;
}

function stringEquals(left: unknown, right: unknown): boolean {
  const normalizedLeft = normalizeString(left)?.toLowerCase();
  const normalizedRight = normalizeString(right)?.toLowerCase();
  return Boolean(normalizedLeft && normalizedRight && normalizedLeft === normalizedRight);
}

function stringContains(value: unknown, keyword: string): boolean {
  const normalized = normalizeString(value)?.toLowerCase();
  return Boolean(normalized && normalized.includes(keyword.toLowerCase()));
}

function parseJsonOrThrow(jsonContent: string, errorCode: string): unknown {
  try {
    return JSON.parse(jsonContent) as unknown;
  } catch {
    throw new Error(errorCode);
  }
}

function ensureSelection(selection: DataTransferSelection): void {
  if (!selection.includeAccounts && !selection.includeConfig) {
    throw new Error('transfer_selection_required');
  }
}

function firstLegacySample(value: unknown): Record<string, unknown> | null {
  if (Array.isArray(value)) {
    return value.find((item) => isRecord(item)) ?? null;
  }
  return isRecord(value) ? value : null;
}

function isDataTransferBundle(value: unknown): value is DataTransferBundle {
  return isRecord(value) && value.schema === DATA_TRANSFER_SCHEMA;
}

function isAccountTransferBundleLike(value: unknown): boolean {
  return isRecord(value) && value.schema === ACCOUNT_TRANSFER_SCHEMA;
}

function buildAccountRegistry(
  entries: Array<readonly [PlatformId, TransferAccountRecord[]]>,
): AccountRegistry {
  const byPlatform = {} as Record<PlatformId, TransferAccountRecord[]>;
  const byId = {} as Record<PlatformId, Map<string, TransferAccountRecord>>;

  for (const platform of ALL_PLATFORM_IDS) {
    byPlatform[platform] = [];
    byId[platform] = new Map<string, TransferAccountRecord>();
  }

  for (const [platform, accounts] of entries) {
    byPlatform[platform] = accounts;
    byId[platform] = new Map(accounts.map((account) => [String(account.id), account]));
  }

  return {
    byPlatform,
    byId,
  };
}

async function loadAccountRegistry(): Promise<AccountRegistry> {
  const entries = await Promise.all(
    ALL_PLATFORM_IDS.map(async (platform) => {
      const accounts = await ACCOUNT_LOADERS[platform]();
      return [platform, accounts] as const;
    }),
  );

  return buildAccountRegistry(entries);
}

function buildAccountRef(platform: PlatformId, account: TransferAccountRecord): DataTransferAccountRef | null {
  const ref: DataTransferAccountRef = { platform };

  switch (platform) {
    case 'antigravity':
      ref.email = normalizeString(account.email) ?? undefined;
      break;
    case 'codex':
      ref.email = normalizeString(account.email) ?? undefined;
      ref.userId = normalizeString(account.user_id) ?? undefined;
      ref.accountId = normalizeString(account.account_id) ?? undefined;
      ref.organizationId = normalizeString(account.organization_id) ?? undefined;
      ref.apiBaseUrl = normalizeString(account.api_base_url) ?? undefined;
      ref.apiProviderId = normalizeString(account.api_provider_id) ?? undefined;
      ref.apiProviderName = normalizeString(account.api_provider_name) ?? undefined;
      break;
    case 'zed':
      ref.email = normalizeString(account.email) ?? undefined;
      ref.userId = normalizeString(account.user_id) ?? undefined;
      ref.githubLogin = normalizeString(account.github_login) ?? undefined;
      break;
    case 'github-copilot':
    case 'windsurf':
      ref.email = normalizeString(account.github_email) ?? normalizeString(account.email) ?? undefined;
      ref.githubLogin = normalizeString(account.github_login) ?? undefined;
      ref.githubId = normalizeNumber(account.github_id) ?? undefined;
      break;
    case 'kiro':
      ref.email = normalizeString(account.email) ?? undefined;
      ref.userId = normalizeString(account.user_id) ?? undefined;
      ref.loginProvider = normalizeString(account.login_provider) ?? undefined;
      break;
    case 'cursor':
    case 'gemini':
      ref.email = normalizeString(account.email) ?? undefined;
      ref.authId = normalizeString(account.auth_id) ?? undefined;
      break;
    case 'qoder':
    case 'trae':
      ref.email = normalizeString(account.email) ?? undefined;
      ref.userId = normalizeString(account.user_id) ?? undefined;
      break;
    case 'codebuddy':
    case 'codebuddy_cn':
    case 'workbuddy':
      ref.email = normalizeString(account.email) ?? undefined;
      ref.uid = normalizeString(account.uid) ?? undefined;
      ref.domain = normalizeString(account.domain) ?? undefined;
      break;
  }

  const hintKeys = Object.keys(ref).filter((key) => key !== 'platform');
  return hintKeys.length > 0 ? ref : null;
}

function scoreAccountRef(ref: DataTransferAccountRef, account: TransferAccountRecord): number {
  let score = 0;

  const addStringScore = (expected: string | undefined, actual: unknown, weight: number) => {
    if (!expected) return;
    if (stringEquals(expected, actual)) {
      score += weight;
    }
  };

  const addNumberScore = (expected: number | undefined, actual: unknown, weight: number) => {
    if (expected == null) return;
    const normalized = normalizeNumber(actual);
    if (normalized != null && normalized === expected) {
      score += weight;
    }
  };

  switch (ref.platform) {
    case 'antigravity':
      addStringScore(ref.email, account.email, 10);
      break;
    case 'codex':
      addStringScore(ref.accountId, account.account_id, 24);
      addStringScore(ref.userId, account.user_id, 20);
      addStringScore(ref.organizationId, account.organization_id, 12);
      addStringScore(ref.email, account.email, 10);
      addStringScore(ref.apiProviderId, account.api_provider_id, 8);
      addStringScore(ref.apiBaseUrl, account.api_base_url, 6);
      break;
    case 'zed':
      addStringScore(ref.userId, account.user_id, 24);
      addStringScore(ref.githubLogin, account.github_login, 14);
      addStringScore(ref.email, account.email, 8);
      break;
    case 'github-copilot':
    case 'windsurf':
      addNumberScore(ref.githubId, account.github_id, 24);
      addStringScore(ref.githubLogin, account.github_login, 14);
      addStringScore(ref.email, account.github_email ?? account.email, 8);
      break;
    case 'kiro':
      addStringScore(ref.userId, account.user_id, 24);
      addStringScore(ref.email, account.email, 10);
      addStringScore(ref.loginProvider, account.login_provider, 4);
      break;
    case 'cursor':
    case 'gemini':
      addStringScore(ref.authId, account.auth_id, 24);
      addStringScore(ref.email, account.email, 10);
      break;
    case 'qoder':
    case 'trae':
      addStringScore(ref.userId, account.user_id, 24);
      addStringScore(ref.email, account.email, 10);
      break;
    case 'codebuddy':
    case 'codebuddy_cn':
    case 'workbuddy':
      addStringScore(ref.uid, account.uid, 24);
      addStringScore(ref.email, account.email, 10);
      addStringScore(ref.domain, account.domain, 4);
      break;
  }

  return score;
}

function resolveAccountRef(
  ref: DataTransferAccountRef | null | undefined,
  registry: AccountRegistry,
): string | null {
  if (!ref) return null;
  const candidates = registry.byPlatform[ref.platform] ?? [];
  let bestScore = 0;
  let bestId: string | null = null;
  let tied = false;

  for (const candidate of candidates) {
    const score = scoreAccountRef(ref, candidate);
    if (score <= 0) continue;
    if (score > bestScore) {
      bestScore = score;
      bestId = String(candidate.id);
      tied = false;
      continue;
    }
    if (score === bestScore && bestId !== String(candidate.id)) {
      tied = true;
    }
  }

  if (bestScore <= 0 || tied) {
    return null;
  }

  return bestId;
}

function mapAccountIdsToRefs(
  platform: PlatformId,
  accountIds: string[] | undefined,
  registry: AccountRegistry,
): DataTransferAccountRef[] {
  if (!Array.isArray(accountIds)) {
    return [];
  }

  const result: DataTransferAccountRef[] = [];
  const seen = new Set<string>();
  const accountMap = registry.byId[platform];

  for (const rawId of accountIds) {
    const accountId = normalizeString(rawId);
    if (!accountId || seen.has(accountId)) continue;
    seen.add(accountId);
    const account = accountMap.get(accountId);
    if (!account) continue;
    const ref = buildAccountRef(platform, account);
    if (ref) {
      result.push(ref);
    }
  }

  return result;
}

function resolveAccountRefsToIds(
  refs: DataTransferAccountRef[] | undefined,
  registry: AccountRegistry,
): { ids: string[]; unresolved: number } {
  if (!Array.isArray(refs)) {
    return { ids: [], unresolved: 0 };
  }

  const ids: string[] = [];
  const seen = new Set<string>();
  let unresolved = 0;

  for (const ref of refs) {
    const id = resolveAccountRef(ref, registry);
    if (!id) {
      unresolved += 1;
      continue;
    }
    if (seen.has(id)) continue;
    seen.add(id);
    ids.push(id);
  }

  return { ids, unresolved };
}

function exportUserConfig(config: RawUserConfig, registry: AccountRegistry): ExportedUserConfig {
  const { auto_switch_selected_account_ids, codex_auto_switch_selected_account_ids, ...rest } = config;

  return {
    ...rest,
    auto_switch_selected_account_refs: mapAccountIdsToRefs(
      'antigravity',
      auto_switch_selected_account_ids,
      registry,
    ),
    codex_auto_switch_selected_account_refs: mapAccountIdsToRefs(
      'codex',
      codex_auto_switch_selected_account_ids,
      registry,
    ),
  };
}

function importUserConfig(
  config: ExportedUserConfig,
  registry: AccountRegistry,
): { config: RawUserConfig; unresolved: number } {
  const {
    auto_switch_selected_account_refs,
    codex_auto_switch_selected_account_refs,
    ...rest
  } = config;

  const antigravityResolved = resolveAccountRefsToIds(auto_switch_selected_account_refs, registry);
  const codexResolved = resolveAccountRefsToIds(codex_auto_switch_selected_account_refs, registry);

  return {
    config: {
      ...rest,
      auto_switch_selected_account_ids: antigravityResolved.ids,
      codex_auto_switch_selected_account_ids: codexResolved.ids,
    },
    unresolved: antigravityResolved.unresolved + codexResolved.unresolved,
  };
}

function exportAccountGroups(groups: AccountGroup[], registry: AccountRegistry): ExportedAccountGroup[] {
  return groups.map((group) => ({
    id: group.id,
    name: group.name,
    createdAt: group.createdAt,
    accountRefs: mapAccountIdsToRefs('antigravity', group.accountIds, registry),
  }));
}

function importAccountGroups(
  groups: ExportedAccountGroup[],
  registry: AccountRegistry,
): { groups: AccountGroup[]; unresolved: number } {
  let unresolved = 0;
  const restored = groups.map((group) => {
    const resolved = resolveAccountRefsToIds(group.accountRefs, registry);
    unresolved += resolved.unresolved;
    return {
      id: group.id,
      name: group.name,
      createdAt: group.createdAt,
      accountIds: resolved.ids,
    };
  });

  return { groups: restored, unresolved };
}

function exportCodexAccountGroups(
  groups: CodexAccountGroup[],
  registry: AccountRegistry,
): ExportedCodexAccountGroup[] {
  return groups.map((group) => ({
    id: group.id,
    name: group.name,
    sortOrder: group.sortOrder,
    createdAt: group.createdAt,
    accountRefs: mapAccountIdsToRefs('codex', group.accountIds, registry),
  }));
}

function importCodexAccountGroups(
  groups: ExportedCodexAccountGroup[],
  registry: AccountRegistry,
): { groups: CodexAccountGroup[]; unresolved: number } {
  let unresolved = 0;
  const restored = groups.map((group) => {
    const resolved = resolveAccountRefsToIds(group.accountRefs, registry);
    unresolved += resolved.unresolved;
    return {
      id: group.id,
      name: group.name,
      sortOrder: group.sortOrder,
      createdAt: group.createdAt,
      accountIds: resolved.ids,
    };
  });

  return { groups: restored, unresolved };
}

function exportInstanceStore(
  platform: InstancePlatform,
  store: RawInstanceStore,
  registry: AccountRegistry,
): ExportedInstanceStore {
  return {
    defaultSettings: {
      bindAccountRef:
        store.defaultSettings.bindAccountId != null
          ? mapAccountIdsToRefs(platform, [store.defaultSettings.bindAccountId], registry)[0] ?? null
          : null,
      extraArgs: normalizeString(store.defaultSettings.extraArgs) ?? '',
      launchMode: store.defaultSettings.launchMode,
      followLocalAccount: normalizeBoolean(store.defaultSettings.followLocalAccount) ?? false,
    },
    instances: Array.isArray(store.instances)
      ? store.instances.map((instance) => ({
          id: instance.id,
          name: instance.name,
          userDataDir: instance.userDataDir,
          workingDir: instance.workingDir ?? null,
          extraArgs: normalizeString(instance.extraArgs) ?? '',
          bindAccountRef:
            instance.bindAccountId != null
              ? mapAccountIdsToRefs(platform, [instance.bindAccountId], registry)[0] ?? null
              : null,
          launchMode: instance.launchMode,
          createdAt: instance.createdAt,
        }))
      : [],
  };
}

function importInstanceStore(
  _platform: InstancePlatform,
  store: ExportedInstanceStore,
  registry: AccountRegistry,
): { store: RawInstanceStore; unresolved: number } {
  let unresolved = 0;

  const defaultResolved = resolveAccountRef(store.defaultSettings.bindAccountRef, registry);
  if (store.defaultSettings.bindAccountRef && !defaultResolved) {
    unresolved += 1;
  }

  const restoredInstances = Array.isArray(store.instances)
    ? store.instances.map((instance) => {
        const resolvedId = resolveAccountRef(instance.bindAccountRef, registry);
        if (instance.bindAccountRef && !resolvedId) {
          unresolved += 1;
        }
        return {
          id: instance.id,
          name: instance.name,
          userDataDir: instance.userDataDir,
          workingDir: instance.workingDir ?? null,
          extraArgs: normalizeString(instance.extraArgs) ?? '',
          bindAccountId: resolvedId,
          launchMode: instance.launchMode,
          createdAt: instance.createdAt,
          lastLaunchedAt: null,
          lastPid: null,
        } as RawInstanceProfile;
      })
    : [];

  return {
    store: {
      defaultSettings: {
        bindAccountId: defaultResolved,
        extraArgs: normalizeString(store.defaultSettings.extraArgs) ?? '',
        launchMode: store.defaultSettings.launchMode,
        followLocalAccount: normalizeBoolean(store.defaultSettings.followLocalAccount) ?? false,
        lastPid: null,
      },
      instances: restoredInstances,
    },
    unresolved,
  };
}

function loadAntigravityWakeupState(): ExportedAntigravityWakeupState {
  const enabled = localStorage.getItem(WAKEUP_ENABLED_KEY) === 'true';
  const tasksRaw = localStorage.getItem(WAKEUP_TASKS_KEY);
  let tasks: WakeupTaskRecord[] = [];

  if (tasksRaw) {
    try {
      const parsed = JSON.parse(tasksRaw) as unknown;
      tasks = Array.isArray(parsed) ? (parsed.filter((item) => isRecord(item)) as WakeupTaskRecord[]) : [];
    } catch {
      tasks = [];
    }
  }

  return {
    enabled,
    tasks,
    official_ls_version_mode: loadWakeupOfficialLsVersionMode(),
  };
}

function exportAntigravityWakeupState(
  registry: AccountRegistry,
): ExportedAntigravityWakeupState {
  const rawState = loadAntigravityWakeupState();
  return {
    enabled: rawState.enabled,
    official_ls_version_mode: rawState.official_ls_version_mode,
    tasks: rawState.tasks.map((task) => {
      const nextTask: WakeupTaskRecord = { ...task };
      const rawSchedule = isRecord(task.schedule) ? { ...task.schedule } : {};
      const refs = mapAccountIdsToRefs(
        'antigravity',
        Array.isArray(rawSchedule.selectedAccounts)
          ? (rawSchedule.selectedAccounts as string[])
          : undefined,
        registry,
      );
      delete rawSchedule.selectedAccounts;
      rawSchedule.selectedAccountRefs = refs;
      nextTask.schedule = rawSchedule;
      return nextTask;
    }),
  };
}

function importAntigravityWakeupState(
  state: ExportedAntigravityWakeupState,
  registry: AccountRegistry,
): { state: ExportedAntigravityWakeupState; unresolved: number; disabledTasks: number } {
  let unresolved = 0;
  let disabledTasks = 0;

  const tasks = state.tasks.map((task) => {
    const nextTask: WakeupTaskRecord = { ...task };
    const rawSchedule = isRecord(task.schedule) ? { ...task.schedule } : {};
    const resolved = resolveAccountRefsToIds(
      Array.isArray(rawSchedule.selectedAccountRefs)
        ? (rawSchedule.selectedAccountRefs as DataTransferAccountRef[])
        : undefined,
      registry,
    );
    unresolved += resolved.unresolved;
    delete rawSchedule.selectedAccountRefs;
    rawSchedule.selectedAccounts = resolved.ids;
    nextTask.schedule = rawSchedule;
    if (resolved.ids.length === 0 && normalizeBoolean(task.enabled) === true) {
      nextTask.enabled = false;
      disabledTasks += 1;
    }
    return nextTask;
  });

  return {
    state: {
      enabled: state.enabled,
      official_ls_version_mode:
        state.official_ls_version_mode ?? DEFAULT_WAKEUP_OFFICIAL_LS_VERSION_MODE,
      tasks,
    },
    unresolved,
    disabledTasks,
  };
}

function exportCodexWakeupState(
  state: Awaited<ReturnType<typeof getCodexWakeupState>>,
  registry: AccountRegistry,
  runtime: { codex_cli_path?: string; node_path?: string },
): ExportedCodexWakeupState {
  return {
    enabled: state.enabled,
    model_presets: state.model_presets,
    runtime,
    tasks: state.tasks.map((task) => ({
      ...task,
      account_refs: mapAccountIdsToRefs('codex', task.account_ids, registry),
    })),
  };
}

function importCodexWakeupState(
  state: ExportedCodexWakeupState,
  registry: AccountRegistry,
): {
  state: {
    enabled: boolean;
    tasks: CodexWakeupTask[];
    model_presets: CodexWakeupModelPreset[];
    runtime: { codex_cli_path?: string; node_path?: string };
  };
  unresolved: number;
  disabledTasks: number;
} {
  let unresolved = 0;
  let disabledTasks = 0;

  const tasks = state.tasks.map((task) => {
    const resolved = resolveAccountRefsToIds(task.account_refs, registry);
    unresolved += resolved.unresolved;
    const restoredTask: CodexWakeupTask = {
      ...task,
      account_ids: resolved.ids,
    };
    delete (restoredTask as unknown as Record<string, unknown>).account_refs;
    if (resolved.ids.length === 0 && restoredTask.enabled) {
      restoredTask.enabled = false;
      disabledTasks += 1;
    }
    return restoredTask;
  });

  return {
    state: {
      enabled: state.enabled,
      tasks,
      model_presets: state.model_presets,
      runtime: state.runtime ?? {},
    },
    unresolved,
    disabledTasks,
  };
}

async function exportConfigBundle(registry: AccountRegistry): Promise<DataTransferConfigBundle> {
  const [
    rawUserConfig,
    groupSettings,
    accountGroups,
    codexAccountGroups,
    codexModelProviders,
    codexWakeupState,
    codexWakeupCliStatus,
    instanceStoreEntries,
  ] = await Promise.all([
    invoke<RawUserConfig>('data_transfer_get_user_config'),
    getGroupSettings(),
    getAccountGroups(),
    getCodexAccountGroups(),
    listCodexModelProviders(),
    getCodexWakeupState(),
    getCodexWakeupCliStatus(),
    Promise.all(
      INSTANCE_PLATFORMS.map(async (platform) => {
        const store = await invoke<RawInstanceStore>('data_transfer_get_instance_store', { platform });
        return [platform, exportInstanceStore(platform, store, registry)] as const;
      }),
    ),
  ]);

  return {
    user_config: exportUserConfig(rawUserConfig, registry),
    group_settings: groupSettings,
    account_groups: exportAccountGroups(accountGroups, registry),
    codex_account_groups: exportCodexAccountGroups(codexAccountGroups, registry),
    codex_model_providers: codexModelProviders,
    instance_stores: Object.fromEntries(instanceStoreEntries) as Partial<
      Record<InstancePlatform, ExportedInstanceStore>
    >,
    antigravity_wakeup: exportAntigravityWakeupState(registry),
    codex_wakeup: exportCodexWakeupState(
      codexWakeupState,
      registry,
      {
        codex_cli_path: normalizeString(codexWakeupCliStatus.configured_codex_cli_path) ?? undefined,
        node_path: normalizeString(codexWakeupCliStatus.configured_node_path) ?? undefined,
      },
    ),
    current_account_refresh_minutes: loadCurrentAccountRefreshMinutesMap(),
  };
}

async function importConfigBundle(bundle: DataTransferConfigBundle): Promise<DataTransferConfigImportResult> {
  const registry = await loadAccountRegistry();
  let unresolvedAccountRefs = 0;
  let disabledTaskCount = 0;

  const userConfigImport = importUserConfig(bundle.user_config, registry);
  unresolvedAccountRefs += userConfigImport.unresolved;

  const needsRestart = await invoke<boolean>('data_transfer_apply_user_config', {
    config: userConfigImport.config,
  });

  await saveGroupSettings(
    bundle.group_settings.groupMappings,
    bundle.group_settings.groupNames,
    bundle.group_settings.groupOrder,
  );

  const accountGroupsImport = importAccountGroups(bundle.account_groups, registry);
  unresolvedAccountRefs += accountGroupsImport.unresolved;
  await invoke('save_account_groups', {
    data: JSON.stringify(accountGroupsImport.groups, null, 2),
  });
  invalidateAccountGroupCache();

  const codexAccountGroupsImport = importCodexAccountGroups(bundle.codex_account_groups, registry);
  unresolvedAccountRefs += codexAccountGroupsImport.unresolved;
  await invoke('save_codex_account_groups', {
    data: JSON.stringify(codexAccountGroupsImport.groups, null, 2),
  });
  invalidateCodexGroupCache();

  await invoke('save_codex_model_providers', {
    data: JSON.stringify(bundle.codex_model_providers, null, 2),
  });
  invalidateCodexModelProviderCache();

  for (const platform of INSTANCE_PLATFORMS) {
    const store = bundle.instance_stores[platform];
    if (!store) continue;
    const imported = importInstanceStore(platform, store, registry);
    unresolvedAccountRefs += imported.unresolved;
    await invoke('data_transfer_replace_instance_store', {
      platform,
      store: imported.store,
    });
  }

  const antigravityWakeupImport = importAntigravityWakeupState(bundle.antigravity_wakeup, registry);
  unresolvedAccountRefs += antigravityWakeupImport.unresolved;
  disabledTaskCount += antigravityWakeupImport.disabledTasks;
  localStorage.setItem(WAKEUP_ENABLED_KEY, antigravityWakeupImport.state.enabled ? 'true' : 'false');
  localStorage.setItem(WAKEUP_TASKS_KEY, JSON.stringify(antigravityWakeupImport.state.tasks));
  localStorage.setItem(
    WAKEUP_OFFICIAL_LS_VERSION_STORAGE_KEY,
    antigravityWakeupImport.state.official_ls_version_mode,
  );
  saveWakeupOfficialLsVersionMode(antigravityWakeupImport.state.official_ls_version_mode);
  await invoke('wakeup_sync_state', {
    enabled: antigravityWakeupImport.state.enabled,
    tasks: antigravityWakeupImport.state.tasks,
    officialLsVersionMode: antigravityWakeupImport.state.official_ls_version_mode,
    runStartupTasks: false,
  });

  const codexWakeupImport = importCodexWakeupState(bundle.codex_wakeup, registry);
  unresolvedAccountRefs += codexWakeupImport.unresolved;
  disabledTaskCount += codexWakeupImport.disabledTasks;
  await saveCodexWakeupState(
    codexWakeupImport.state.enabled,
    codexWakeupImport.state.tasks,
    codexWakeupImport.state.model_presets,
  );
  await updateCodexWakeupRuntimeConfig(
    normalizeString(codexWakeupImport.state.runtime.codex_cli_path) ?? undefined,
    normalizeString(codexWakeupImport.state.runtime.node_path) ?? undefined,
  );

  saveCurrentAccountRefreshMinutesMap(bundle.current_account_refresh_minutes);
  window.dispatchEvent(new Event('config-updated'));
  window.dispatchEvent(new Event('wakeup-tasks-updated'));

  return {
    applied: true,
    unresolved_account_ref_count: unresolvedAccountRefs,
    disabled_task_count: disabledTaskCount,
    needs_restart: needsRestart,
  };
}

function synthesizeAccountImportResult(
  platform: PlatformId,
  importedCount: number,
): AccountTransferImportResult {
  return {
    imported_count: importedCount,
    platform_success_count: importedCount > 0 ? 1 : 0,
    platform_failed_count: importedCount > 0 ? 0 : 1,
    platform_skipped_count: 0,
    details: [
      {
        platform,
        imported_count: importedCount,
        skipped: false,
      },
    ],
  };
}

function detectLegacyPlatform(value: unknown): PlatformId | null {
  const sample = firstLegacySample(value);
  if (!sample) return null;

  const id = normalizeString(sample.id);
  if (id?.startsWith('codebuddy_cn_')) return 'codebuddy_cn';
  if (id?.startsWith('workbuddy_')) return 'workbuddy';
  if (id?.startsWith('codebuddy_')) return 'codebuddy';

  if ('tokens' in sample || 'OPENAI_API_KEY' in sample || 'auth_mode' in sample || 'authMode' in sample) {
    return 'codex';
  }
  if ('windsurf_api_key' in sample || 'windsurf_auth_token' in sample || 'windsurf_plan_status' in sample) {
    return 'windsurf';
  }
  if ('copilot_token' in sample) {
    return 'github-copilot';
  }
  if ('zed' in sample) {
    return 'zed';
  }
  if ('user_raw' in sample || 'subscription_raw' in sample || 'plan_raw' in sample) {
    return 'zed';
  }
  if ('kiro_auth_token_raw' in sample || 'kiro_usage_raw' in sample || 'login_provider' in sample) {
    return 'kiro';
  }
  if ('gemini_auth_raw' in sample || 'gemini_usage_raw' in sample || 'selected_auth_type' in sample) {
    return 'gemini';
  }
  if ('cursor_auth_raw' in sample || 'cursor_usage_raw' in sample || 'membership_type' in sample) {
    return 'cursor';
  }
  if ('trae_auth_raw' in sample || 'trae_profile_raw' in sample || 'trae_server_raw' in sample) {
    return 'trae';
  }
  if ('auth_user_info_raw' in sample || 'auth_credit_usage_raw' in sample || 'credits_usage_percent' in sample) {
    return 'qoder';
  }
  if ('uid' in sample || 'enterprise_id' in sample || 'dosage_notify_code' in sample) {
    if (stringContains(sample.domain, 'workbuddy')) return 'workbuddy';
    if (stringContains(sample.domain, 'codebuddy.cn')) return 'codebuddy_cn';
    if (stringContains(sample.domain, 'codebuddy')) return 'codebuddy';
    return id?.startsWith('workbuddy_')
      ? 'workbuddy'
      : id?.startsWith('codebuddy_cn_')
        ? 'codebuddy_cn'
        : 'codebuddy';
  }
  if ('github_login' in sample && 'user_id' in sample && ('plan_raw' in sample || 'usage_raw' in sample)) {
    return 'zed';
  }
  if ('github_login' in sample || 'github_id' in sample) {
    return 'github-copilot';
  }
  if ('token' in sample || ('refresh_token' in sample && 'email' in sample)) {
    return 'antigravity';
  }

  return null;
}

async function importLegacyAccountJson(
  platform: PlatformId,
  jsonContent: string,
): Promise<AccountTransferImportResult> {
  const importer = LEGACY_IMPORTERS[platform];
  if (!importer) {
    throw new Error('unsupported_legacy_account_json');
  }
  const imported = await importer(jsonContent);
  const importedCount = Array.isArray(imported) ? imported.length : 0;
  return synthesizeAccountImportResult(platform, importedCount);
}

export function getDataTransferFileNameBase(selection: DataTransferSelection): string {
  if (selection.includeAccounts && selection.includeConfig) {
    return 'cockpit_data_backup';
  }
  if (selection.includeAccounts) {
    return 'cockpit_accounts_backup';
  }
  return 'cockpit_config_backup';
}

export async function exportDataTransferJson(selection: DataTransferSelection): Promise<string> {
  ensureSelection(selection);
  const bundle: DataTransferBundle = {
    schema: DATA_TRANSFER_SCHEMA,
    version: DATA_TRANSFER_VERSION,
    exported_at: new Date().toISOString(),
    sections: {
      accounts: selection.includeAccounts,
      config: selection.includeConfig,
    },
  };

  if (selection.includeAccounts) {
    bundle.accounts = await buildAccountTransferBundle();
  }

  if (selection.includeConfig) {
    const registry = await loadAccountRegistry();
    bundle.config = await exportConfigBundle(registry);
  }

  return JSON.stringify(bundle, null, 2);
}

export async function importDataTransferJson(
  jsonContent: string,
  options: DataTransferImportOptions,
): Promise<DataTransferImportResult> {
  ensureSelection(options);
  const parsed = parseJsonOrThrow(jsonContent, 'invalid_json');

  if (isDataTransferBundle(parsed)) {
    if (parsed.version !== DATA_TRANSFER_VERSION) {
      throw new Error('invalid_bundle_version');
    }

    const warnings: DataTransferWarningCode[] = [];
    let accountResult: AccountTransferImportResult | null = null;
    let configResult: DataTransferConfigImportResult | null = null;

    if (options.includeAccounts) {
      if (parsed.accounts) {
        accountResult = await importAllAccountsFromTransferJson(JSON.stringify(parsed.accounts), {
          onProgress: options.onAccountProgress,
        });
      } else {
        warnings.push('accounts_section_missing');
      }
    }

    if (options.includeConfig) {
      if (parsed.config) {
        configResult = await importConfigBundle(parsed.config);
      } else {
        warnings.push('config_section_missing');
      }
    }

    if (!accountResult && !configResult) {
      throw new Error('selected_sections_missing');
    }

    return {
      detected_format: 'data_bundle',
      imported_account_count: accountResult?.imported_count ?? 0,
      account_result: accountResult,
      config_result: configResult,
      warnings,
    };
  }

  if (isAccountTransferBundleLike(parsed)) {
    if (!options.includeAccounts) {
      throw new Error('accounts_section_required');
    }

    const accountResult = await importAllAccountsFromTransferJson(jsonContent, {
      onProgress: options.onAccountProgress,
    });

    return {
      detected_format: 'account_bundle',
      imported_account_count: accountResult.imported_count,
      account_result: accountResult,
      config_result: null,
      warnings: options.includeConfig ? ['config_section_missing'] : [],
    };
  }

  const legacyPlatform = detectLegacyPlatform(parsed);
  if (!legacyPlatform) {
    throw new Error('unsupported_legacy_account_json');
  }
  if (!options.includeAccounts) {
    throw new Error('accounts_section_required');
  }

  const accountResult = await importLegacyAccountJson(legacyPlatform, jsonContent);
  return {
    detected_format: 'legacy_account_json',
    legacy_account_platform: legacyPlatform,
    imported_account_count: accountResult.imported_count,
    account_result: accountResult,
    config_result: null,
    warnings: options.includeConfig ? ['config_section_missing'] : [],
  };
}

export { DATA_TRANSFER_SCHEMA, DATA_TRANSFER_VERSION, CURRENT_ACCOUNT_REFRESH_STORAGE_KEY };
