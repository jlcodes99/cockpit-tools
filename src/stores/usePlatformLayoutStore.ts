import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { ALL_PLATFORM_IDS, PlatformId } from '../types/platform';
import { CLASSIC_SIDEBAR_ENTRY_LIMIT } from './useSideNavLayoutStore';

const PLATFORM_LAYOUT_STORAGE_KEY = 'agtools.platform_layout.v1';
const LEGACY_TRAY_CORE_IDS: PlatformId[] = ['antigravity', 'codex', 'github-copilot', 'windsurf'];
const TRAY_MIGRATED_PLATFORM_IDS: PlatformId[] = [
  'antigravity_ide',
  'claude_manager',
  'zed',
  'kiro',
  'cursor',
  'gemini',
  'codebuddy',
  'codebuddy_cn',
  'qoder',
  'trae',
  'trae_solo',
  'trae_cn',
  'trae_solo_cn',
  'workbuddy',
];
const DEFAULT_CODEBUDDY_GROUP_ID = 'codebuddy-suite';
const DEFAULT_ANTIGRAVITY_GROUP_ID = 'antigravity-suite';
const DEFAULT_TRAE_GROUP_ID = 'trae-suite';
const TRAE_SUITE_PLATFORM_IDS: PlatformId[] = ['trae', 'trae_solo', 'trae_cn', 'trae_solo_cn'];

const PLATFORM_ENTRY_PREFIX = 'platform:';
const GROUP_ENTRY_PREFIX = 'group:';
export const API_RELAY_LAYOUT_ENTRY_ID = 'feature:api-relay' as const;

export type PlatformLayoutEntryId = `platform:${PlatformId}` | `group:${string}`;
export type ApiRelayLayoutEntryId = typeof API_RELAY_LAYOUT_ENTRY_ID;
export type PlatformGroupIconKind = 'platform' | 'custom';

export interface PlatformLayoutGroupChildConfig {
  platformId: PlatformId;
  name?: string;
  iconKind?: PlatformGroupIconKind;
  iconPlatformId?: PlatformId;
  iconCustomDataUrl?: string;
}

export interface PlatformLayoutGroup {
  id: string;
  name: string;
  platformIds: PlatformId[];
  defaultPlatformId: PlatformId;
  iconKind: PlatformGroupIconKind;
  iconPlatformId?: PlatformId;
  iconCustomDataUrl?: string;
  childConfigs?: PlatformLayoutGroupChildConfig[];
}

type PersistedPlatformLayout = {
  orderedPlatformIds?: PlatformId[];
  hiddenPlatformIds?: PlatformId[];
  sidebarPlatformIds?: PlatformId[];
  trayPlatformIds?: PlatformId[];
  traySortMode?: 'auto' | 'manual';
  platformGroups?: PlatformLayoutGroup[];
  orderedEntryIds?: PlatformLayoutEntryId[];
  hiddenEntryIds?: PlatformLayoutEntryId[];
  sidebarEntryIds?: PlatformLayoutEntryId[];
  antigravityGroupFirstMigrated?: boolean;
  traeSuiteDefaultGroupRestored?: boolean;
  apiRelaySidebarVisible?: boolean;
  apiRelayDashboardVisible?: boolean;
  apiRelayEntryOrder?: number;
};

interface PlatformLayoutState {
  orderedPlatformIds: PlatformId[];
  hiddenPlatformIds: PlatformId[];
  sidebarPlatformIds: PlatformId[];
  trayPlatformIds: PlatformId[];
  traySortMode: 'auto' | 'manual';

  platformGroups: PlatformLayoutGroup[];
  orderedEntryIds: PlatformLayoutEntryId[];
  hiddenEntryIds: PlatformLayoutEntryId[];
  sidebarEntryIds: PlatformLayoutEntryId[];
  antigravityGroupFirstMigrated: boolean;
  traeSuiteDefaultGroupRestored: boolean;
  apiRelaySidebarVisible: boolean;
  apiRelayDashboardVisible: boolean;
  apiRelayEntryOrder: number;

  movePlatform: (fromIndex: number, toIndex: number) => void;
  toggleHiddenPlatform: (id: PlatformId) => void;
  setHiddenPlatform: (id: PlatformId, hidden: boolean) => void;
  toggleSidebarPlatform: (id: PlatformId) => void;
  setSidebarPlatform: (id: PlatformId, enabled: boolean) => void;

  moveEntry: (fromIndex: number, toIndex: number) => void;
  setLayoutEntryOrder: (entryIds: PlatformLayoutEntryId[], apiRelayEntryOrder: number) => void;
  reorderGroupPlatforms: (groupId: string, fromIndex: number, toIndex: number) => void;
  toggleHiddenEntry: (id: PlatformLayoutEntryId) => void;
  setHiddenEntry: (id: PlatformLayoutEntryId, hidden: boolean) => void;
  toggleSidebarEntry: (id: PlatformLayoutEntryId) => void;
  setSidebarEntry: (id: PlatformLayoutEntryId, enabled: boolean) => void;
  syncSidebarEntriesFromDashboard: () => void;
  setApiRelaySidebarVisible: (visible: boolean) => void;
  setApiRelayDashboardVisible: (visible: boolean) => void;
  setApiRelayEntryOrder: (order: number) => void;

  upsertPlatformGroup: (group: PlatformLayoutGroup) => void;
  removePlatformGroup: (groupId: string) => void;

  toggleTrayPlatform: (id: PlatformId) => void;
  setTrayPlatform: (id: PlatformId, enabled: boolean) => void;
  syncTrayLayout: () => void;
  resetPlatformLayout: () => void;
}

interface NormalizedLayoutStateData {
  orderedPlatformIds: PlatformId[];
  hiddenPlatformIds: PlatformId[];
  sidebarPlatformIds: PlatformId[];
  trayPlatformIds: PlatformId[];
  traySortMode: 'auto' | 'manual';
  platformGroups: PlatformLayoutGroup[];
  orderedEntryIds: PlatformLayoutEntryId[];
  hiddenEntryIds: PlatformLayoutEntryId[];
  sidebarEntryIds: PlatformLayoutEntryId[];
  antigravityGroupFirstMigrated: boolean;
  traeSuiteDefaultGroupRestored: boolean;
  apiRelaySidebarVisible: boolean;
  apiRelayDashboardVisible: boolean;
  apiRelayEntryOrder: number;
}

let trayLayoutSyncTimer: number | null = null;

export function makePlatformEntryId(platformId: PlatformId): PlatformLayoutEntryId {
  return `${PLATFORM_ENTRY_PREFIX}${platformId}` as PlatformLayoutEntryId;
}

export function makeGroupEntryId(groupId: string): PlatformLayoutEntryId {
  return `${GROUP_ENTRY_PREFIX}${groupId}` as PlatformLayoutEntryId;
}

export function parsePlatformEntryId(entryId: string): PlatformId | null {
  if (!entryId.startsWith(PLATFORM_ENTRY_PREFIX)) {
    return null;
  }
  const value = entryId.slice(PLATFORM_ENTRY_PREFIX.length);
  if (!ALL_PLATFORM_IDS.includes(value as PlatformId)) {
    return null;
  }
  return value as PlatformId;
}

export function parseGroupEntryId(entryId: string): string | null {
  if (!entryId.startsWith(GROUP_ENTRY_PREFIX)) {
    return null;
  }
  const value = entryId.slice(GROUP_ENTRY_PREFIX.length).trim();
  return value || null;
}

export function findGroupByPlatform(
  groups: PlatformLayoutGroup[],
  platformId: PlatformId,
): PlatformLayoutGroup | null {
  for (const group of groups) {
    if (group.platformIds.includes(platformId)) {
      return group;
    }
  }
  return null;
}

export function getGroupChildConfig(
  group: PlatformLayoutGroup,
  platformId: PlatformId,
): PlatformLayoutGroupChildConfig | null {
  const childConfigs = group.childConfigs ?? [];
  return childConfigs.find((item) => item.platformId === platformId) ?? null;
}

export function resolveGroupChildName(
  group: PlatformLayoutGroup,
  platformId: PlatformId,
  fallbackName: string,
): string {
  const config = getGroupChildConfig(group, platformId);
  if (!config?.name?.trim()) {
    return fallbackName;
  }
  return config.name.trim();
}

