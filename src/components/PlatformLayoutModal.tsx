import { hydrateDurableUiSettings } from '../utils/durableUiSettings';
import {
  ChangeEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { arrayMove } from '@dnd-kit/sortable';
import { SortableLayoutItem, SortableLayoutList } from './SortableLayoutList';
import { SingleSelectDropdown } from './SingleSelectDropdown';
import { useTranslation } from 'react-i18next';
import {
  Check,
  ChevronDown,
  ChevronRight,
  ChevronsDownUp,
  ChevronsUpDown,
  GripVertical,
  Pencil,
  Plus,
  Trash2,
  Upload,
  X,
} from 'lucide-react';
import apiKeyFunIcon from '../assets/icons/apikey-fun.png';
import { isMenuVisiblePlatform, MENU_VISIBLE_PLATFORM_IDS, PlatformId } from '../types/platform';
import { useSponsorStore } from '../stores/useSponsorStore';
import {
  API_RELAY_LAYOUT_ENTRY_ID,
  ApiRelayLayoutEntryId,
  getGroupChildConfig,
  parseGroupEntryId,
  parsePlatformEntryId,
  PlatformGroupIconKind,
  PlatformLayoutEntryId,
  PlatformLayoutGroup,
  PlatformLayoutGroupChildConfig,
  resolveEntryDefaultPlatformId,
  resolveEntryPlatformIds,
  resolveGroupChildIcon,
  resolveGroupChildName,
  makePlatformEntryId,
  usePlatformLayoutStore,
} from '../stores/usePlatformLayoutStore';
import { getPlatformLabel, renderPlatformIcon } from '../utils/platformMeta';
import { useEscClose } from '../hooks/useEscClose';
import { scrollElementIntoView } from '../utils/reducedMotion';
import {
  getPlatformLayoutPersistenceError,
  retryPlatformLayoutPersistence,
} from '../utils/uiPreferences';
import {
  PLATFORM_LAYOUT_ICONS_CHANGED_EVENT,
  readPlatformLayoutCustomIcons,
  persistPlatformLayoutCustomIcons,
} from '../utils/platformLayoutIconStorage';

interface PlatformLayoutModalProps {
  open: boolean;
  requestedEditGroupId?: string | null;
  onClose: () => void;
}

type LayoutEntryId = PlatformLayoutEntryId | ApiRelayLayoutEntryId;

interface LayoutEntryItem {
  id: LayoutEntryId;
  type: 'platform' | 'group' | 'api-relay';
  label: string;
  hidden: boolean;
  group: PlatformLayoutGroup | null;
  defaultPlatformId: PlatformId | null;
  platformIds: PlatformId[];
}

interface PlatformLayoutCustomIcon {
  id: string;
  name: string;
  dataUrl: string;
  createdAt: number;
}

interface IconSelectorProps {
  customIcons: PlatformLayoutCustomIcon[];
  iconKind: PlatformGroupIconKind;
  iconPlatformId: PlatformId;
  iconCustomDataUrl?: string;
  onSelectPlatform: (platformId: PlatformId) => void;
  onSelectCustom: (dataUrl: string) => void;
  onUploadCustom: (dataUrl: string, fileName?: string) => void;
  onDeleteCustom: (iconId: string) => void;
}

function createCustomIconId() {
  return `custom-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function normalizeCustomIconName(fileName?: string) {
  if (!fileName) {
    return `Custom ${new Date().toLocaleDateString()}`;
  }
  const trimmed = fileName.trim();
  if (!trimmed) {
    return `Custom ${new Date().toLocaleDateString()}`;
  }
  return trimmed;
}

function isPlatformLayoutEntryId(id: LayoutEntryId): id is PlatformLayoutEntryId {
  return id !== API_RELAY_LAYOUT_ENTRY_ID;
}

function loadCustomIcons(): PlatformLayoutCustomIcon[] {
  if (typeof window === 'undefined') {
    return [];
  }
  try {
    const raw = readPlatformLayoutCustomIcons();
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw) as PlatformLayoutCustomIcon[];
    if (!Array.isArray(parsed)) {
      return [];
    }
    const dedup = new Map<string, PlatformLayoutCustomIcon>();
    for (const item of parsed) {
      if (!item || typeof item !== 'object' || typeof item.dataUrl !== 'string') {
        continue;
      }
      const dataUrl = item.dataUrl.trim();
      if (!dataUrl) {
        continue;
      }
      if (dedup.has(dataUrl)) {
        continue;
      }
      dedup.set(dataUrl, {
        id: typeof item.id === 'string' && item.id.trim() ? item.id : createCustomIconId(),
        name:
          typeof item.name === 'string' && item.name.trim()
            ? item.name.trim()
            : normalizeCustomIconName(),
        dataUrl,
        createdAt: typeof item.createdAt === 'number' ? item.createdAt : Date.now(),
      });
    }
    return Array.from(dedup.values());
  } catch {
    return [];
  }
}

function renderGroupIcon(group: PlatformLayoutGroup, size: number) {
  if (group.iconKind === 'custom' && group.iconCustomDataUrl) {
    return (
      <img
        src={group.iconCustomDataUrl}
        alt={group.name}
        className="platform-layout-group-icon"
        style={{ width: size, height: size }}
      />
    );
  }
  return renderPlatformIcon(group.iconPlatformId ?? group.defaultPlatformId, size);
}

function createGroupId(name: string, existingIds: string[]): string {
  const base =
    name
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9_-]+/g, '-')
      .replace(/^-+|-+$/g, '') || 'group';

  let candidate = base;
  let index = 1;
  while (existingIds.includes(candidate)) {
    index += 1;
    candidate = `${base}-${index}`;
  }
  return candidate;
}

function IconSelector({
  customIcons,
  iconKind,
  iconPlatformId,
  iconCustomDataUrl,
  onSelectPlatform,
  onSelectCustom,
  onUploadCustom,
  onDeleteCustom,
}: IconSelectorProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const selectedCustom = useMemo(
    () => customIcons.find((item) => item.dataUrl === iconCustomDataUrl),
    [customIcons, iconCustomDataUrl],
  );

  useEffect(() => {
    if (!open) {
      return;
    }
    const handleMouseDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (rootRef.current?.contains(target)) {
        return;
      }
      setOpen(false);
    };
    document.addEventListener('mousedown', handleMouseDown);
    return () => document.removeEventListener('mousedown', handleMouseDown);
  }, [open]);

  const currentLabel =
    iconKind === 'custom'
      ? selectedCustom?.name ?? t('platformLayout.groupIconCustom', '自定义图片')
      : getPlatformLabel(iconPlatformId, t);

  return (
    <div className="platform-layout-icon-select" ref={rootRef}>
      <button
        type="button"
        className="platform-layout-icon-select-trigger"
        onClick={() => setOpen((prev) => !prev)}
      >
        <span className="platform-layout-icon-select-trigger-icon">
          {iconKind === 'custom' && iconCustomDataUrl ? (
            <img
              src={iconCustomDataUrl}
              alt={currentLabel}
              className="platform-layout-group-icon"
              style={{ width: 16, height: 16 }}
            />
          ) : (
            renderPlatformIcon(iconPlatformId, 16)
          )}
        </span>
        <span className="platform-layout-icon-select-trigger-label">{currentLabel}</span>
        <ChevronDown size={14} />
      </button>

      {open && (
        <div className="platform-layout-icon-select-menu">
          <input
            ref={fileInputRef}
            type="file"
            accept="image/*"
            style={{ display: 'none' }}
            onChange={(event: ChangeEvent<HTMLInputElement>) => {
              const file = event.target.files?.[0];
              if (!file) {
                return;
              }
              const reader = new FileReader();
              reader.onload = () => {
                if (typeof reader.result === 'string') {
                  onUploadCustom(reader.result, file.name);
                  onSelectCustom(reader.result);
                  setOpen(false);
                }
              };
              reader.readAsDataURL(file);
              event.target.value = '';
            }}
          />

          <button
            type="button"
            className="platform-layout-icon-select-item"
            onClick={() => {
              fileInputRef.current?.click();
            }}
          >
            <span className="platform-layout-icon-select-item-icon">
              <Upload size={14} />
            </span>
            <span>
              {t('platformLayout.groupIconCustom', '自定义图片')}
              {' · '}
              {t('common.add', '添加')}
            </span>
          </button>

          <div className="platform-layout-icon-select-divider" />

          {MENU_VISIBLE_PLATFORM_IDS.map((platformId) => (
            <button
              type="button"
              className={`platform-layout-icon-select-item ${
                iconKind === 'platform' && iconPlatformId === platformId ? 'is-active' : ''
              }`}
              key={`icon-platform-${platformId}`}
              onClick={() => {
                onSelectPlatform(platformId);
                setOpen(false);
              }}
            >
              <span className="platform-layout-icon-select-item-icon">{renderPlatformIcon(platformId, 14)}</span>
              <span>{getPlatformLabel(platformId, t)}</span>
              {iconKind === 'platform' && iconPlatformId === platformId && <Check size={12} />}
            </button>
          ))}

          {customIcons.length > 0 && (
            <>
              <div className="platform-layout-icon-select-divider" />
              {customIcons.map((icon) => {
                const active = iconKind === 'custom' && icon.dataUrl === iconCustomDataUrl;
                return (
                  <div
                    className={`platform-layout-icon-select-item-wrap ${active ? 'is-active' : ''}`}
                    key={icon.id}
                  >
                    <button
                      type="button"
                      className="platform-layout-icon-select-item"
                      onClick={() => {
                        onSelectCustom(icon.dataUrl);
                        setOpen(false);
                      }}
                    >
                      <span className="platform-layout-icon-select-item-icon">
                        <img
                          src={icon.dataUrl}
                          alt={icon.name}
                          className="platform-layout-group-icon"
                          style={{ width: 14, height: 14 }}
                        />
                      </span>
                      <span>{icon.name}</span>
                      {active && <Check size={12} />}
                    </button>
                    <button
                      type="button"
                      className="platform-layout-icon-select-delete"
                      onClick={(event) => {
                        event.stopPropagation();
                        onDeleteCustom(icon.id);
                      }}
                      aria-label={t('common.delete', '删除')}
                    >
                      <X size={12} />
                    </button>
                  </div>
                );
              })}
            </>
          )}
        </div>
      )}
    </div>
  );
}

export function PlatformLayoutModal({
  open,
  requestedEditGroupId = null,
  onClose,
}: PlatformLayoutModalProps) {
  const { t } = useTranslation();
  const [sorting, setSorting] = useState(false);
  useEscClose(open && !sorting, onClose);
  const {
    orderedEntryIds,
    hiddenEntryIds,
    sidebarEntryIds,
    trayPlatformIds,
    platformGroups,
    apiRelaySidebarVisible,
    apiRelayDashboardVisible,
    apiRelayEntryOrder,
    disabledPlatformIds,
    reorderGroupPlatforms,
    setLayoutEntryOrder,
    setHiddenEntry,
    setDisabledPlatform,
    setSidebarEntry,
    setTrayPlatform,
    setApiRelaySidebarVisible,
    setApiRelayDashboardVisible,
    upsertPlatformGroup,
    removePlatformGroup,
    resetPlatformLayout,
  } = usePlatformLayoutStore();
  const apiRelayEntryEnabled = useSponsorStore((state) => Boolean(state.state.sponsorModule));

  const [customIcons, setCustomIcons] = useState<PlatformLayoutCustomIcon[]>(() => loadCustomIcons());
  const [customIconStorageFailed, setCustomIconStorageFailed] = useState(false);
  const [persistenceError, setPersistenceError] = useState(getPlatformLayoutPersistenceError);
  const [retryingPersistence, setRetryingPersistence] = useState(false);
  const persistenceErrorRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const refresh = () => setPersistenceError(getPlatformLayoutPersistenceError());
    window.addEventListener('agtools:platform-layout-persistence-changed', refresh);
    refresh();
    return () => window.removeEventListener('agtools:platform-layout-persistence-changed', refresh);
  }, []);

  useEffect(() => {
    if (open && persistenceError) {
      scrollElementIntoView(persistenceErrorRef.current, { block: 'nearest' });
    }
  }, [open, persistenceError]);

  const retryPersistence = async () => {
    setPersistenceError(null);
    setRetryingPersistence(true);
    try {
      await retryPlatformLayoutPersistence();
    } finally {
      setPersistenceError(getPlatformLayoutPersistenceError());
      setRetryingPersistence(false);
    }
  };

  const [expandedGroupIds, setExpandedGroupIds] = useState<string[]>([]);
  const [addingChildGroupId, setAddingChildGroupId] = useState<string | null>(null);
  const [pendingAddChildIds, setPendingAddChildIds] = useState<PlatformId[]>([]);

  const [groupEditorOpen, setGroupEditorOpen] = useState(false);
  const [editingGroupId, setEditingGroupId] = useState<string | null>(null);
  const [groupDraftName, setGroupDraftName] = useState('');
  const [groupDraftPlatformIds, setGroupDraftPlatformIds] = useState<PlatformId[]>([]);
  const [groupDraftDefaultPlatformId, setGroupDraftDefaultPlatformId] = useState<PlatformId | ''>('');
  const [groupDraftIconKind, setGroupDraftIconKind] = useState<PlatformGroupIconKind>('platform');
  const [groupDraftIconPlatformId, setGroupDraftIconPlatformId] = useState<PlatformId>('codebuddy');
  const [groupDraftIconCustomDataUrl, setGroupDraftIconCustomDataUrl] = useState('');
  const [groupDraftError, setGroupDraftError] = useState('');
  const [handledRequestedEditGroupId, setHandledRequestedEditGroupId] = useState<string | null>(null);

  const [childEditorOpen, setChildEditorOpen] = useState(false);
  const [childEditorGroupId, setChildEditorGroupId] = useState<string | null>(null);
  const [childEditorPlatformId, setChildEditorPlatformId] = useState<PlatformId | null>(null);
  const [childDraftName, setChildDraftName] = useState('');
  const [childDraftIconKind, setChildDraftIconKind] = useState<PlatformGroupIconKind>('platform');
  const [childDraftIconPlatformId, setChildDraftIconPlatformId] = useState<PlatformId>('codebuddy');
  const [childDraftIconCustomDataUrl, setChildDraftIconCustomDataUrl] = useState('');
  const [childDraftSetDefault, setChildDraftSetDefault] = useState(false);
  const [childDraftError, setChildDraftError] = useState('');
  const [confirmAction, setConfirmAction] = useState<
    | { kind: 'reset' }
    | { kind: 'deleteGroup'; groupId: string; groupName: string }
    | null
  >(null);

  const hiddenSet = useMemo(() => new Set(hiddenEntryIds), [hiddenEntryIds]);
  const sidebarSet = useMemo(() => new Set(sidebarEntryIds), [sidebarEntryIds]);
  const traySet = useMemo(() => new Set(trayPlatformIds), [trayPlatformIds]);

  const entries = useMemo<LayoutEntryItem[]>(() => {
    const result: LayoutEntryItem[] = [];
    for (const entryId of orderedEntryIds) {
      const platformId = parsePlatformEntryId(entryId);
      if (platformId) {
        if (!isMenuVisiblePlatform(platformId)) {
          continue;
        }
        result.push({
          id: entryId,
          type: 'platform',
          label: getPlatformLabel(platformId, t),
          hidden: hiddenSet.has(entryId),
          group: null,
          defaultPlatformId: platformId,
          platformIds: [platformId],
        });
        continue;
      }

      const groupId = parseGroupEntryId(entryId);
      if (!groupId) {
        continue;
      }
      const group = platformGroups.find((item) => item.id === groupId);
      if (!group) {
        continue;
      }
      const defaultPlatformId = resolveEntryDefaultPlatformId(entryId, platformGroups);
      if (!defaultPlatformId) {
        continue;
      }

      const visiblePlatformIds = resolveEntryPlatformIds(entryId, platformGroups).filter(isMenuVisiblePlatform);
      if (visiblePlatformIds.length === 0) {
        continue;
      }

      result.push({
        id: entryId,
        type: 'group',
        label: group.name,
        hidden: hiddenSet.has(entryId),
        group,
        defaultPlatformId: visiblePlatformIds.includes(defaultPlatformId)
          ? defaultPlatformId
          : visiblePlatformIds[0],
        platformIds: visiblePlatformIds,
      });
    }

    if (apiRelayEntryEnabled) {
      const insertIndex = Math.max(0, Math.min(apiRelayEntryOrder, result.length));
      result.splice(insertIndex, 0, {
        id: API_RELAY_LAYOUT_ENTRY_ID,
        type: 'api-relay',
        label: t('nav.apiRelay', '中转站'),
        hidden: !apiRelayDashboardVisible,
        group: null,
        defaultPlatformId: null,
        platformIds: [],
      });
    }

    return result;
  }, [
    orderedEntryIds,
    platformGroups,
    hiddenSet,
    t,
    apiRelayEntryEnabled,
    apiRelayEntryOrder,
    apiRelayDashboardVisible,
  ]);

  const layoutEntryOrderIds = useMemo<LayoutEntryId[]>(() => {
    const result: LayoutEntryId[] = [...orderedEntryIds];
    if (!apiRelayEntryEnabled) {
      return result;
    }
    const insertIndex = Math.max(0, Math.min(apiRelayEntryOrder, result.length));
    result.splice(insertIndex, 0, API_RELAY_LAYOUT_ENTRY_ID);
    return result;
  }, [apiRelayEntryEnabled, apiRelayEntryOrder, orderedEntryIds]);

  const allGroupIds = useMemo(
    () => entries.filter((entry) => entry.type === 'group' && !!entry.group).map((entry) => entry.id),
    [entries],
  );

  const allGroupExpanded = allGroupIds.length > 0 && allGroupIds.every((id) => expandedGroupIds.includes(id));

  const availableAddChildByGroup = useMemo(() => {
    const map = new Map<string, PlatformId[]>();
    for (const group of platformGroups) {
      const list = MENU_VISIBLE_PLATFORM_IDS.filter((platformId) => !group.platformIds.includes(platformId));
      map.set(group.id, list);
    }
    return map;
  }, [platformGroups]);

  const iconSaveSequence = useRef(0);
  const changeCustomIcons = useCallback(async (
    edit: (previous: PlatformLayoutCustomIcon[]) => PlatformLayoutCustomIcon[],
  ) => {
    const sequence = ++iconSaveSequence.current;
    let saved = false;
    try {
      await hydrateDurableUiSettings();
      const previous = loadCustomIcons();
      const next = edit(previous);
      saved = next === previous || await persistPlatformLayoutCustomIcons(next);
    } catch (error) {
      console.warn('[Platform layout] Could not load custom icons:', error);
    }
    if (sequence === iconSaveSequence.current) setCustomIconStorageFailed(!saved);
    return saved;
  }, []);

  useEffect(() => {
    const reload = () => setCustomIcons(loadCustomIcons());
    window.addEventListener(PLATFORM_LAYOUT_ICONS_CHANGED_EVENT, reload);
    reload();
    return () => window.removeEventListener(PLATFORM_LAYOUT_ICONS_CHANGED_EVENT, reload);
  }, []);

  useEffect(() => {
    if (!open) return;
    const used = new Set<string>();
    for (const group of platformGroups) {
      if (group.iconKind === 'custom' && group.iconCustomDataUrl?.trim()) {
        used.add(group.iconCustomDataUrl.trim());
      }
      for (const child of group.childConfigs ?? []) {
        if (child.iconKind === 'custom' && child.iconCustomDataUrl?.trim()) {
          used.add(child.iconCustomDataUrl.trim());
        }
      }
    }

    if (used.size === 0) {
      return;
    }

    void changeCustomIcons((prev) => {
      const next = [...prev];
      let changed = false;
      for (const dataUrl of used) {
        if (next.some((item) => item.dataUrl === dataUrl)) {
          continue;
        }
        changed = true;
        next.push({
          id: createCustomIconId(),
          name: normalizeCustomIconName(),
          dataUrl,
          createdAt: Date.now(),
        });
      }
      return changed ? next : prev;
    });
  }, [open, platformGroups, changeCustomIcons]);

  useEffect(() => {
    if (!open) {
      setGroupEditorOpen(false);
      setChildEditorOpen(false);
      setEditingGroupId(null);
      setChildEditorGroupId(null);
      setChildEditorPlatformId(null);
      setAddingChildGroupId(null);
      setPendingAddChildIds([]);
      setGroupDraftError('');
      setChildDraftError('');
      setSorting(false);
      setHandledRequestedEditGroupId(null);
    }
  }, [open]);

  useEffect(() => {
    if (groupDraftPlatformIds.length === 0) {
      setGroupDraftDefaultPlatformId('');
      return;
    }
    if (!groupDraftDefaultPlatformId || !groupDraftPlatformIds.includes(groupDraftDefaultPlatformId)) {
      setGroupDraftDefaultPlatformId(groupDraftPlatformIds[0]);
    }
  }, [groupDraftPlatformIds, groupDraftDefaultPlatformId]);

  const upsertCustomIcon = (dataUrl: string, fileName?: string) => {
    const cleaned = dataUrl.trim();
    if (!cleaned) {
      return;
    }
    void changeCustomIcons((prev) => {
      if (prev.some((item) => item.dataUrl === cleaned)) {
        return prev;
      }
      return [
        ...prev,
        {
          id: createCustomIconId(),
          name: normalizeCustomIconName(fileName),
          dataUrl: cleaned,
          createdAt: Date.now(),
        },
      ];
    });
  };

  const clearCustomIconUsage = (dataUrl: string) => {
    for (const group of platformGroups) {
      let changed = false;
      let nextGroup: PlatformLayoutGroup = group;

      if (group.iconKind === 'custom' && group.iconCustomDataUrl === dataUrl) {
        changed = true;
        nextGroup = {
          ...nextGroup,
          iconKind: 'platform',
          iconPlatformId: nextGroup.iconPlatformId ?? nextGroup.defaultPlatformId,
          iconCustomDataUrl: undefined,
        };
      }

      const currentChildConfigs = nextGroup.childConfigs ?? [];
      const nextChildConfigs: PlatformLayoutGroupChildConfig[] = currentChildConfigs.map((child) => {
        if (child.iconKind !== 'custom' || child.iconCustomDataUrl !== dataUrl) {
          return child;
        }
        changed = true;
        return {
          ...child,
          iconKind: 'platform' as PlatformGroupIconKind,
          iconPlatformId: child.iconPlatformId ?? child.platformId,
          iconCustomDataUrl: undefined,
        };
      });

      if (!changed) {
        continue;
      }

      upsertPlatformGroup({
        ...nextGroup,
        childConfigs: nextChildConfigs,
      });
    }
  };

  const removeCustomIcon = async (iconId: string) => {
    const icon = customIcons.find((item) => item.id === iconId);
    if (!icon) {
      return;
    }

    const confirmed = window.confirm(
      t('platformLayout.customIconDeleteConfirm', '确认删除这个自定义图标吗？'),
    );
    if (!confirmed) {
      return;
    }

    if (!await changeCustomIcons(prev => prev.filter(item => item.id !== iconId))) return;
    clearCustomIconUsage(icon.dataUrl);

    if (groupDraftIconKind === 'custom' && groupDraftIconCustomDataUrl === icon.dataUrl) {
      setGroupDraftIconKind('platform');
      setGroupDraftIconCustomDataUrl('');
    }
    if (childDraftIconKind === 'custom' && childDraftIconCustomDataUrl === icon.dataUrl) {
      setChildDraftIconKind('platform');
      setChildDraftIconCustomDataUrl('');
    }
  };

  const commitLayoutEntryOrder = (nextIds: LayoutEntryId[]) => {
    const nextApiRelayOrder = nextIds.indexOf(API_RELAY_LAYOUT_ENTRY_ID);
    setLayoutEntryOrder(
      nextIds.filter(isPlatformLayoutEntryId),
      nextApiRelayOrder >= 0 ? nextApiRelayOrder : apiRelayEntryOrder,
    );
  };

  const toggleGroupExpanded = (groupId: string) => {
    setExpandedGroupIds((prev) =>
      prev.includes(groupId) ? prev.filter((id) => id !== groupId) : [...prev, groupId],
    );
  };

  const toggleExpandAll = () => {
    if (allGroupExpanded) {
      setExpandedGroupIds([]);
      return;
    }
    setExpandedGroupIds(allGroupIds);
  };

  const sidebarCandidates = entries.flatMap((entry) => [
    entry.id,
    ...(entry.group ? entry.platformIds.map(makePlatformEntryId) : []),
  ]);
  const isSidebarEntrySelected = (id: LayoutEntryItem['id']) =>
    id === API_RELAY_LAYOUT_ENTRY_ID ? apiRelaySidebarVisible : sidebarSet.has(id);

  const isDashboardEntryVisible = (entry: LayoutEntryItem) =>
    entry.type === 'api-relay' ? apiRelayDashboardVisible : !entry.hidden;

  const sidebarSelectedCount = sidebarCandidates.filter(isSidebarEntrySelected).length;
  const sidebarBulkEnabled = entries.length > 0
    && entries.every(({ id }) => isSidebarEntrySelected(id));

  const setSidebarSelection = (entry: LayoutEntryItem, enabled: boolean) => {
    if (entry.type === 'api-relay') {
      setApiRelaySidebarVisible(enabled);
    } else {
      setSidebarEntry(entry.id as PlatformLayoutEntryId, enabled);
    }
  };

  const handleBulkSidebar = (enabled: boolean) => {
    if (enabled) {
      entries.filter(({ id }) => !isSidebarEntrySelected(id))
        .forEach((entry) => setSidebarSelection(entry, true));
      return;
    }

    sidebarCandidates.filter(isSidebarEntrySelected).forEach((id) => {
      if (id === API_RELAY_LAYOUT_ENTRY_ID) {
        setApiRelaySidebarVisible(false);
      } else {
        setSidebarEntry(id, false);
      }
    });
  };

  const handleBulkDashboard = (enabled: boolean) => {
    entries.forEach((entry) => {
      if (entry.type === 'api-relay') {
        setApiRelayDashboardVisible(enabled);
        return;
      }
      setHiddenEntry(entry.id as PlatformLayoutEntryId, !enabled);
    });
  };

  const handleBulkTray = (enabled: boolean) => {
    MENU_VISIBLE_PLATFORM_IDS.forEach((platformId) => setTrayPlatform(platformId, enabled));
  };

  const dashboardBulkEnabled = entries.length > 0
    && entries.every(isDashboardEntryVisible);
  const trayBulkEnabled = MENU_VISIBLE_PLATFORM_IDS.every((platformId) => traySet.has(platformId));

  // store 里存的是裸平台 ID，这里的 entry.id 带 `platform:` 前缀，需要对齐形态
  const disabledSet = useMemo(
    () => new Set<string>(disabledPlatformIds.map((id) => makePlatformEntryId(id))),
    [disabledPlatformIds],
  );

  // 批量操作必须覆盖分组内的子平台，否则 entries 全是 group 类型时等于空循环
  const handleBulkEnable = (enabled: boolean) => {
    for (const entry of entries) {
      if (entry.type === 'api-relay') continue;
      for (const platformId of entry.platformIds) {
        setDisabledPlatform(platformId, !enabled);
      }
    }
  };
  const allEnabled =
    entries.length > 0 &&
    entries.every(
      (entry) =>
        entry.type === 'api-relay' ||
        entry.platformIds.every((id) => !disabledSet.has(makePlatformEntryId(id))),
    );

  const openCreateGroupEditor = () => {
    const firstPlatform = MENU_VISIBLE_PLATFORM_IDS[0] ?? 'codebuddy';

    setEditingGroupId(null);
    setGroupDraftName('');
    setGroupDraftPlatformIds([firstPlatform]);
    setGroupDraftDefaultPlatformId(firstPlatform);
    setGroupDraftIconKind('platform');
    setGroupDraftIconPlatformId(firstPlatform);
    setGroupDraftIconCustomDataUrl('');
    setGroupDraftError('');
    setGroupEditorOpen(true);
  };

  const openEditGroupEditor = (group: PlatformLayoutGroup) => {
    setEditingGroupId(group.id);
    setGroupDraftName(group.name);
    setGroupDraftPlatformIds([...group.platformIds]);
    setGroupDraftDefaultPlatformId(group.defaultPlatformId);
    setGroupDraftIconKind(group.iconKind);
    setGroupDraftIconPlatformId(group.iconPlatformId ?? group.defaultPlatformId);
    setGroupDraftIconCustomDataUrl(group.iconCustomDataUrl ?? '');
    setGroupDraftError('');
    setGroupEditorOpen(true);
  };

  useEffect(() => {
    if (!open || !requestedEditGroupId || handledRequestedEditGroupId === requestedEditGroupId) {
      return;
    }

    const group = platformGroups.find((item) => item.id === requestedEditGroupId);
    if (!group) {
      return;
    }

    setExpandedGroupIds((prev) => (prev.includes(group.id) ? prev : [...prev, group.id]));
    openEditGroupEditor(group);
    setHandledRequestedEditGroupId(requestedEditGroupId);
  }, [open, requestedEditGroupId, handledRequestedEditGroupId, platformGroups]);

  const closeGroupEditor = () => {
    setGroupEditorOpen(false);
    setEditingGroupId(null);
    setGroupDraftError('');
  };

  const openEditChildEditor = (group: PlatformLayoutGroup, platformId: PlatformId) => {
    const fallbackName = getPlatformLabel(platformId, t);
    const childConfig = getGroupChildConfig(group, platformId);
    const childIcon = resolveGroupChildIcon(group, platformId);

    setChildEditorGroupId(group.id);
    setChildEditorPlatformId(platformId);
    setChildDraftName(resolveGroupChildName(group, platformId, fallbackName));
    setChildDraftIconKind(childIcon.iconKind);
    setChildDraftIconPlatformId(childIcon.iconPlatformId);
    setChildDraftIconCustomDataUrl(childIcon.iconCustomDataUrl ?? '');
    setChildDraftSetDefault(group.defaultPlatformId === platformId);
    setChildDraftError('');
    setChildEditorOpen(true);

    if (childConfig?.iconKind === 'custom' && childConfig.iconCustomDataUrl) {
      upsertCustomIcon(childConfig.iconCustomDataUrl);
    }
  };

  const closeChildEditor = () => {
    setChildEditorOpen(false);
    setChildEditorGroupId(null);
    setChildEditorPlatformId(null);
    setChildDraftError('');
  };

  const handleGroupDraftPlatformsToggle = (platformId: PlatformId, checked: boolean) => {
    setGroupDraftPlatformIds((prev) => {
      const next = checked ? [...prev, platformId] : prev.filter((item) => item !== platformId);
      return Array.from(new Set(next));
    });
  };

  const handleSaveGroup = () => {
    const name = groupDraftName.trim();
    if (!name) {
      setGroupDraftError(t('platformLayout.groupNameRequired', '请输入分组名称'));
      return;
    }
    if (groupDraftPlatformIds.length === 0 || !groupDraftDefaultPlatformId) {
      setGroupDraftError(t('platformLayout.groupChildrenRequired', '至少选择一个子平台'));
      return;
    }
    if (groupDraftIconKind === 'custom' && !groupDraftIconCustomDataUrl.trim()) {
      setGroupDraftError(t('platformLayout.groupCustomIconRequired', '请上传自定义图标'));
      return;
    }

    const existingGroup = editingGroupId
      ? platformGroups.find((group) => group.id === editingGroupId) ?? null
      : null;

    const existingGroupIds = platformGroups.map((group) => group.id);
    const groupId = editingGroupId ?? createGroupId(name, existingGroupIds);

    const nextChildConfigs = (existingGroup?.childConfigs ?? []).filter((child) =>
      groupDraftPlatformIds.includes(child.platformId),
    );

    upsertPlatformGroup({
      id: groupId,
      name,
      platformIds: [...groupDraftPlatformIds],
      defaultPlatformId: groupDraftDefaultPlatformId,
      iconKind: groupDraftIconKind,
      iconPlatformId: groupDraftIconKind === 'platform' ? groupDraftIconPlatformId : groupDraftDefaultPlatformId,
      iconCustomDataUrl: groupDraftIconKind === 'custom' ? groupDraftIconCustomDataUrl : undefined,
      childConfigs: nextChildConfigs,
    });

    setExpandedGroupIds((prev) => (prev.includes(groupId) ? prev : [...prev, groupId]));
    closeGroupEditor();
  };

  const handleDeleteGroup = () => {
    if (!editingGroupId) {
      return;
    }
    const existingGroup = platformGroups.find((item) => item.id === editingGroupId);
    setConfirmAction({
      kind: 'deleteGroup',
      groupId: editingGroupId,
      groupName: existingGroup?.name || groupDraftName.trim() || editingGroupId,
    });
  };

  const handleConfirmAction = () => {
    if (!confirmAction) return;
    if (confirmAction.kind === 'reset') {
      resetPlatformLayout();
    } else if (confirmAction.kind === 'deleteGroup') {
      removePlatformGroup(confirmAction.groupId);
      closeGroupEditor();
    }
    setConfirmAction(null);
  };

  const handleSaveChild = () => {
    if (!childEditorGroupId || !childEditorPlatformId) {
      return;
    }
    const group = platformGroups.find((item) => item.id === childEditorGroupId);
    if (!group) {
      return;
    }

    const name = childDraftName.trim();
    if (!name) {
      setChildDraftError(t('platformLayout.groupNameRequired', '请输入分组名称'));
      return;
    }
    if (childDraftIconKind === 'custom' && !childDraftIconCustomDataUrl.trim()) {
      setChildDraftError(t('platformLayout.groupCustomIconRequired', '请上传自定义图标'));
      return;
    }

    const nextChildConfigs: PlatformLayoutGroupChildConfig[] = [
      ...(group.childConfigs ?? []).filter((child) => child.platformId !== childEditorPlatformId),
      {
        platformId: childEditorPlatformId,
        name,
        iconKind: childDraftIconKind,
        iconPlatformId: childDraftIconPlatformId,
        iconCustomDataUrl: childDraftIconKind === 'custom' ? childDraftIconCustomDataUrl : undefined,
      },
    ];

    upsertPlatformGroup({
      ...group,
      defaultPlatformId: childDraftSetDefault ? childEditorPlatformId : group.defaultPlatformId,
      childConfigs: nextChildConfigs,
    });

    closeChildEditor();
  };

  const handleSetGroupDefault = (group: PlatformLayoutGroup, platformId: PlatformId) => {
    if (group.defaultPlatformId === platformId) {
      return;
    }

    upsertPlatformGroup({
      ...group,
      defaultPlatformId: platformId,
    });
  };

  const removeChildFromGroup = (group: PlatformLayoutGroup, platformId: PlatformId) => {
    if (group.platformIds.length <= 1) {
      return false;
    }

    const removeIndex = group.platformIds.indexOf(platformId);
    if (removeIndex < 0) {
      return false;
    }

    const nextPlatformIds = group.platformIds.filter((item) => item !== platformId);
    const nextDefaultPlatformId =
      group.defaultPlatformId === platformId
        ? nextPlatformIds[Math.min(removeIndex, nextPlatformIds.length - 1)] ?? nextPlatformIds[0]
        : group.defaultPlatformId;

    upsertPlatformGroup({
      ...group,
      platformIds: nextPlatformIds,
      defaultPlatformId: nextDefaultPlatformId,
      childConfigs: (group.childConfigs ?? []).filter((child) => child.platformId !== platformId),
    });

    return true;
  };

  const handleRemoveChildFromGroup = () => {
    if (!childEditorGroupId || !childEditorPlatformId) {
      return;
    }
    const group = platformGroups.find((item) => item.id === childEditorGroupId);
    if (!group) {
      return;
    }

    const removed = removeChildFromGroup(group, childEditorPlatformId);
    if (!removed) {
      return;
    }

    closeChildEditor();
  };

  const togglePendingAddChild = (platformId: PlatformId, checked: boolean) => {
    setPendingAddChildIds((prev) => {
      const next = checked ? [...prev, platformId] : prev.filter((item) => item !== platformId);
      return Array.from(new Set(next));
    });
  };

  const openAddChildPanel = (groupId: string) => {
    setAddingChildGroupId(groupId);
    setPendingAddChildIds([]);
  };

  const closeAddChildPanel = () => {
    setAddingChildGroupId(null);
    setPendingAddChildIds([]);
  };

  const applyAddChildren = (group: PlatformLayoutGroup) => {
    if (pendingAddChildIds.length === 0) {
      closeAddChildPanel();
      return;
    }

    const nextPlatformIds = Array.from(new Set([...group.platformIds, ...pendingAddChildIds]));

    upsertPlatformGroup({
      ...group,
      platformIds: nextPlatformIds,
      childConfigs: group.childConfigs,
    });

    closeAddChildPanel();
    setExpandedGroupIds((prev) => (prev.includes(group.id) ? prev : [...prev, group.id]));
  };

  if (!open) return null;

  return (
    <div className="modal-overlay">
      <div className="modal modal-lg" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2>{t('platformLayout.title', '平台布局')}</h2>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close', '关闭')}>
            <X />
          </button>
        </div>

        <div className="modal-body platform-layout-modal-body">
          {persistenceError && (
            <div ref={persistenceErrorRef} className="platform-layout-group-error" role="alert">
              <span>{t('common.failed')}: {persistenceError}</span>
              <button className="btn btn-secondary" disabled={retryingPersistence} onClick={() => void retryPersistence()}>
                {t('common.windowsOperation.retry')}
              </button>
            </div>
          )}
          <div className="platform-layout-summary">
            <span>
              {t('platformLayout.sidebarSelectedUnlimited', { count: sidebarSelectedCount })}
            </span>
            <div className="platform-layout-summary-actions">
              <button className="btn btn-secondary" onClick={openCreateGroupEditor}>
                <Plus size={14} />
                <span>{t('platformLayout.addGroup', '新建分组')}</span>
              </button>
              <button className="btn btn-secondary" onClick={() => setConfirmAction({ kind: 'reset' })}>
                {t('platformLayout.reset', '恢复默认')}
              </button>
            </div>
          </div>

          <div className="platform-layout-tip">
            {t(
              'platformLayout.tipWithGroupsUnlimited',
              '拖拽可排序；可选择任意数量入口显示在侧边栏。分组子级不参与侧边栏/仪表盘开关，仅用于菜单栏与默认平台切换。',
            )}
          </div>

          {customIconStorageFailed && (
            <div className="platform-layout-group-error" role="alert">
              {t('platformLayout.customIconStorageFailed')}
            </div>
          )}

          <div className="platform-layout-bulk-header">
            <div className="platform-layout-bulk-header-left">
              <button type="button" className="btn btn-secondary platform-layout-expand-all-btn" onClick={toggleExpandAll}>
                {allGroupExpanded ? <ChevronsDownUp size={14} /> : <ChevronsUpDown size={14} />}
                <span>
                  {allGroupExpanded
                    ? t('platformLayout.collapseAllChildren')
                    : t('platformLayout.expandAllChildren')}
                </span>
              </button>
            </div>
            <div className="platform-layout-bulk-header-right">
              {/* 与行内控制区列顺序保持一致：启用 → 侧边栏 → 仪表盘 → 菜单栏 */}
              <div className="platform-layout-bulk-cell">
                <label className="platform-layout-bulk-toggle" title={t('platformLayout.disabledHint', '禁用后该平台的所有自动活动将停止')}>
                  <input
                    type="checkbox"
                    checked={allEnabled}
                    onChange={() => handleBulkEnable(!allEnabled)}
                  />
                  <span>{t('platformLayout.enabledToggle', '启用')}</span>
                </label>
              </div>
              <div className="platform-layout-bulk-cell">
                <label className="platform-layout-bulk-toggle">
                  <input
                    type="checkbox"
                    checked={sidebarBulkEnabled}
                    onChange={() => handleBulkSidebar(!sidebarBulkEnabled)}
                  />
                  <span>{t('platformLayout.sidebarToggle', '侧边栏')}</span>
                </label>
              </div>
              <div className="platform-layout-bulk-cell">
                <label className="platform-layout-bulk-toggle">
                  <input
                    type="checkbox"
                    checked={dashboardBulkEnabled}
                    onChange={() => handleBulkDashboard(!dashboardBulkEnabled)}
                  />
                  <span>{t('platformLayout.dashboardToggle', '仪表盘')}</span>
                </label>
              </div>
              <div className="platform-layout-bulk-cell">
                <label className="platform-layout-bulk-toggle">
                  <input
                    type="checkbox"
                    checked={trayBulkEnabled}
                    onChange={() => handleBulkTray(!trayBulkEnabled)}
                  />
                  <span>{t('platformLayout.trayToggle', '菜单栏显示')}</span>
                </label>
              </div>
              <div className="platform-layout-bulk-cell is-edit-column" />
            </div>
          </div>

          <SortableLayoutList
            items={entries.map(({ id }) => id)}
            className="platform-layout-list"
            onSortingChange={setSorting}
            onReorder={(activeId, overId) => {
              const from = layoutEntryOrderIds.findIndex((id) => id === activeId);
              const to = layoutEntryOrderIds.findIndex((id) => id === overId);
              if (from >= 0 && to >= 0) commitLayoutEntryOrder(arrayMove(layoutEntryOrderIds, from, to));
            }}
          >
            {entries.map((entry) => {
              const isApiRelayEntry = entry.type === 'api-relay';
              const selected = isSidebarEntrySelected(entry.id);
              const isGroup = entry.type === 'group' && !!entry.group;
              const groupId = entry.id;
              const groupExpanded = isGroup && expandedGroupIds.includes(groupId);
              const groupTrayEnabled = isApiRelayEntry
                ? false
                : isGroup
                  ? entry.platformIds.every((platformId) => traySet.has(platformId))
                  : entry.defaultPlatformId
                    ? traySet.has(entry.defaultPlatformId)
                    : false;
              // 分组整组启用状态：组内所有平台都启用时才勾选
              const groupAllEnabled =
                isGroup &&
                entry.platformIds.every(
                  (platformId) => !disabledSet.has(makePlatformEntryId(platformId)),
                );

              const rowClass = [
                'platform-layout-row',
                isApiRelayEntry ? 'is-api-relay-entry' : '',
                entry.hidden ? 'is-hidden' : '',
              ]
                .join(' ')
                .trim();

              return (
                <SortableLayoutItem
                  key={entry.id}
                  id={entry.id}
                  className={`platform-layout-entry ${entry.type === 'group' ? 'is-group-entry' : ''}`}
                >
                  {(handle) => <>
                  <div
                    className={rowClass}
                    onClick={() => {
                      if (entry.group) {
                        toggleGroupExpanded(groupId);
                      }
                    }}
                  >
                    <div className="platform-layout-main">
                      <button
                        type="button"
                        className="platform-layout-drag-trigger"
                        {...handle}
                        onClick={(event) => event.stopPropagation()}
                        aria-label={t('platformLayout.dragHandleLabel', '拖拽排序')}
                      >
                        <GripVertical size={16} className="drag-handle" />
                      </button>

                      {entry.group && (
                        <button
                          type="button"
                          className="platform-layout-expand-trigger"
                          onClick={(event) => {
                            event.stopPropagation();
                            toggleGroupExpanded(groupId);
                          }}
                          aria-label={
                            groupExpanded
                              ? t('platformLayout.collapseChildren', '收起子级')
                              : t('platformLayout.expandChildren', '展开子级')
                          }
                        >
                          {groupExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                        </button>
                      )}

                      {isApiRelayEntry && (
                        <button
                          type="button"
                          className="platform-layout-expand-trigger"
                          disabled
                          aria-label={t('platformLayout.expandChildren', '展开子级')}
                        >
                          <ChevronRight size={14} />
                        </button>
                      )}

                      <div className="platform-layout-icon">
                        {isApiRelayEntry ? (
                          <img
                            src={apiKeyFunIcon}
                            alt=""
                            className="platform-layout-group-icon"
                            style={{ width: 18, height: 18 }}
                          />
                        ) : entry.group ? (
                          renderGroupIcon(entry.group, 18)
                        ) : entry.defaultPlatformId ? (
                          renderPlatformIcon(entry.defaultPlatformId, 18)
                        ) : null}
                      </div>
                      <span
                        className="platform-layout-name"
                        onDoubleClick={(event) => {
                          event.stopPropagation();
                          if (entry.group) {
                            openEditGroupEditor(entry.group);
                          }
                        }}
                      >
                        {entry.label}
                      </span>

                    </div>

                    <div className="platform-layout-controls-grid" onClick={(event) => event.stopPropagation()}>
                      {/* 启用排在最前：它是影响面最大的动作（会停止一切后台活动） */}
                      {isApiRelayEntry && (
                        <span className="platform-layout-toggle platform-layout-enable-placeholder" aria-hidden="true">
                          <span className="platform-layout-checkbox-placeholder" />
                          <span>{t('platformLayout.enabledToggle', '启用')}</span>
                        </span>
                      )}
                      {entry.type === 'platform' && (
                        <label
                          className={`platform-layout-toggle ${disabledSet.has(entry.id) ? 'is-disabled' : ''}`}
                          title={t('platformLayout.disabledHint', '禁用后该平台的所有自动活动将停止，且其在应用设置中的配置项一并隐藏')}
                        >
                          <input
                            type="checkbox"
                            checked={!disabledSet.has(entry.id)}
                            onChange={(event) => {
                              const platformId = parsePlatformEntryId(entry.id);
                              if (!platformId) {
                                return;
                              }
                              setDisabledPlatform(platformId, !event.target.checked);
                            }}
                          />
                          <span>{t('platformLayout.enabledToggle', '启用')}</span>
                        </label>
                      )}

                      {isGroup && (
                        <label
                          className={`platform-layout-toggle ${groupAllEnabled ? '' : 'is-disabled'}`}
                          title={t('platformLayout.groupEnabledHint', '批量启用/禁用该分组内的所有平台')}
                        >
                          <input
                            type="checkbox"
                            checked={groupAllEnabled}
                            onChange={() =>
                              entry.platformIds.forEach((platformId) =>
                                setDisabledPlatform(platformId, groupAllEnabled),
                              )
                            }
                          />
                          <span>{t('platformLayout.enabledToggle', '启用')}</span>
                        </label>
                      )}

                      <label className="platform-layout-toggle">
                        <input
                          type="checkbox"
                          checked={selected}
                          onChange={(event) => setSidebarSelection(entry, event.target.checked)}
                        />
                        <span>{t('platformLayout.sidebarToggle', '侧边栏显示')}</span>
                      </label>

                      <label className="platform-layout-toggle">
                        <input
                          type="checkbox"
                          checked={isApiRelayEntry ? apiRelayDashboardVisible : !entry.hidden}
                          onChange={(event) => {
                            if (isApiRelayEntry) {
                              setApiRelayDashboardVisible(event.target.checked);
                              return;
                            }
                            setHiddenEntry(entry.id as PlatformLayoutEntryId, !event.target.checked);
                          }}
                        />
                        <span>{t('platformLayout.dashboardToggle', '仪表盘显示')}</span>
                      </label>

                      <label
                        className={`platform-layout-toggle ${isApiRelayEntry ? 'is-disabled' : ''}`}
                        title={
                          isApiRelayEntry
                            ? t('platformLayout.apiRelayTrayDisabled', '中转站暂不支持菜单栏显示')
                            : undefined
                        }
                      >
                        <input
                          type="checkbox"
                          checked={groupTrayEnabled}
                          disabled={isApiRelayEntry}
                          readOnly={isApiRelayEntry}
                          onChange={() => {
                            if (isApiRelayEntry) {
                              return;
                            }
                            const target = !groupTrayEnabled;
                            entry.platformIds.forEach((platformId) => setTrayPlatform(platformId, target));
                          }}
                        />
                        <span>{t('platformLayout.trayToggle', '菜单栏显示')}</span>
                      </label>

                      {entry.group ? (
                        <button
                          type="button"
                          className="action-btn"
                          onClick={() => openEditGroupEditor(entry.group!)}
                          title={t('platformLayout.editGroup', '编辑')}
                          aria-label={t('platformLayout.editGroup', '编辑')}
                        >
                          <Pencil size={13} />
                        </button>
                      ) : isApiRelayEntry ? (
                        <button
                          type="button"
                          className="action-btn"
                          disabled
                          title={t('platformLayout.editGroup', '编辑')}
                          aria-label={t('platformLayout.editGroup', '编辑')}
                        >
                          <Pencil size={13} />
                        </button>
                      ) : (
                        <span className="platform-layout-empty-edit-slot" />
                      )}
                    </div>
                  </div>

                  {entry.group && groupExpanded && (
                    <div className="platform-layout-group-children">
                      <div className="platform-layout-add-child-row">
                        <button
                          type="button"
                          className="btn btn-secondary"
                          onClick={() => {
                            if (addingChildGroupId === entry.group!.id) {
                              closeAddChildPanel();
                            } else {
                              openAddChildPanel(entry.group!.id);
                            }
                          }}
                        >
                          <Plus size={14} />
                          <span>{t('platformLayout.addChildPlatform')}</span>
                        </button>

                        {addingChildGroupId === entry.group.id && (
                          <div className="platform-layout-add-child-panel" onClick={(event) => event.stopPropagation()}>
                            <div className="platform-layout-add-child-options">
                              {(availableAddChildByGroup.get(entry.group.id) ?? []).map((platformId) => (
                                <label
                                  key={`add-child-${entry.group?.id}-${platformId}`}
                                  className="platform-layout-child-picker-item"
                                >
                                  <input
                                    type="checkbox"
                                    checked={pendingAddChildIds.includes(platformId)}
                                    onChange={(event) =>
                                      togglePendingAddChild(platformId, event.target.checked)
                                    }
                                  />
                                  <span className="platform-layout-child-picker-icon">
                                    {renderPlatformIcon(platformId, 14)}
                                  </span>
                                  <span>{getPlatformLabel(platformId, t)}</span>
                                </label>
                              ))}
                              {(availableAddChildByGroup.get(entry.group.id) ?? []).length === 0 && (
                                <span className="platform-layout-no-options">
                                  {t('platformLayout.noAvailableChildren', '暂无可添加的平台')}
                                </span>
                              )}
                            </div>
                            <div className="platform-layout-add-child-actions">
                              <button type="button" className="btn btn-secondary" onClick={closeAddChildPanel}>
                                {t('common.cancel', '取消')}
                              </button>
                              <button
                                type="button"
                                className="btn btn-primary"
                                onClick={() => applyAddChildren(entry.group!)}
                              >
                                {t('common.confirm', '确认')}
                              </button>
                            </div>
                          </div>
                        )}
                      </div>

                      <SortableLayoutList
                        items={entry.platformIds}
                        className="platform-layout-child-sortables"
                        onSortingChange={setSorting}
                        onReorder={(activeId, overId) => {
                          const group = entry.group!;
                          const from = group.platformIds.findIndex((id) => id === activeId);
                          const to = group.platformIds.findIndex((id) => id === overId);
                          if (from >= 0 && to >= 0) reorderGroupPlatforms(group.id, from, to);
                        }}
                      >
                      {entry.platformIds.map((platformId) => {
                        const childEntryId = makePlatformEntryId(platformId);
                        const childSidebarSelected = sidebarSet.has(childEntryId);
                        const isDefault = entry.group?.defaultPlatformId === platformId;
                        const childIcon = resolveGroupChildIcon(entry.group!, platformId);
                        const childName = resolveGroupChildName(
                          entry.group!,
                          platformId,
                          getPlatformLabel(platformId, t),
                        );
                        return (
                          <SortableLayoutItem
                            className="platform-layout-child-row"
                            key={platformId}
                            id={platformId}
                          >
                            {(childHandle) => <>
                            <div className="platform-layout-child-main">
                              <button
                                type="button"
                                className="platform-layout-child-drag-trigger"
                                {...childHandle}
                                onClick={(event) => event.stopPropagation()}
                                aria-label={t('platformLayout.dragHandleLabel', '拖拽排序')}
                              >
                                <GripVertical size={14} className="drag-handle" />
                              </button>
                              <div className="platform-layout-child-icon">
                                {childIcon.iconKind === 'custom' && childIcon.iconCustomDataUrl ? (
                                  <img
                                    src={childIcon.iconCustomDataUrl}
                                    alt={childName}
                                    className="platform-layout-group-icon"
                                    style={{ width: 16, height: 16 }}
                                  />
                                ) : (
                                  renderPlatformIcon(childIcon.iconPlatformId, 16)
                                )}
                              </div>
                              <span
                                className="platform-layout-child-name"
                                onDoubleClick={() => openEditChildEditor(entry.group!, platformId)}
                              >
                                {childName}
                              </span>
                            </div>

                            <div className="platform-layout-controls-grid is-child-grid">
                              <label
                                className={`platform-layout-toggle ${disabledSet.has(makePlatformEntryId(platformId)) ? 'is-disabled' : ''}`}
                                title={t('platformLayout.disabledHint', '禁用后该平台的所有自动活动将停止，且其在应用设置中的配置项一并隐藏')}
                              >
                                <input
                                  type="checkbox"
                                  checked={!disabledSet.has(makePlatformEntryId(platformId))}
                                  onChange={(event) =>
                                    setDisabledPlatform(platformId, !event.target.checked)
                                  }
                                />
                                <span>{t('platformLayout.enabledToggle', '启用')}</span>
                              </label>
                              <label className="platform-layout-toggle">
                                <input
                                  type="checkbox"
                                  checked={childSidebarSelected}
                                  onChange={(event) => setSidebarEntry(childEntryId, event.target.checked)}
                                />
                                <span>{t('platformLayout.sidebarToggle', '侧边栏显示')}</span>
                              </label>
                              <label className="platform-layout-toggle">
                                <input
                                  type="checkbox"
                                  checked={isDefault}
                                  onChange={() => handleSetGroupDefault(entry.group!, platformId)}
                                />
                                <span>{t('platformLayout.groupDefault', '默认平台')}</span>
                              </label>
                              <label className="platform-layout-toggle">
                                <input
                                  type="checkbox"
                                  checked={traySet.has(platformId)}
                                  onChange={(event) => setTrayPlatform(platformId, event.target.checked)}
                                />
                                <span>{t('platformLayout.trayToggle', '菜单栏显示')}</span>
                              </label>
                              <button
                                type="button"
                                className="action-btn"
                                onClick={() => openEditChildEditor(entry.group!, platformId)}
                                title={t('platformLayout.editGroup', '编辑')}
                                aria-label={t('platformLayout.editGroup', '编辑')}
                              >
                                <Pencil size={13} />
                              </button>
                              <button
                                type="button"
                                className="action-btn is-danger platform-layout-child-delete-btn"
                                onClick={() => removeChildFromGroup(entry.group!, platformId)}
                                disabled={entry.group!.platformIds.length <= 1}
                                title={
                                  entry.group!.platformIds.length <= 1
                                    ? t('platformLayout.groupChildrenRequired', '至少选择一个子平台')
                                    : t('common.delete', '删除')
                                }
                                aria-label={
                                  entry.group!.platformIds.length <= 1
                                    ? t('platformLayout.groupChildrenRequired', '至少选择一个子平台')
                                    : t('common.delete', '删除')
                                }
                              >
                                <Trash2 size={13} />
                              </button>
                            </div>
                            </>}
                          </SortableLayoutItem>
                        );
                      })}
                      </SortableLayoutList>
                    </div>
                  )}
                  </>}
                </SortableLayoutItem>
              );
            })}
          </SortableLayoutList>
        </div>

        {groupEditorOpen && (
          <div className="platform-layout-group-editor-overlay">
            <div className="platform-layout-group-editor-modal" onClick={(event) => event.stopPropagation()}>
              <div className="platform-layout-group-editor-header">
                <span>
                  {editingGroupId
                    ? t('platformLayout.editGroup', '编辑分组')
                    : t('platformLayout.addGroup', '新建分组')}
                </span>
                <button
                  type="button"
                  className="platform-layout-group-editor-close"
                  onClick={closeGroupEditor}
                  aria-label={t('common.close', '关闭')}
                >
                  <X size={16} />
                </button>
              </div>

              <div className="platform-layout-group-editor-grid">
                <label className="platform-layout-group-field">
                  <span>{t('platformLayout.groupName', '分组名称')}</span>
                  <input
                    type="text"
                    value={groupDraftName}
                    onChange={(event) => setGroupDraftName(event.target.value)}
                    placeholder={t('platformLayout.groupNamePlaceholder', '例如：CodeBuddy 套件')}
                  />
                </label>

                <div className="platform-layout-group-field">
                  <span>{t('platformLayout.groupIcon', '分组图标')}</span>
                  <IconSelector
                    customIcons={customIcons}
                    iconKind={groupDraftIconKind}
                    iconPlatformId={groupDraftIconPlatformId}
                    iconCustomDataUrl={groupDraftIconCustomDataUrl}
                    onSelectPlatform={(platformId) => {
                      setGroupDraftIconKind('platform');
                      setGroupDraftIconPlatformId(platformId);
                      setGroupDraftIconCustomDataUrl('');
                    }}
                    onSelectCustom={(dataUrl) => {
                      setGroupDraftIconKind('custom');
                      setGroupDraftIconCustomDataUrl(dataUrl);
                    }}
                    onUploadCustom={(dataUrl, fileName) => {
                      upsertCustomIcon(dataUrl, fileName);
                    }}
                    onDeleteCustom={removeCustomIcon}
                  />
                </div>

                <div className="platform-layout-group-field full-width">
                  <span>{t('platformLayout.groupChildren', '子级平台')}</span>
                  <div className="platform-layout-group-children-picker">
                    {MENU_VISIBLE_PLATFORM_IDS.map((platformId) => {
                      const checked = groupDraftPlatformIds.includes(platformId);
                      return (
                        <label
                          key={`group-draft-${platformId}`}
                          className="platform-layout-child-picker-item"
                        >
                          <input
                            type="checkbox"
                            checked={checked}
                            onChange={(event) =>
                              handleGroupDraftPlatformsToggle(platformId, event.target.checked)
                            }
                          />
                          <span className="platform-layout-child-picker-icon">
                            {renderPlatformIcon(platformId, 14)}
                          </span>
                          <span>{getPlatformLabel(platformId, t)}</span>
                        </label>
                      );
                    })}
                  </div>
                </div>

                <div className="platform-layout-group-field">
                  <span>{t('platformLayout.groupDefault', '默认平台')}</span>
                  <SingleSelectDropdown
                    value={groupDraftDefaultPlatformId}
                    options={groupDraftPlatformIds.map((platformId) => ({
                      value: platformId,
                      label: getPlatformLabel(platformId, t),
                    }))}
                    onChange={(nextValue) => setGroupDraftDefaultPlatformId(nextValue as PlatformId)}
                    disabled={groupDraftPlatformIds.length === 0}
                    ariaLabel={t('platformLayout.groupDefault', '默认平台')}
                    menuMaxHeight={240}
                  />
                </div>
              </div>

              {groupDraftError && <div className="platform-layout-group-error">{groupDraftError}</div>}

              <div className="platform-layout-group-editor-actions">
                {editingGroupId && (
                  <button type="button" className="btn btn-danger" onClick={handleDeleteGroup}>
                    <Trash2 size={14} />
                    <span>{t('platformLayout.deleteGroup', '删除分组')}</span>
                  </button>
                )}
                <div className="platform-layout-group-editor-actions-right">
                  <button type="button" className="btn btn-secondary" onClick={closeGroupEditor}>
                    {t('common.cancel', '取消')}
                  </button>
                  <button type="button" className="btn btn-primary" onClick={handleSaveGroup}>
                    {t('common.save', '保存')}
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}

        {childEditorOpen && childEditorGroupId && childEditorPlatformId && (
          <div className="platform-layout-group-editor-overlay">
            <div className="platform-layout-group-editor-modal" onClick={(event) => event.stopPropagation()}>
              <div className="platform-layout-group-editor-header">
                <span>{t('platformLayout.editChildPlatform', '编辑子级平台')}</span>
                <button
                  type="button"
                  className="platform-layout-group-editor-close"
                  onClick={closeChildEditor}
                  aria-label={t('common.close', '关闭')}
                >
                  <X size={16} />
                </button>
              </div>

              <div className="platform-layout-group-editor-grid">
                <label className="platform-layout-group-field">
                  <span>{t('platformLayout.groupName', '分组名称')}</span>
                  <input
                    type="text"
                    value={childDraftName}
                    onChange={(event) => setChildDraftName(event.target.value)}
                  />
                </label>

                <div className="platform-layout-group-field">
                  <span>{t('platformLayout.groupIcon', '分组图标')}</span>
                  <IconSelector
                    customIcons={customIcons}
                    iconKind={childDraftIconKind}
                    iconPlatformId={childDraftIconPlatformId}
                    iconCustomDataUrl={childDraftIconCustomDataUrl}
                    onSelectPlatform={(platformId) => {
                      setChildDraftIconKind('platform');
                      setChildDraftIconPlatformId(platformId);
                      setChildDraftIconCustomDataUrl('');
                    }}
                    onSelectCustom={(dataUrl) => {
                      setChildDraftIconKind('custom');
                      setChildDraftIconCustomDataUrl(dataUrl);
                    }}
                    onUploadCustom={(dataUrl, fileName) => {
                      upsertCustomIcon(dataUrl, fileName);
                    }}
                    onDeleteCustom={removeCustomIcon}
                  />
                </div>

                <label className="platform-layout-group-field full-width">
                  <span>{t('platformLayout.groupDefault', '默认平台')}</span>
                  <label className="platform-layout-toggle">
                    <input
                      type="checkbox"
                      checked={childDraftSetDefault}
                      onChange={(event) => setChildDraftSetDefault(event.target.checked)}
                    />
                    <span>{t('platformLayout.setAsDefaultInGroup', '设为分组内默认平台')}</span>
                  </label>
                </label>
              </div>

              {childDraftError && <div className="platform-layout-group-error">{childDraftError}</div>}

              <div className="platform-layout-group-editor-actions">
                <button
                  type="button"
                  className="btn btn-danger"
                  onClick={handleRemoveChildFromGroup}
                  disabled={(platformGroups.find((item) => item.id === childEditorGroupId)?.platformIds.length ?? 0) <= 1}
                  title={
                    (platformGroups.find((item) => item.id === childEditorGroupId)?.platformIds.length ?? 0) <= 1
                      ? t('platformLayout.groupChildrenRequired', '至少选择一个子平台')
                      : t('platformLayout.removeFromGroup', '移出分组')
                  }
                >
                  <Trash2 size={14} />
                  <span>{t('platformLayout.removeFromGroup', '移出分组')}</span>
                </button>
                <div className="platform-layout-group-editor-actions-right">
                  <button type="button" className="btn btn-secondary" onClick={closeChildEditor}>
                    {t('common.cancel', '取消')}
                  </button>
                  <button type="button" className="btn btn-primary" onClick={handleSaveChild}>
                    {t('common.save', '保存')}
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}

        {confirmAction && (
          <div className="platform-layout-group-editor-overlay">
            <div className="platform-layout-group-editor-modal platform-layout-confirm-modal" onClick={(event) => event.stopPropagation()}>
              <div className="platform-layout-group-editor-header">
                <span>
                  {confirmAction.kind === 'reset'
                    ? t('platformLayout.reset', '恢复默认')
                    : t('platformLayout.deleteGroup', '删除分组')}
                </span>
                <button
                  type="button"
                  className="platform-layout-group-editor-close"
                  onClick={() => setConfirmAction(null)}
                  aria-label={t('common.close', '关闭')}
                >
                  <X size={16} />
                </button>
              </div>
              <p className="platform-layout-confirm-desc">
                {confirmAction.kind === 'reset'
                  ? t('platformLayout.confirmResetDesc', '恢复默认将清空自定义分组与排序设置，确定继续吗？')
                  : t('platformLayout.confirmDeleteGroupDesc', '确定删除分组「{{name}}」？组内平台将恢复为独立显示。', {
                      name: confirmAction.groupName,
                    })}
              </p>
              <div className="platform-layout-group-editor-actions">
                <div className="platform-layout-group-editor-actions-right">
                  <button type="button" className="btn btn-secondary" onClick={() => setConfirmAction(null)}>
                    {t('common.cancel', '取消')}
                  </button>
                  <button type="button" className="btn btn-danger" onClick={handleConfirmAction}>
                    {confirmAction.kind === 'reset'
                      ? t('platformLayout.reset', '恢复默认')
                      : t('platformLayout.deleteGroup', '删除分组')}
                  </button>
                </div>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