export function resolveGroupChildIcon(
  group: PlatformLayoutGroup,
  platformId: PlatformId,
): {
  iconKind: PlatformGroupIconKind;
  iconPlatformId: PlatformId;
  iconCustomDataUrl?: string;
} {
  const config = getGroupChildConfig(group, platformId);
  if (config?.iconKind === 'custom' && config.iconCustomDataUrl?.trim()) {
    return {
      iconKind: 'custom',
      iconPlatformId: platformId,
      iconCustomDataUrl: config.iconCustomDataUrl.trim(),
    };
  }
  const iconPlatformId = ALL_PLATFORM_IDS.includes(config?.iconPlatformId as PlatformId)
    ? (config?.iconPlatformId as PlatformId)
    : platformId;
  return {
    iconKind: 'platform',
    iconPlatformId,
  };
}

export function resolveEntryIdForPlatform(
  platformId: PlatformId,
  groups: PlatformLayoutGroup[],
): PlatformLayoutEntryId {
  const group = findGroupByPlatform(groups, platformId);
  if (group) {
    return makeGroupEntryId(group.id);
  }
  return makePlatformEntryId(platformId);
}

export function resolveEntryDefaultPlatformId(
  entryId: PlatformLayoutEntryId,
  groups: PlatformLayoutGroup[],
): PlatformId | null {
  const platformId = parsePlatformEntryId(entryId);
  if (platformId) {
    return platformId;
  }
  const groupId = parseGroupEntryId(entryId);
  if (!groupId) {
    return null;
  }
  const group = groups.find((item) => item.id === groupId);
  if (!group) {
    return null;
  }
  return group.defaultPlatformId;
}

export function resolveEntryPlatformIds(
  entryId: PlatformLayoutEntryId,
  groups: PlatformLayoutGroup[],
): PlatformId[] {
  const platformId = parsePlatformEntryId(entryId);
  if (platformId) {
    return [platformId];
  }
  const groupId = parseGroupEntryId(entryId);
  if (!groupId) {
    return [];
  }
  const group = groups.find((item) => item.id === groupId);
  return group ? [...group.platformIds] : [];
}

function createDefaultTraeSuiteGroup(): PlatformLayoutGroup {
  return {
    id: DEFAULT_TRAE_GROUP_ID,
    name: 'Trae',
    platformIds: TRAE_SUITE_PLATFORM_IDS,
    defaultPlatformId: 'trae',
    iconKind: 'platform',
    iconPlatformId: 'trae',
    childConfigs: [
      { platformId: 'trae', name: 'Trae' },
      { platformId: 'trae_solo', name: 'TRAE SOLO' },
      { platformId: 'trae_cn', name: 'Trae CN' },
      { platformId: 'trae_solo_cn', name: 'TRAE SOLO CN' },
    ],
  };
}

function defaultPlatformGroups(): PlatformLayoutGroup[] {
  return [
    {
      id: DEFAULT_ANTIGRAVITY_GROUP_ID,
      name: 'Antigravity',
      platformIds: ['antigravity', 'antigravity_ide'],
      defaultPlatformId: 'antigravity_ide',
      iconKind: 'platform',
      iconPlatformId: 'antigravity_ide',
      childConfigs: [
        { platformId: 'antigravity', name: 'Antigravity' },
        { platformId: 'antigravity_ide', name: 'Antigravity IDE' },
      ],
    },
    {
      id: DEFAULT_CODEBUDDY_GROUP_ID,
      name: 'CodeBuddy',
      platformIds: ['codebuddy', 'codebuddy_cn', 'workbuddy'],
      defaultPlatformId: 'codebuddy',
      iconKind: 'platform',
      iconPlatformId: 'codebuddy',
    },
    createDefaultTraeSuiteGroup(),
  ];
}

function sanitizePlatformIds(list: unknown): PlatformId[] {
  if (!Array.isArray(list)) return [];
  const seen = new Set<PlatformId>();
  const result: PlatformId[] = [];
  for (const item of list) {
    if (typeof item !== 'string') continue;
    const id = item as PlatformId;
    if (!ALL_PLATFORM_IDS.includes(id)) continue;
    if (seen.has(id)) continue;
    seen.add(id);
    result.push(id);
  }
  return result;
}

function normalizeOrder(order: PlatformId[]): PlatformId[] {
  const next = sanitizePlatformIds(order);
  for (const id of ALL_PLATFORM_IDS) {
    if (!next.includes(id)) {
      next.push(id);
    }
  }
  return next;
}

function defaultPlatformOrder(): PlatformId[] {
  return [...ALL_PLATFORM_IDS];
}

function defaultSidebarEntryIds(
  groups: PlatformLayoutGroup[],
  orderedEntryIds = buildEntryOrderFromPlatformOrder(defaultPlatformOrder(), groups),
): PlatformLayoutEntryId[] {
  return orderedEntryIds.slice(0, CLASSIC_SIDEBAR_ENTRY_LIMIT - 1);
}

function defaultSidebarPlatformIds(): PlatformId[] {
  return ['claude_manager', 'codex', 'antigravity', 'zed', 'github-copilot'];
}

function normalizeHidden(hidden: PlatformId[]): PlatformId[] {
  return sanitizePlatformIds(hidden);
}

function normalizeSidebar(sidebar: PlatformId[], hidden: PlatformId[]): PlatformId[] {
  return sanitizePlatformIds(sidebar).filter((id) => !hidden.includes(id));
}

function normalizeTray(
  tray: PlatformId[],
  rawOrder: PlatformId[] = [],
  allowLegacyMigration = false,
): PlatformId[] {
  const normalized = sanitizePlatformIds(tray);
  const rawOrderSet = new Set(sanitizePlatformIds(rawOrder));
  const hasLegacyDefault = LEGACY_TRAY_CORE_IDS.every((id) => normalized.includes(id))
    && normalized.length <= ALL_PLATFORM_IDS.length - 1;

  if (!allowLegacyMigration || !hasLegacyDefault) {
    return normalized;
  }

  const next = [...normalized];
  for (const id of TRAY_MIGRATED_PLATFORM_IDS) {
    if (next.includes(id) || rawOrderSet.has(id)) {
      continue;
    }
    next.push(id);
  }
  return next;
}

function normalizeTraySortMode(mode: unknown): 'auto' | 'manual' {
  return mode === 'manual' ? 'manual' : 'auto';
}

function normalizeApiRelayEntryOrder(order: unknown, entryCount: number): number {
  const raw = typeof order === 'number' ? order : Number(order);
  if (!Number.isFinite(raw)) {
    return 0;
  }
  return Math.max(0, Math.min(entryCount, Math.trunc(raw)));
}

function normalizeGroupId(raw: unknown, index: number): string {
  if (typeof raw === 'string') {
    const cleaned = raw.trim().toLowerCase().replace(/[^a-z0-9_-]/g, '-');
    if (cleaned) {
      return cleaned;
    }
  }
  return `group-${index + 1}`;
}

function normalizeGroupName(raw: unknown, fallbackPlatform: PlatformId): string {
  if (typeof raw === 'string') {
    const name = raw.trim();
    if (name) {
      return name;
    }
  }
  if (fallbackPlatform === 'antigravity') {
    return 'Antigravity';
  }
  if (fallbackPlatform === 'antigravity_ide') {
    return 'Antigravity IDE';
  }
  if (fallbackPlatform === 'codebuddy_cn') {
    return 'CodeBuddy CN';
  }
  if (fallbackPlatform === 'github-copilot') {
    return 'GitHub Copilot';
  }
  if (fallbackPlatform === 'zed') {
    return 'Zed';
  }
  if (fallbackPlatform === 'claude_manager') {
    return 'Claude';
  }
  if (fallbackPlatform === 'workbuddy') {
    return 'WorkBuddy';
  }
  if (fallbackPlatform === 'qoder') {
    return 'Qoder';
  }
  if (fallbackPlatform === 'trae') {
    return 'Trae';
  }
  if (fallbackPlatform === 'trae_solo') {
    return 'TRAE SOLO';
  }
  if (fallbackPlatform === 'trae_cn') {
    return 'Trae CN';
  }
  if (fallbackPlatform === 'trae_solo_cn') {
    return 'TRAE SOLO CN';
  }
  if (fallbackPlatform === 'gemini') {
    return 'Gemini Cli';
  }
  return fallbackPlatform.charAt(0).toUpperCase() + fallbackPlatform.slice(1);
}

function normalizeAntigravitySuiteGroupName(name: string, platformIds: PlatformId[]): string {
  if (
    platformIds.includes('antigravity')
    && platformIds.includes('antigravity_ide')
    && (name === 'Antigravity IDE' || name === 'Antigravity')
  ) {
    return 'Antigravity';
  }
  return name;
}

function isDefaultTraeSuiteSingletonGroup(group: PlatformLayoutGroup): boolean {
  const platformId = group.platformIds[0];
  return (
    group.platformIds.length === 1
    && TRAE_SUITE_PLATFORM_IDS.includes(platformId)
    && group.id === `platform-${platformId}`
    && group.name === normalizeGroupName(undefined, platformId)
    && group.defaultPlatformId === platformId
    && group.iconKind === 'platform'
    && group.iconPlatformId === platformId
    && (group.childConfigs ?? []).length === 0
  );
}

function restoreDefaultTraeSuiteGroup(groups: PlatformLayoutGroup[]): PlatformLayoutGroup[] {
  const singletonIndexes = TRAE_SUITE_PLATFORM_IDS.map((platformId) =>
    groups.findIndex((group) =>
      group.platformIds.length === 1
      && group.platformIds[0] === platformId
      && isDefaultTraeSuiteSingletonGroup(group)
    )
  );
  if (singletonIndexes.some((index) => index < 0)) {
    return groups;
  }

  const singletonIndexSet = new Set(singletonIndexes);
  const insertIndex = Math.min(...singletonIndexes);
  const next = groups.filter((_, index) => !singletonIndexSet.has(index));
  next.splice(insertIndex, 0, createDefaultTraeSuiteGroup());
  return next;
}

function normalizeGroupChildName(raw: unknown, platformId: PlatformId): string | undefined {
  if (typeof raw !== 'string') {
    return undefined;
  }
  const value = raw.trim();
  if (!value) {
    return undefined;
  }
  if (platformId === 'antigravity_ide' && value === 'Antigravity') {
    return 'Antigravity IDE';
  }
  if (platformId === 'claude_manager' && (value === 'Claude' || value === 'Claude CLI')) {
    return 'Claude';
  }
  return value;
}

function normalizeGroupChildConfigs(
  rawChildConfigs: unknown,
  platformIds: PlatformId[],
): PlatformLayoutGroupChildConfig[] {
  if (!Array.isArray(rawChildConfigs) || platformIds.length === 0) {
    return [];
  }

  const platformSet = new Set(platformIds);
  const dedup = new Map<PlatformId, PlatformLayoutGroupChildConfig>();

  for (const item of rawChildConfigs) {
    if (!item || typeof item !== 'object') {
      continue;
    }
    const record = item as Partial<PlatformLayoutGroupChildConfig>;
    const platformId = platformSet.has(record.platformId as PlatformId)
      ? (record.platformId as PlatformId)
      : null;
    if (!platformId) {
      continue;
    }
    const name = normalizeGroupChildName(record.name, platformId);
    const iconKind: PlatformGroupIconKind = record.iconKind === 'custom' ? 'custom' : 'platform';
    const iconPlatformId = ALL_PLATFORM_IDS.includes(record.iconPlatformId as PlatformId)
      ? (record.iconPlatformId as PlatformId)
      : platformId;
    const iconCustomDataUrl =
      iconKind === 'custom' && typeof record.iconCustomDataUrl === 'string'
        ? record.iconCustomDataUrl.trim()
        : undefined;

    if (!name && iconKind !== 'custom' && iconPlatformId === platformId) {
      dedup.delete(platformId);
      continue;
    }

    dedup.set(platformId, {
      platformId,
      name,
      iconKind,
      iconPlatformId,
      iconCustomDataUrl,
    });
  }

  return platformIds
    .map((platformId) => dedup.get(platformId))
    .filter((item): item is PlatformLayoutGroupChildConfig => !!item);
}

function normalizePlatformGroups(
  raw: unknown,
  fallbackToDefault: boolean,
  options: { restoreDefaultTraeSuiteGroup?: boolean } = {},
): PlatformLayoutGroup[] {
  const shouldRestoreDefaultTraeSuiteGroup = options.restoreDefaultTraeSuiteGroup === true;
  const source = Array.isArray(raw) ? raw : (fallbackToDefault ? defaultPlatformGroups() : []);
  const result: PlatformLayoutGroup[] = [];
  const usedPlatformIds = new Set<PlatformId>();
  const usedGroupIds = new Set<string>();

  source.forEach((item, index) => {
    if (!item || typeof item !== 'object') {
      return;
    }
    const record = item as Partial<PlatformLayoutGroup>;
    let groupId = normalizeGroupId(record.id, index);
    if (usedGroupIds.has(groupId)) {
      groupId = `${groupId}-${index + 1}`;
    }
    const platformIds = sanitizePlatformIds(record.platformIds).filter((platformId) => {
      if (usedPlatformIds.has(platformId)) {
        return false;
      }
      usedPlatformIds.add(platformId);
      return true;
    });
    if (platformIds.length === 0) {
      return;
    }

    const defaultPlatformId = platformIds.includes(record.defaultPlatformId as PlatformId)
      ? (record.defaultPlatformId as PlatformId)
      : platformIds[0];

    const iconKind: PlatformGroupIconKind = record.iconKind === 'custom' ? 'custom' : 'platform';
    const iconPlatformId = platformIds.includes(record.iconPlatformId as PlatformId)
      ? (record.iconPlatformId as PlatformId)
      : defaultPlatformId;
    const iconCustomDataUrl =
      iconKind === 'custom' && typeof record.iconCustomDataUrl === 'string'
        ? record.iconCustomDataUrl.trim()
        : undefined;

    result.push({
      id: groupId,
      name: normalizeAntigravitySuiteGroupName(
        normalizeGroupName(record.name, defaultPlatformId),
        platformIds,
      ),
      platformIds,
      defaultPlatformId,
      iconKind,
      iconPlatformId,
      iconCustomDataUrl,
      childConfigs: normalizeGroupChildConfigs(record.childConfigs, platformIds),
    });
    usedGroupIds.add(groupId);
  });

  if (!usedPlatformIds.has('antigravity_ide')) {
    const antigravityGroup = result.find((group) => group.platformIds.includes('antigravity'));
    if (antigravityGroup) {
      antigravityGroup.platformIds = [...antigravityGroup.platformIds, 'antigravity_ide'];
      antigravityGroup.defaultPlatformId = 'antigravity_ide';
      antigravityGroup.iconPlatformId =
        antigravityGroup.iconKind === 'custom' ? antigravityGroup.iconPlatformId : 'antigravity_ide';
      if (antigravityGroup.name === 'Antigravity IDE' || antigravityGroup.name === 'Antigravity') {
        antigravityGroup.name = 'Antigravity';
      }
      antigravityGroup.childConfigs = normalizeGroupChildConfigs(
        [
          ...(antigravityGroup.childConfigs ?? []),
          { platformId: 'antigravity', name: 'Antigravity' },
          { platformId: 'antigravity_ide', name: 'Antigravity IDE' },
        ],
        antigravityGroup.platformIds,
      );
      usedPlatformIds.add('antigravity_ide');
    }
  }

  if (shouldRestoreDefaultTraeSuiteGroup) {
    const restoredTraeSuiteGroups = restoreDefaultTraeSuiteGroup(result);
    if (restoredTraeSuiteGroups !== result) {
      result.splice(0, result.length, ...restoredTraeSuiteGroups);
    }
  }

  const missingTraeSuiteIds = TRAE_SUITE_PLATFORM_IDS.filter(
    (platformId) => !usedPlatformIds.has(platformId),
  );
  if (shouldRestoreDefaultTraeSuiteGroup && missingTraeSuiteIds.length > 0) {
    const traeGroup =
      result.find((group) => group.id === DEFAULT_TRAE_GROUP_ID)
      ?? result.find((group) => group.platformIds.includes('trae'));
    if (traeGroup) {
      traeGroup.platformIds = [...traeGroup.platformIds, ...missingTraeSuiteIds];
      if (!TRAE_SUITE_PLATFORM_IDS.includes(traeGroup.defaultPlatformId)) {
        traeGroup.defaultPlatformId = 'trae';
      }
      if (traeGroup.iconKind !== 'custom') {
        traeGroup.iconPlatformId = 'trae';
      }
      if (traeGroup.name === 'platform-trae' || traeGroup.name === 'Trae') {
        traeGroup.name = 'Trae';
      }
      traeGroup.childConfigs = normalizeGroupChildConfigs(
        [
          ...(traeGroup.childConfigs ?? []),
          { platformId: 'trae', name: 'Trae' },
          { platformId: 'trae_solo', name: 'TRAE SOLO' },
          { platformId: 'trae_cn', name: 'Trae CN' },
          { platformId: 'trae_solo_cn', name: 'TRAE SOLO CN' },
        ],
        traeGroup.platformIds,
      );
      missingTraeSuiteIds.forEach((platformId) => usedPlatformIds.add(platformId));
    }
  }

  for (const platformId of ALL_PLATFORM_IDS) {
    if (usedPlatformIds.has(platformId)) {
      continue;
    }
    let singletonId = `platform-${platformId}`;
    let index = 1;
    while (usedGroupIds.has(singletonId)) {
      index += 1;
      singletonId = `platform-${platformId}-${index}`;
    }
    result.push({
      id: singletonId,
      name: normalizeGroupName(undefined, platformId),
      platformIds: [platformId],
      defaultPlatformId: platformId,
      iconKind: 'platform',
      iconPlatformId: platformId,
      childConfigs: [],
    });
    usedGroupIds.add(singletonId);
    usedPlatformIds.add(platformId);
  }

  return result;
}

function sortGroupPlatformsByOrder(group: PlatformLayoutGroup, order: PlatformId[]): PlatformLayoutGroup {
  const rank = new Map<PlatformId, number>();
  order.forEach((platformId, index) => rank.set(platformId, index));
  const sorted = [...group.platformIds].sort((a, b) => {
    const aRank = rank.get(a) ?? Number.MAX_SAFE_INTEGER;
    const bRank = rank.get(b) ?? Number.MAX_SAFE_INTEGER;
    return aRank - bRank;
  });
  return {
    ...group,
    platformIds: sorted,
    defaultPlatformId: sorted.includes(group.defaultPlatformId) ? group.defaultPlatformId : sorted[0],
    iconPlatformId: sorted.includes(group.iconPlatformId as PlatformId)
      ? group.iconPlatformId
      : sorted.includes(group.defaultPlatformId)
        ? group.defaultPlatformId
        : sorted[0],
    childConfigs: normalizeGroupChildConfigs(group.childConfigs, sorted),
  };
}

function getAvailableEntryIds(groups: PlatformLayoutGroup[]): PlatformLayoutEntryId[] {
  const grouped = new Set<PlatformId>();
  const entries: PlatformLayoutEntryId[] = [];

  for (const group of groups) {
    entries.push(makeGroupEntryId(group.id));
    for (const platformId of group.platformIds) {
      grouped.add(platformId);
    }
  }

  for (const platformId of ALL_PLATFORM_IDS) {
    if (grouped.has(platformId)) {
      continue;
    }
    entries.push(makePlatformEntryId(platformId));
  }

  return entries;
}

function buildEntryOrderFromPlatformOrder(
  platformOrder: PlatformId[],
  groups: PlatformLayoutGroup[],
): PlatformLayoutEntryId[] {
  const order = normalizeOrder(platformOrder);
  const platformToGroup = new Map<PlatformId, string>();
  for (const group of groups) {
    for (const platformId of group.platformIds) {
      platformToGroup.set(platformId, group.id);
    }
  }

  const addedGroups = new Set<string>();
  const entries: PlatformLayoutEntryId[] = [];
  for (const platformId of order) {
    const groupId = platformToGroup.get(platformId);
    if (groupId) {
      if (!addedGroups.has(groupId)) {
        entries.push(makeGroupEntryId(groupId));
        addedGroups.add(groupId);
      }
      continue;
    }
    entries.push(makePlatformEntryId(platformId));
  }

  const fallback = getAvailableEntryIds(groups);
  for (const entryId of fallback) {
    if (!entries.includes(entryId)) {
      entries.push(entryId);
    }
  }
  return entries;
}

function findDefaultAntigravityGroup(groups: PlatformLayoutGroup[]): PlatformLayoutGroup | null {
  return groups.find((group) => group.id === DEFAULT_ANTIGRAVITY_GROUP_ID)
    ?? groups.find((group) =>
      group.platformIds.includes('antigravity') && group.platformIds.includes('antigravity_ide')
    )
    ?? null;
}

function promoteDefaultAntigravityGroupEntry(
  entries: PlatformLayoutEntryId[],
  groups: PlatformLayoutGroup[],
): PlatformLayoutEntryId[] {
  const group = findDefaultAntigravityGroup(groups);
  if (!group) {
    return entries;
  }

  const groupEntryId = makeGroupEntryId(group.id);
  const index = entries.indexOf(groupEntryId);
  if (index <= 0) {
    return entries;
  }

  return [groupEntryId, ...entries.filter((entryId) => entryId !== groupEntryId)];
}

function normalizeEntryOrder(
  rawEntryIds: unknown,
  groups: PlatformLayoutGroup[],
  platformOrder: PlatformId[],
): PlatformLayoutEntryId[] {
  const available = getAvailableEntryIds(groups);
  const availableSet = new Set(available);
  const fallback = buildEntryOrderFromPlatformOrder(platformOrder, groups);

  if (!Array.isArray(rawEntryIds)) {
    return fallback;
  }

  const seen = new Set<PlatformLayoutEntryId>();
  const entries: PlatformLayoutEntryId[] = [];
  for (const item of rawEntryIds) {
    if (typeof item !== 'string') continue;
    const platformId = parsePlatformEntryId(item);
    const entryId = platformId
      ? resolveEntryIdForPlatform(platformId, groups)
      : (item as PlatformLayoutEntryId);
    if (!availableSet.has(entryId) || seen.has(entryId)) {
      continue;
    }
    seen.add(entryId);
    entries.push(entryId);
  }

  for (const entryId of fallback) {
    if (!seen.has(entryId)) {
      entries.push(entryId);
      seen.add(entryId);
    }
  }

  return entries;
}

function normalizeEntryVisibilityList(
  rawEntryIds: unknown,
  orderedEntryIds: PlatformLayoutEntryId[],
): PlatformLayoutEntryId[] {
  if (!Array.isArray(rawEntryIds)) {
    return [];
  }
  const orderSet = new Set(orderedEntryIds);
  const seen = new Set<PlatformLayoutEntryId>();
  const entries: PlatformLayoutEntryId[] = [];
  for (const item of rawEntryIds) {
    if (typeof item !== 'string') continue;
    const entryId = item as PlatformLayoutEntryId;
    if (!orderSet.has(entryId) || seen.has(entryId)) {
      continue;
    }
    seen.add(entryId);
    entries.push(entryId);
  }
  return entries;
}

function deriveEntryVisibilityFromLegacyPlatforms(
  legacyIds: PlatformId[],
  orderedEntryIds: PlatformLayoutEntryId[],
  groups: PlatformLayoutGroup[],
): PlatformLayoutEntryId[] {
  const legacySet = new Set(legacyIds);
  return orderedEntryIds.filter((entryId) => {
    const platformIds = resolveEntryPlatformIds(entryId, groups);
    return platformIds.some((platformId) => legacySet.has(platformId));
  });
}

function normalizeHiddenEntryIds(
  rawHiddenEntryIds: unknown,
  orderedEntryIds: PlatformLayoutEntryId[],
  groups: PlatformLayoutGroup[],
  legacyHiddenPlatformIds: PlatformId[],
): PlatformLayoutEntryId[] {
  const normalized = normalizeEntryVisibilityList(rawHiddenEntryIds, orderedEntryIds);
  if (normalized.length > 0) {
    return normalized;
  }

  if (Array.isArray(rawHiddenEntryIds)) {
    const rawItems = rawHiddenEntryIds.filter((item): item is string => typeof item === 'string');
    if (rawItems.length === 0) {
      return [];
    }

    const isLegacyPlatformEntryList = rawItems.every((entryId) => {
      const platformId = parsePlatformEntryId(entryId);
      if (!platformId) {
        return false;
      }
      const resolvedEntryId = resolveEntryIdForPlatform(platformId, groups);
      return resolvedEntryId !== entryId;
    });

    if (!isLegacyPlatformEntryList) {
      return [];
    }
  }

  return deriveEntryVisibilityFromLegacyPlatforms(
    legacyHiddenPlatformIds,
    orderedEntryIds,
    groups,
  );
}

function normalizeSidebarEntryIds(
  rawSidebarEntryIds: unknown,
  orderedEntryIds: PlatformLayoutEntryId[],
  _hiddenEntryIds: PlatformLayoutEntryId[],
  groups: PlatformLayoutGroup[],
  legacySidebarPlatformIds: PlatformId[],
): PlatformLayoutEntryId[] {
  const normalized = normalizeEntryVisibilityList(rawSidebarEntryIds, orderedEntryIds);
  if (normalized.length > 0) {
    return normalized;
  }

  if (Array.isArray(rawSidebarEntryIds)) {
    const rawItems = rawSidebarEntryIds.filter((item): item is string => typeof item === 'string');
    if (rawItems.length === 0) {
      return [];
    }

    const isLegacyPlatformEntryList = rawItems.every((entryId) => {
      const platformId = parsePlatformEntryId(entryId);
      if (!platformId) {
        return false;
      }
      const resolvedEntryId = resolveEntryIdForPlatform(platformId, groups);
      return resolvedEntryId !== entryId;
    });

    if (!isLegacyPlatformEntryList) {
      return [];
    }
  }

  const fallback = deriveEntryVisibilityFromLegacyPlatforms(
    legacySidebarPlatformIds,
    orderedEntryIds,
    groups,
  );

  if (fallback.length > 0) {
    return fallback;
  }

  return orderedEntryIds;
}

function derivePlatformOrderFromEntryOrder(
  orderedEntryIds: PlatformLayoutEntryId[],
  groups: PlatformLayoutGroup[],
  previousPlatformOrder: PlatformId[],
): PlatformId[] {
  const previousOrder = normalizeOrder(previousPlatformOrder);
  const rank = new Map<PlatformId, number>();
  previousOrder.forEach((platformId, index) => rank.set(platformId, index));

  const order: PlatformId[] = [];
  const seen = new Set<PlatformId>();

  const pushPlatform = (platformId: PlatformId) => {
    if (seen.has(platformId)) return;
    seen.add(platformId);
    order.push(platformId);
  };

  for (const entryId of orderedEntryIds) {
    const platformId = parsePlatformEntryId(entryId);
    if (platformId) {
      pushPlatform(platformId);
      continue;
    }

    const groupId = parseGroupEntryId(entryId);
    if (!groupId) {
      continue;
    }
    const group = groups.find((item) => item.id === groupId);
    if (!group) {
      continue;
    }

    const sorted = [...group.platformIds].sort((a, b) => {
      const aRank = rank.get(a) ?? Number.MAX_SAFE_INTEGER;
      const bRank = rank.get(b) ?? Number.MAX_SAFE_INTEGER;
      return aRank - bRank;
    });
    sorted.forEach(pushPlatform);
  }

  previousOrder.forEach(pushPlatform);
  ALL_PLATFORM_IDS.forEach(pushPlatform);
  return order;
}

function deriveHiddenPlatformIds(
  hiddenEntryIds: PlatformLayoutEntryId[],
  groups: PlatformLayoutGroup[],
): PlatformId[] {
  const hiddenSet = new Set(hiddenEntryIds);
  const result: PlatformId[] = [];
  for (const platformId of ALL_PLATFORM_IDS) {
    const entryId = resolveEntryIdForPlatform(platformId, groups);
    if (hiddenSet.has(entryId)) {
      result.push(platformId);
    }
  }
  return result;
}

function deriveSidebarPlatformIds(
  sidebarEntryIds: PlatformLayoutEntryId[],
  _hiddenEntryIds: PlatformLayoutEntryId[],
  groups: PlatformLayoutGroup[],
): PlatformId[] {
  const result: PlatformId[] = [];
  for (const entryId of sidebarEntryIds) {
    const platformId = resolveEntryDefaultPlatformId(entryId, groups);
    if (!platformId || result.includes(platformId)) {
      continue;
    }
    result.push(platformId);
  }
  return result;
}

function toTrayGroupPayload(groups: PlatformLayoutGroup[]) {
  return groups.map((group) => ({
    id: group.id,
    name: group.name,
    platformIds: [...group.platformIds],
    defaultPlatformId: group.defaultPlatformId,
  }));
}

function syncTrayLayoutToBackend(
  state: Pick<
    PlatformLayoutState,
    'orderedPlatformIds' | 'trayPlatformIds' | 'traySortMode' | 'orderedEntryIds' | 'platformGroups'
  >,
) {
  invoke('save_tray_platform_layout', {
    sortMode: state.traySortMode,
    orderedPlatformIds: state.orderedPlatformIds,
    trayPlatformIds: state.trayPlatformIds,
    orderedEntryIds: state.orderedEntryIds,
    platformGroups: toTrayGroupPayload(state.platformGroups),
  }).catch((error) => {
    console.error('同步托盘平台布局失败:', error);
  });
}

function scheduleTrayLayoutSync(
  state: Pick<
    PlatformLayoutState,
    'orderedPlatformIds' | 'trayPlatformIds' | 'traySortMode' | 'orderedEntryIds' | 'platformGroups'
  >,
) {
  if (typeof window === 'undefined') {
    return;
  }
  if (trayLayoutSyncTimer) {
    window.clearTimeout(trayLayoutSyncTimer);
  }
  trayLayoutSyncTimer = window.setTimeout(() => {
    trayLayoutSyncTimer = null;
    syncTrayLayoutToBackend(state);
  }, 120);
}

function normalizeStateData(
  raw: {
    orderedPlatformIds: PlatformId[];
    hiddenPlatformIds: PlatformId[];
    sidebarPlatformIds: PlatformId[];
    trayPlatformIds: PlatformId[];
    traySortMode: 'auto' | 'manual';
    platformGroups: PlatformLayoutGroup[];
    orderedEntryIds: PlatformLayoutEntryId[];
    hiddenEntryIds: PlatformLayoutEntryId[];
    sidebarEntryIds: PlatformLayoutEntryId[];
    antigravityGroupFirstMigrated?: boolean;
    traeSuiteDefaultGroupRestored?: boolean;
    apiRelaySidebarVisible?: boolean;
    apiRelayDashboardVisible?: boolean;
    apiRelayEntryOrder?: number;
  },
  options: {
    allowLegacyTrayMigration?: boolean;
    promoteAntigravityGroupEntry?: boolean;
  } = {},
): NormalizedLayoutStateData {
  const normalizedPlatformOrder = normalizeOrder(raw.orderedPlatformIds);
  const platformGroups = normalizePlatformGroups(raw.platformGroups, false)
    .map((group) => sortGroupPlatformsByOrder(group, normalizedPlatformOrder));
  const normalizedEntryIds = normalizeEntryOrder(raw.orderedEntryIds, platformGroups, normalizedPlatformOrder);
  const orderedEntryIds = options.promoteAntigravityGroupEntry === true
    ? promoteDefaultAntigravityGroupEntry(normalizedEntryIds, platformGroups)
    : normalizedEntryIds;
  const orderedPlatformIds = derivePlatformOrderFromEntryOrder(
    orderedEntryIds,
    platformGroups,
    normalizedPlatformOrder,
  );
  const hiddenEntryIds = normalizeHiddenEntryIds(
    raw.hiddenEntryIds,
    orderedEntryIds,
    platformGroups,
    normalizeHidden(raw.hiddenPlatformIds),
  );
  const sidebarEntryIds = normalizeSidebarEntryIds(
    raw.sidebarEntryIds,
    orderedEntryIds,
    hiddenEntryIds,
    platformGroups,
    normalizeSidebar(raw.sidebarPlatformIds, []),
  );

  const hiddenPlatformIds = deriveHiddenPlatformIds(hiddenEntryIds, platformGroups);
  const sidebarPlatformIds = deriveSidebarPlatformIds(sidebarEntryIds, hiddenEntryIds, platformGroups);

  return {
    orderedPlatformIds,
    hiddenPlatformIds,
    sidebarPlatformIds,
    trayPlatformIds: normalizeTray(
      raw.trayPlatformIds,
      orderedPlatformIds,
      options.allowLegacyTrayMigration === true,
    ),
    traySortMode: normalizeTraySortMode(raw.traySortMode),
    platformGroups,
    orderedEntryIds,
    hiddenEntryIds,
    sidebarEntryIds,
    antigravityGroupFirstMigrated:
      raw.antigravityGroupFirstMigrated !== false || options.promoteAntigravityGroupEntry === true,
    traeSuiteDefaultGroupRestored: raw.traeSuiteDefaultGroupRestored !== false,
    apiRelaySidebarVisible: raw.apiRelaySidebarVisible !== false,
    apiRelayDashboardVisible: raw.apiRelayDashboardVisible !== false,
    apiRelayEntryOrder: normalizeApiRelayEntryOrder(raw.apiRelayEntryOrder, orderedEntryIds.length),
  };
}

function loadPersistedState(): NormalizedLayoutStateData {
  try {
    const raw = localStorage.getItem(PLATFORM_LAYOUT_STORAGE_KEY);
    if (!raw) {
      const defaultGroups = defaultPlatformGroups();
      const defaultOrder = defaultPlatformOrder();
      const defaults = normalizeStateData({
        orderedPlatformIds: defaultOrder,
        hiddenPlatformIds: [],
        sidebarPlatformIds: defaultSidebarPlatformIds(),
        trayPlatformIds: defaultOrder,
        traySortMode: 'auto',
        platformGroups: defaultGroups,
        orderedEntryIds: buildEntryOrderFromPlatformOrder(defaultOrder, defaultGroups),
        hiddenEntryIds: [],
        sidebarEntryIds: defaultSidebarEntryIds(defaultGroups),
        antigravityGroupFirstMigrated: true,
        traeSuiteDefaultGroupRestored: true,
        apiRelaySidebarVisible: true,
        apiRelayDashboardVisible: true,
        apiRelayEntryOrder: 0,
      });
      return defaults;
    }

    const parsed = JSON.parse(raw) as PersistedPlatformLayout;
    const antigravityGroupFirstMigrated = parsed.antigravityGroupFirstMigrated === true;
    const traeSuiteDefaultGroupRestored = parsed.traeSuiteDefaultGroupRestored === true;
    const orderedPlatformIds = normalizeOrder(parsed.orderedPlatformIds ?? defaultPlatformOrder());
    const hiddenPlatformIds = normalizeHidden(parsed.hiddenPlatformIds ?? []);
    const sidebarPlatformIds = normalizeSidebar(
      parsed.sidebarPlatformIds ?? defaultSidebarPlatformIds(),
      hiddenPlatformIds,
    );

    const platformGroups = normalizePlatformGroups(
      parsed.platformGroups,
      parsed.platformGroups === undefined,
      { restoreDefaultTraeSuiteGroup: !traeSuiteDefaultGroupRestored },
    ).map((group) => sortGroupPlatformsByOrder(group, orderedPlatformIds));

    const orderedEntryIds = normalizeEntryOrder(parsed.orderedEntryIds, platformGroups, orderedPlatformIds);
    const hiddenEntryIds = normalizeHiddenEntryIds(
      parsed.hiddenEntryIds,
      orderedEntryIds,
      platformGroups,
      hiddenPlatformIds,
    );
    const sidebarEntryIds = normalizeSidebarEntryIds(
      parsed.sidebarEntryIds,
      orderedEntryIds,
      hiddenEntryIds,
      platformGroups,
      sidebarPlatformIds,
    );

    const normalized = normalizeStateData({
      orderedPlatformIds,
      hiddenPlatformIds,
      sidebarPlatformIds,
      trayPlatformIds: normalizeTray(
        parsed.trayPlatformIds ?? defaultPlatformOrder(),
        sanitizePlatformIds(parsed.orderedPlatformIds ?? []),
        true,
      ),
      traySortMode: normalizeTraySortMode(parsed.traySortMode),
      platformGroups,
      orderedEntryIds,
      hiddenEntryIds,
      sidebarEntryIds,
      antigravityGroupFirstMigrated,
      traeSuiteDefaultGroupRestored: true,
      apiRelaySidebarVisible: parsed.apiRelaySidebarVisible,
      apiRelayDashboardVisible: parsed.apiRelayDashboardVisible,
      apiRelayEntryOrder: parsed.apiRelayEntryOrder,
    }, {
      promoteAntigravityGroupEntry: !antigravityGroupFirstMigrated,
    });
    if (!antigravityGroupFirstMigrated || !traeSuiteDefaultGroupRestored) {
      persist(normalized);
    }
    return normalized;
  } catch {
    const defaultGroups = defaultPlatformGroups();
    const defaultOrder = defaultPlatformOrder();
    return normalizeStateData({
      orderedPlatformIds: defaultOrder,
      hiddenPlatformIds: [],
      sidebarPlatformIds: defaultSidebarPlatformIds(),
      trayPlatformIds: defaultOrder,
      traySortMode: 'auto',
      platformGroups: defaultGroups,
      orderedEntryIds: buildEntryOrderFromPlatformOrder(defaultOrder, defaultGroups),
      hiddenEntryIds: [],
      sidebarEntryIds: defaultSidebarEntryIds(defaultGroups),
      antigravityGroupFirstMigrated: true,
      traeSuiteDefaultGroupRestored: true,
      apiRelaySidebarVisible: true,
      apiRelayDashboardVisible: true,
      apiRelayEntryOrder: 0,
    });
  }
}

function persist(
  state: Pick<
    PlatformLayoutState,
    | 'orderedPlatformIds'
    | 'hiddenPlatformIds'
    | 'sidebarPlatformIds'
    | 'trayPlatformIds'
    | 'traySortMode'
    | 'platformGroups'
    | 'orderedEntryIds'
    | 'hiddenEntryIds'
    | 'sidebarEntryIds'
    | 'antigravityGroupFirstMigrated'
    | 'traeSuiteDefaultGroupRestored'
    | 'apiRelaySidebarVisible'
    | 'apiRelayDashboardVisible'
    | 'apiRelayEntryOrder'
  >,
) {
  try {
    localStorage.setItem(PLATFORM_LAYOUT_STORAGE_KEY, JSON.stringify(state));
  } catch {
    // ignore persistence failures
  }
}

export const usePlatformLayoutStore = create<PlatformLayoutState>((set, get) => ({
  ...loadPersistedState(),

  movePlatform: (fromIndex, toIndex) => {
    const current = [...get().orderedPlatformIds];
    if (fromIndex < 0 || toIndex < 0 || fromIndex >= current.length || toIndex >= current.length) return;
    if (fromIndex === toIndex) return;

    const [item] = current.splice(fromIndex, 1);
    current.splice(toIndex, 0, item);

    const nextGroups = get().platformGroups.map((group) => sortGroupPlatformsByOrder(group, current));
    const nextOrderedEntryIds = normalizeEntryOrder(get().orderedEntryIds, nextGroups, current);

    const next = normalizeStateData({
      orderedPlatformIds: current,
      hiddenPlatformIds: get().hiddenPlatformIds,
      sidebarPlatformIds: get().sidebarPlatformIds,
      trayPlatformIds: get().trayPlatformIds,
      traySortMode: 'manual',
      platformGroups: nextGroups,
      orderedEntryIds: nextOrderedEntryIds,
      hiddenEntryIds: get().hiddenEntryIds,
      sidebarEntryIds: get().sidebarEntryIds,
      apiRelaySidebarVisible: get().apiRelaySidebarVisible,
      apiRelayDashboardVisible: get().apiRelayDashboardVisible,
      apiRelayEntryOrder: get().apiRelayEntryOrder,
    });

    set(next);
    persist(next);
    scheduleTrayLayoutSync(next);
  },

  toggleHiddenPlatform: (id) => {
    const entryId = resolveEntryIdForPlatform(id, get().platformGroups);
    get().toggleHiddenEntry(entryId);
  },

  setHiddenPlatform: (id, hidden) => {
    const entryId = resolveEntryIdForPlatform(id, get().platformGroups);
    get().setHiddenEntry(entryId, hidden);
  },

  toggleSidebarPlatform: (id) => {
    const entryId = resolveEntryIdForPlatform(id, get().platformGroups);
    get().toggleSidebarEntry(entryId);
  },

  setSidebarPlatform: (id, enabled) => {
    const entryId = resolveEntryIdForPlatform(id, get().platformGroups);
    get().setSidebarEntry(entryId, enabled);
  },

  moveEntry: (fromIndex, toIndex) => {
    const current = [...get().orderedEntryIds];
    if (fromIndex < 0 || toIndex < 0 || fromIndex >= current.length || toIndex >= current.length) return;
    if (fromIndex === toIndex) return;

    const [item] = current.splice(fromIndex, 1);
    current.splice(toIndex, 0, item);

    const orderedPlatformIds = derivePlatformOrderFromEntryOrder(
      current,
      get().platformGroups,
      get().orderedPlatformIds,
    );

    const nextGroups = get().platformGroups.map((group) => sortGroupPlatformsByOrder(group, orderedPlatformIds));

    const next = normalizeStateData({
      orderedPlatformIds,
      hiddenPlatformIds: get().hiddenPlatformIds,
      sidebarPlatformIds: get().sidebarPlatformIds,
      trayPlatformIds: get().trayPlatformIds,
      traySortMode: 'manual',
      platformGroups: nextGroups,
      orderedEntryIds: current,
      hiddenEntryIds: get().hiddenEntryIds,
      sidebarEntryIds: get().sidebarEntryIds,
      apiRelaySidebarVisible: get().apiRelaySidebarVisible,
      apiRelayDashboardVisible: get().apiRelayDashboardVisible,
      apiRelayEntryOrder: get().apiRelayEntryOrder,
    });

    set(next);
    persist(next);
    scheduleTrayLayoutSync(next);
  },

  setLayoutEntryOrder: (entryIds, apiRelayEntryOrder) => {
    const normalizedEntryIds = normalizeEntryOrder(
      entryIds,
      get().platformGroups,
      get().orderedPlatformIds,
    );
    const orderedPlatformIds = derivePlatformOrderFromEntryOrder(
      normalizedEntryIds,
      get().platformGroups,
      get().orderedPlatformIds,
    );
    const nextGroups = get().platformGroups.map((group) =>
      sortGroupPlatformsByOrder(group, orderedPlatformIds),
    );
    const orderedEntryIds = normalizeEntryOrder(
      normalizedEntryIds,
      nextGroups,
      orderedPlatformIds,
    );

    const next = normalizeStateData({
      orderedPlatformIds,
      hiddenPlatformIds: get().hiddenPlatformIds,
      sidebarPlatformIds: get().sidebarPlatformIds,
      trayPlatformIds: get().trayPlatformIds,
      traySortMode: 'manual',
      platformGroups: nextGroups,
      orderedEntryIds,
      hiddenEntryIds: get().hiddenEntryIds,
      sidebarEntryIds: get().sidebarEntryIds,
      apiRelaySidebarVisible: get().apiRelaySidebarVisible,
      apiRelayDashboardVisible: get().apiRelayDashboardVisible,
      apiRelayEntryOrder,
    });

    set(next);
    persist(next);
    scheduleTrayLayoutSync(next);
  },

  reorderGroupPlatforms: (groupId, fromIndex, toIndex) => {
    const currentGroups = get().platformGroups;
    const targetGroup = currentGroups.find((group) => group.id === groupId);
    if (!targetGroup) {
      return;
    }

    const nextGroupPlatformIds = [...targetGroup.platformIds];
    if (fromIndex < 0 || toIndex < 0 || fromIndex >= nextGroupPlatformIds.length || toIndex >= nextGroupPlatformIds.length) {
      return;
    }
    if (fromIndex === toIndex) {
      return;
    }

    const [moved] = nextGroupPlatformIds.splice(fromIndex, 1);
    nextGroupPlatformIds.splice(toIndex, 0, moved);

    const groupPlatformSet = new Set(targetGroup.platformIds);
    const nextOrderedPlatformIds = [...get().orderedPlatformIds];
    const groupPlatformPositions: number[] = [];
    nextOrderedPlatformIds.forEach((platformId, index) => {
      if (groupPlatformSet.has(platformId)) {
        groupPlatformPositions.push(index);
      }
    });

    if (groupPlatformPositions.length !== nextGroupPlatformIds.length) {
      return;
    }

    groupPlatformPositions.forEach((position, index) => {
      nextOrderedPlatformIds[position] = nextGroupPlatformIds[index];
    });

    const mergedGroups = currentGroups.map((group) => {
      if (group.id !== groupId) {
        return group;
      }
      return {
        ...group,
        platformIds: nextGroupPlatformIds,
        defaultPlatformId: nextGroupPlatformIds.includes(group.defaultPlatformId)
          ? group.defaultPlatformId
          : nextGroupPlatformIds[0],
        iconPlatformId: nextGroupPlatformIds.includes(group.iconPlatformId as PlatformId)
          ? group.iconPlatformId
          : nextGroupPlatformIds[0],
      };
    });

    const normalizedGroups = normalizePlatformGroups(mergedGroups, false)
      .map((group) => sortGroupPlatformsByOrder(group, nextOrderedPlatformIds));

    const orderedEntryIds = normalizeEntryOrder(
      get().orderedEntryIds,
      normalizedGroups,
      nextOrderedPlatformIds,
    );

    const orderedPlatformIds = derivePlatformOrderFromEntryOrder(
      orderedEntryIds,
      normalizedGroups,
      nextOrderedPlatformIds,
    );

    const next = normalizeStateData({
      orderedPlatformIds,
      hiddenPlatformIds: get().hiddenPlatformIds,
      sidebarPlatformIds: get().sidebarPlatformIds,
      trayPlatformIds: get().trayPlatformIds,
      traySortMode: 'manual',
      platformGroups: normalizedGroups,
      orderedEntryIds,
      hiddenEntryIds: get().hiddenEntryIds,
      sidebarEntryIds: get().sidebarEntryIds,
      apiRelaySidebarVisible: get().apiRelaySidebarVisible,
      apiRelayDashboardVisible: get().apiRelayDashboardVisible,
      apiRelayEntryOrder: get().apiRelayEntryOrder,
    });

    set(next);
    persist(next);
    scheduleTrayLayoutSync(next);
  },

  toggleHiddenEntry: (id) => {
    const current = [...get().hiddenEntryIds];
    const exists = current.includes(id);
    const nextHidden = exists ? current.filter((item) => item !== id) : [...current, id];

    const next = normalizeStateData({
      orderedPlatformIds: get().orderedPlatformIds,
      hiddenPlatformIds: get().hiddenPlatformIds,
      sidebarPlatformIds: get().sidebarPlatformIds,
      trayPlatformIds: get().trayPlatformIds,
      traySortMode: get().traySortMode,
      platformGroups: get().platformGroups,
      orderedEntryIds: get().orderedEntryIds,
      hiddenEntryIds: nextHidden,
      sidebarEntryIds: get().sidebarEntryIds,
      apiRelaySidebarVisible: get().apiRelaySidebarVisible,
      apiRelayDashboardVisible: get().apiRelayDashboardVisible,
      apiRelayEntryOrder: get().apiRelayEntryOrder,
    });

    set(next);
    persist(next);
  },

  setHiddenEntry: (id, hidden) => {
    const has = get().hiddenEntryIds.includes(id);
    if ((hidden && has) || (!hidden && !has)) return;
    get().toggleHiddenEntry(id);
  },

  toggleSidebarEntry: (id) => {
    const current = [...get().sidebarEntryIds];
    let nextSidebar: PlatformLayoutEntryId[] = [];

    if (current.includes(id)) {
      nextSidebar = current.filter((item) => item !== id);
    } else {
      nextSidebar = [...current, id];
    }

    const next = normalizeStateData({
      orderedPlatformIds: get().orderedPlatformIds,
      hiddenPlatformIds: get().hiddenPlatformIds,
      sidebarPlatformIds: get().sidebarPlatformIds,
      trayPlatformIds: get().trayPlatformIds,
      traySortMode: get().traySortMode,
      platformGroups: get().platformGroups,
      orderedEntryIds: get().orderedEntryIds,
      hiddenEntryIds: get().hiddenEntryIds,
      sidebarEntryIds: nextSidebar,
      apiRelaySidebarVisible: get().apiRelaySidebarVisible,
      apiRelayDashboardVisible: get().apiRelayDashboardVisible,
      apiRelayEntryOrder: get().apiRelayEntryOrder,
    });

    set(next);
    persist(next);
  },

  setSidebarEntry: (id, enabled) => {
    const has = get().sidebarEntryIds.includes(id);
    if ((enabled && has) || (!enabled && !has)) return;
    get().toggleSidebarEntry(id);
  },

  setApiRelaySidebarVisible: (visible) => {
    if (get().apiRelaySidebarVisible === visible) {
      return;
    }
    const next = {
      ...get(),
      apiRelaySidebarVisible: visible,
    };
    set({ apiRelaySidebarVisible: visible });
    persist(next);
  },

  setApiRelayDashboardVisible: (visible) => {
    if (get().apiRelayDashboardVisible === visible) {
      return;
    }
    const next = {
      ...get(),
      apiRelayDashboardVisible: visible,
    };
    set({ apiRelayDashboardVisible: visible });
    persist(next);
  },

  setApiRelayEntryOrder: (order) => {
    const nextOrder = normalizeApiRelayEntryOrder(order, get().orderedEntryIds.length);
    if (get().apiRelayEntryOrder === nextOrder) {
      return;
    }
    const next = {
      ...get(),
      apiRelayEntryOrder: nextOrder,
    };
    set({ apiRelayEntryOrder: nextOrder });
    persist(next);
  },

  syncSidebarEntriesFromDashboard: () => {
    const hiddenSet = new Set(get().hiddenEntryIds);
    const nextSidebarEntries = get().orderedEntryIds.filter((entryId) => !hiddenSet.has(entryId));
    const currentSidebarEntries = get().sidebarEntryIds;
    if (
      currentSidebarEntries.length === nextSidebarEntries.length
      && currentSidebarEntries.every((entryId, index) => entryId === nextSidebarEntries[index])
    ) {
      return;
    }

    const next = normalizeStateData({
      orderedPlatformIds: get().orderedPlatformIds,
      hiddenPlatformIds: get().hiddenPlatformIds,
      sidebarPlatformIds: get().sidebarPlatformIds,
      trayPlatformIds: get().trayPlatformIds,
      traySortMode: get().traySortMode,
      platformGroups: get().platformGroups,
      orderedEntryIds: get().orderedEntryIds,
      hiddenEntryIds: get().hiddenEntryIds,
      sidebarEntryIds: nextSidebarEntries,
      apiRelaySidebarVisible: get().apiRelaySidebarVisible,
      apiRelayDashboardVisible: get().apiRelayDashboardVisible,
      apiRelayEntryOrder: get().apiRelayEntryOrder,
    });

    set(next);
    persist(next);
  },

  upsertPlatformGroup: (group) => {
    const currentGroups = [...get().platformGroups];
    const hasExisting = currentGroups.some((item) => item.id === group.id);
    const merged = hasExisting
      ? currentGroups.map((item) => (item.id === group.id ? { ...item, ...group } : item))
      : [...currentGroups, group];

    const targetPlatformSet = new Set(group.platformIds);
    const redistributed = merged.flatMap((item) => {
      if (item.id === group.id) {
        return [item];
      }
      const retainedPlatformIds = item.platformIds.filter((platformId) => !targetPlatformSet.has(platformId));
      if (retainedPlatformIds.length === 0) {
        return [];
      }
      return [{
        ...item,
        platformIds: retainedPlatformIds,
        defaultPlatformId: retainedPlatformIds.includes(item.defaultPlatformId)
          ? item.defaultPlatformId
          : retainedPlatformIds[0],
        iconPlatformId: retainedPlatformIds.includes(item.iconPlatformId as PlatformId)
          ? item.iconPlatformId
          : retainedPlatformIds[0],
        childConfigs: (item.childConfigs ?? []).filter((child) => retainedPlatformIds.includes(child.platformId)),
      }];
    });

    const normalizedGroups = normalizePlatformGroups(redistributed, false)
      .map((item) => sortGroupPlatformsByOrder(item, get().orderedPlatformIds));

    const orderedEntryIds = normalizeEntryOrder(
      get().orderedEntryIds,
      normalizedGroups,
      get().orderedPlatformIds,
    );
    const orderedPlatformIds = derivePlatformOrderFromEntryOrder(
      orderedEntryIds,
      normalizedGroups,
      get().orderedPlatformIds,
    );

    const next = normalizeStateData({
      orderedPlatformIds,
      hiddenPlatformIds: get().hiddenPlatformIds,
      sidebarPlatformIds: get().sidebarPlatformIds,
      trayPlatformIds: get().trayPlatformIds,
      traySortMode: get().traySortMode,
      platformGroups: normalizedGroups,
      orderedEntryIds,
      hiddenEntryIds: get().hiddenEntryIds,
      sidebarEntryIds: get().sidebarEntryIds,
      apiRelaySidebarVisible: get().apiRelaySidebarVisible,
      apiRelayDashboardVisible: get().apiRelayDashboardVisible,
      apiRelayEntryOrder: get().apiRelayEntryOrder,
    });

    set(next);
    persist(next);
    scheduleTrayLayoutSync(next);
  },

  removePlatformGroup: (groupId) => {
    const nextGroups = get().platformGroups.filter((group) => group.id !== groupId);
    const orderedEntryIds = normalizeEntryOrder(
      get().orderedEntryIds,
      nextGroups,
      get().orderedPlatformIds,
    );
    const orderedPlatformIds = derivePlatformOrderFromEntryOrder(
      orderedEntryIds,
      nextGroups,
      get().orderedPlatformIds,
    );

    const next = normalizeStateData({
      orderedPlatformIds,
      hiddenPlatformIds: get().hiddenPlatformIds,
      sidebarPlatformIds: get().sidebarPlatformIds,
      trayPlatformIds: get().trayPlatformIds,
      traySortMode: get().traySortMode,
      platformGroups: nextGroups,
      orderedEntryIds,
      hiddenEntryIds: get().hiddenEntryIds,
      sidebarEntryIds: get().sidebarEntryIds,
      apiRelaySidebarVisible: get().apiRelaySidebarVisible,
      apiRelayDashboardVisible: get().apiRelayDashboardVisible,
      apiRelayEntryOrder: get().apiRelayEntryOrder,
    });

    set(next);
    persist(next);
    scheduleTrayLayoutSync(next);
  },

  toggleTrayPlatform: (id) => {
    const current = [...get().trayPlatformIds];
    const exists = current.includes(id);
    const nextTray = exists
      ? current.filter((item) => item !== id)
      : [...current, id];

    const next = normalizeStateData({
      orderedPlatformIds: get().orderedPlatformIds,
      hiddenPlatformIds: get().hiddenPlatformIds,
      sidebarPlatformIds: get().sidebarPlatformIds,
      trayPlatformIds: normalizeTray(nextTray),
      traySortMode: get().traySortMode,
      platformGroups: get().platformGroups,
      orderedEntryIds: get().orderedEntryIds,
      hiddenEntryIds: get().hiddenEntryIds,
      sidebarEntryIds: get().sidebarEntryIds,
      apiRelaySidebarVisible: get().apiRelaySidebarVisible,
      apiRelayDashboardVisible: get().apiRelayDashboardVisible,
      apiRelayEntryOrder: get().apiRelayEntryOrder,
    });

    set(next);
    persist(next);
    scheduleTrayLayoutSync(next);
  },

  setTrayPlatform: (id, enabled) => {
    const current = get().trayPlatformIds.includes(id);
    if (current === enabled) return;
    get().toggleTrayPlatform(id);
  },

  syncTrayLayout: () => {
    const state = get();
    syncTrayLayoutToBackend({
      orderedPlatformIds: state.orderedPlatformIds,
      trayPlatformIds: state.trayPlatformIds,
      traySortMode: state.traySortMode,
      orderedEntryIds: state.orderedEntryIds,
      platformGroups: state.platformGroups,
    });
  },

  resetPlatformLayout: () => {
    const defaults = defaultPlatformGroups();
    const defaultOrder = defaultPlatformOrder();
    const next = normalizeStateData({
      orderedPlatformIds: defaultOrder,
      hiddenPlatformIds: [],
      sidebarPlatformIds: defaultSidebarPlatformIds(),
      trayPlatformIds: defaultOrder,
      traySortMode: 'auto',
      platformGroups: defaults,
      orderedEntryIds: buildEntryOrderFromPlatformOrder(defaultOrder, defaults),
      hiddenEntryIds: [],
      sidebarEntryIds: defaultSidebarEntryIds(defaults),
      apiRelaySidebarVisible: true,
      apiRelayDashboardVisible: true,
      apiRelayEntryOrder: 0,
    });

    set(next);
    persist(next);
    scheduleTrayLayoutSync(next);
  },
}));

if (typeof window !== 'undefined') {
  window.setTimeout(() => {
    usePlatformLayoutStore.getState().syncTrayLayout();
  }, 0);
}
