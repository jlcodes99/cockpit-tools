import {
  ChangeEvent,
  MouseEvent as ReactMouseEvent,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
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
import { isMenuVisiblePlatform, MENU_VISIBLE_PLATFORM_IDS, PlatformId } from '../types/platform';
import {
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
  usePlatformLayoutStore,
} from '../stores/usePlatformLayoutStore';
import { ORIGINAL_SIDEBAR_ENTRY_LIMIT, useSideNavLayoutStore } from '../stores/useSideNavLayoutStore';
import { getPlatformLabel, renderPlatformIcon } from '../utils/platformMeta';
import { useEscClose } from '../hooks/useEscClose';

const PLATFORM_LAYOUT_ICON_STORAGE_KEY = 'agtools.platform_layout.custom_icons.v1';

interface PlatformLayoutModalProps {
  open: boolean;
  requestedEditGroupId?: string | null;
  onClose: () => void;
}

interface LayoutEntryItem {
  id: PlatformLayoutEntryId;
  type: 'platform' | 'group';
  label: string;
  hidden: boolean;
  group: PlatformLayoutGroup | null;
  defaultPlatformId: PlatformId;
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

function loadCustomIcons(): PlatformLayoutCustomIcon[] {
  if (typeof window === 'undefined') {
    return [];
  }
  try {
    const raw = localStorage.getItem(PLATFORM_LAYOUT_ICON_STORAGE_KEY);
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
  useEscClose(open, onClose);
  const sideNavLayoutMode = useSideNavLayoutStore((state) => state.mode);
  const isSidebarSelectionLimited = sideNavLayoutMode === 'original';
  const sidebarSelectionLimit = isSidebarSelectionLimited
    ? ORIGINAL_SIDEBAR_ENTRY_LIMIT
    : Number.MAX_SAFE_INTEGER;
  const {
    orderedEntryIds,
    hiddenEntryIds,
    sidebarEntryIds,
    trayPlatformIds,
    platformGroups,
    moveEntry,
    reorderGroupPlatforms,
    setHiddenEntry,
    setSidebarEntry,
    setTrayPlatform,
    upsertPlatformGroup,
    removePlatformGroup,
    resetPlatformLayout,
  } = usePlatformLayoutStore();

  const [draggingId, setDraggingId] = useState<PlatformLayoutEntryId | null>(null);
  const [dropTargetId, setDropTargetId] = useState<PlatformLayoutEntryId | null>(null);
  const [draggingChild, setDraggingChild] = useState<{ groupId: string; platformId: PlatformId } | null>(null);
  const [dropChildTarget, setDropChildTarget] = useState<{ groupId: string; platformId: PlatformId } | null>(null);

  const [customIcons, setCustomIcons] = useState<PlatformLayoutCustomIcon[]>(() => loadCustomIcons());

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
    return result;
  }, [orderedEntryIds, platformGroups, hiddenSet, t]);

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

  useEffect(() => {
    if (typeof window === 'undefined') {
      return;
    }
    localStorage.setItem(PLATFORM_LAYOUT_ICON_STORAGE_KEY, JSON.stringify(customIcons));
  }, [customIcons]);

  useEffect(() => {
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

    setCustomIcons((prev) => {
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
  }, [platformGroups]);

  useEffect(() => {
    if (!open || (!draggingId && !draggingChild)) return;
    const handleMouseUp = () => {
      setDraggingId(null);
      setDropTargetId(null);
      setDraggingChild(null);
      setDropChildTarget(null);
    };
    window.addEventListener('mouseup', handleMouseUp);
    return () => window.removeEventListener('mouseup', handleMouseUp);
  }, [open, draggingId, draggingChild]);

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
      setDraggingId(null);
      setDropTargetId(null);
      setDraggingChild(null);
      setDropChildTarget(null);
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
    setCustomIcons((prev) => {
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

  const removeCustomIcon = (iconId: string) => {
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

    setCustomIcons((prev) => prev.filter((item) => item.id !== iconId));
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

  const stopDragging = () => {
    setDraggingId(null);
    setDropTargetId(null);
  };

  const stopChildDragging = () => {
    setDraggingChild(null);
    setDropChildTarget(null);
  };

  const handleDragStart = (event: ReactMouseEvent, id: PlatformLayoutEntryId) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    setDraggingId(id);
    setDropTargetId(null);
  };

  const handleDragMove = (targetId: PlatformLayoutEntryId) => {
    if (!draggingId) return;
    if (draggingId === targetId) {
      setDropTargetId(null);
      return;
    }
    setDropTargetId(targetId);
    const fromIndex = orderedEntryIds.indexOf(draggingId);
    const toIndex = orderedEntryIds.indexOf(targetId);
    if (fromIndex < 0 || toIndex < 0) return;
    moveEntry(fromIndex, toIndex);
  };

  const handleChildDragStart = (event: ReactMouseEvent, groupId: string, platformId: PlatformId) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    setDraggingChild({ groupId, platformId });
    setDropChildTarget(null);
  };

  const handleChildDragMove = (groupId: string, targetPlatformId: PlatformId) => {
    if (!draggingChild) return;
    if (draggingChild.groupId !== groupId) {
      setDropChildTarget(null);
      return;
    }
    if (draggingChild.platformId === targetPlatformId) {
      setDropChildTarget(null);
      return;
    }

    const group = platformGroups.find((item) => item.id === groupId);
    if (!group) return;
    const fromIndex = group.platformIds.indexOf(draggingChild.platformId);
    const toIndex = group.platformIds.indexOf(targetPlatformId);
    if (fromIndex < 0 || toIndex < 0) return;

    setDropChildTarget({ groupId, platformId: targetPlatformId });
    reorderGroupPlatforms(groupId, fromIndex, toIndex);
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

  const handleBulkSidebar = (enabled: boolean) => {
    entries.forEach((entry) => {
      setSidebarEntry(entry.id, false);
    });
    if (!enabled) {
      return;
    }
    const visibleEntries = entries.filter((entry) => !entry.hidden);
    const targetEntries = isSidebarSelectionLimited
      ? visibleEntries.slice(0, sidebarSelectionLimit)
      : visibleEntries;
    targetEntries.forEach((entry) => setSidebarEntry(entry.id, true));
  };

  const handleBulkDashboard = (enabled: boolean) => {
    entries.forEach((entry) => {
      setHiddenEntry(entry.id, !enabled);
    });
  };

  const handleBulkTray = (enabled: boolean) => {
    MENU_VISIBLE_PLATFORM_IDS.forEach((platformId) => setTrayPlatform(platformId, enabled));
  };

  const sidebarVisibleEntries = useMemo(
    () => entries.filter((entry) => !entry.hidden),
    [entries],
  );
  const sidebarBulkTargetCount = isSidebarSelectionLimited
    ? Math.min(sidebarSelectionLimit, sidebarVisibleEntries.length)
    : sidebarVisibleEntries.length;
  const sidebarBulkEnabled = sidebarBulkTargetCount > 0
    && sidebarVisibleEntries.filter((entry) => sidebarSet.has(entry.id)).length >= sidebarBulkTargetCount;
  const dashboardBulkEnabled = entries.length > 0 && entries.every((entry) => !entry.hidden);
  const trayBulkEnabled = MENU_VISIBLE_PLATFORM_IDS.every((platformId) => traySet.has(platformId));

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
    removePlatformGroup(editingGroupId);
    closeGroupEditor();
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
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal modal-lg" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2>{t('platformLayout.title', '平台布局')}</h2>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close', '关闭')}>
            <X />
          </button>
        </div>

        <div className="modal-body platform-layout-modal-body">
          <div className="platform-layout-summary">
            <span>
              {isSidebarSelectionLimited
                ? t('platformLayout.sidebarSelected', {
                  count: sidebarEntryIds.length,
                  max: sidebarSelectionLimit,
                  defaultValue: '侧边栏已选择 {{count}}/{{max}}',
                })
                : t('platformLayout.sidebarSelectedUnlimited', {
                  count: sidebarEntryIds.length,
                  defaultValue: '侧边栏已选择 {{count}}',
                })}
            </span>
            <div className="platform-layout-summary-actions">
              <button className="btn btn-secondary" onClick={openCreateGroupEditor}>
                <Plus size={14} />
                <span>{t('platformLayout.addGroup', '新建分组')}</span>
              </button>
              <button className="btn btn-secondary" onClick={resetPlatformLayout}>
                {t('platformLayout.reset', '恢复默认')}
              </button>
            </div>
          </div>

          <div className="platform-layout-tip">
            {isSidebarSelectionLimited
              ? t(
                'platformLayout.tipWithGroups',
                '拖拽可排序；最多选择三个入口显示在侧边栏。分组子级不参与侧边栏/仪表盘开关，仅用于菜单栏与默认平台切换。',
              )
              : t(
                'platformLayout.tipWithGroupsUnlimited',
                '拖拽可排序；可选择任意数量入口显示在侧边栏。分组子级不参与侧边栏/仪表盘开关，仅用于菜单栏与默认平台切换。',
              )}
          </div>

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
              <div className="platform-layout-bulk-cell">
                <label className="platform-layout-bulk-toggle">
                  <input
                    type="checkbox"
                    checked={sidebarBulkEnabled}
                    onChange={() => handleBulkSidebar(!sidebarBulkEnabled)}
                  />
                  <span>{t('platformLayout.sidebarToggle', '侧边栏显示')}</span>
                </label>
              </div>
              <div className="platform-layout-bulk-cell">
                <label className="platform-layout-bulk-toggle">
                  <input
                    type="checkbox"
                    checked={dashboardBulkEnabled}
                    onChange={() => handleBulkDashboard(!dashboardBulkEnabled)}
                  />
                  <span>{t('platformLayout.dashboardToggle', '仪表盘显示')}</span>
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

          <div
            className={`platform-layout-list ${draggingId || draggingChild ? 'is-sorting' : ''}`}
            onMouseUp={() => {
              stopDragging();
              stopChildDragging();
            }}
            onMouseLeave={() => {
              stopDragging();
              stopChildDragging();
            }}
          >
            {entries.map((entry) => {
              const selected = sidebarSet.has(entry.id);
              const sidebarFull = isSidebarSelectionLimited && sidebarEntryIds.length >= sidebarSelectionLimit;
              const sidebarDisabled = entry.hidden || (!selected && sidebarFull);
              const isGroup = entry.type === 'group' && !!entry.group;
              const groupId = entry.id;
              const groupExpanded = expandedGroupIds.includes(groupId);
              const groupTrayEnabled = isGroup
                ? entry.platformIds.every((platformId) => traySet.has(platformId))
                : traySet.has(entry.defaultPlatformId);

              const rowClass = [
                'platform-layout-row',
                entry.hidden ? 'is-hidden' : '',
                draggingId === entry.id ? 'is-dragging' : '',
                draggingId && draggingId !== entry.id ? 'is-drop-candidate' : '',
                draggingId && draggingId !== entry.id && dropTargetId === entry.id ? 'is-drop-target' : '',
              ]
                .join(' ')
                .trim();

              return (
                <div
                  key={entry.id}
                  className={`platform-layout-entry ${entry.type === 'group' ? 'is-group-entry' : ''}`}
                >
                  <div
                    className={rowClass}
                    onMouseEnter={() => handleDragMove(entry.id)}
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
                        onMouseDown={(event) => handleDragStart(event, entry.id)}
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

                      <div className="platform-layout-icon">
                        {entry.group
                          ? renderGroupIcon(entry.group, 18)
                          : renderPlatformIcon(entry.defaultPlatformId, 18)}
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
                      <label className={`platform-layout-toggle ${sidebarDisabled ? 'is-disabled' : ''}`}>
                        <input
                          type="checkbox"
                          checked={selected}
                          disabled={sidebarDisabled}
                          onChange={(event) => setSidebarEntry(entry.id, event.target.checked)}
                        />
                        <span>{t('platformLayout.sidebarToggle', '侧边栏显示')}</span>
                      </label>

                      <label className="platform-layout-toggle">
                        <input
                          type="checkbox"
                          checked={!entry.hidden}
                          onChange={(event) => setHiddenEntry(entry.id, !event.target.checked)}
                        />
                        <span>{t('platformLayout.dashboardToggle', '仪表盘显示')}</span>
                      </label>

                      <label className="platform-layout-toggle">
                        <input
                          type="checkbox"
                          checked={groupTrayEnabled}
                          onChange={() => {
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

                      {entry.platformIds.map((platformId) => {
                        const isDefault = entry.group?.defaultPlatformId === platformId;
                        const childIcon = resolveGroupChildIcon(entry.group!, platformId);
                        const childName = resolveGroupChildName(
                          entry.group!,
                          platformId,
                          getPlatformLabel(platformId, t),
                        );
                        const draggingChildInCurrentGroup = draggingChild?.groupId === entry.group?.id;
                        const isDraggingChildRow =
                          draggingChildInCurrentGroup && draggingChild?.platformId === platformId;
                        const isChildDropCandidate =
                          draggingChildInCurrentGroup && draggingChild?.platformId !== platformId;
                        const isChildDropTarget =
                          dropChildTarget?.groupId === entry.group?.id && dropChildTarget?.platformId === platformId;
                        const childRowClass = [
                          'platform-layout-child-row',
                          isDraggingChildRow ? 'is-dragging' : '',
                          isChildDropCandidate ? 'is-drop-candidate' : '',
                          isChildDropTarget ? 'is-drop-target' : '',
                        ]
                          .join(' ')
                          .trim();

                        return (
                          <div
                            className={childRowClass}
                            key={`${entry.id}:${platformId}`}
                            onMouseEnter={() => handleChildDragMove(entry.group!.id, platformId)}
                          >
                            <div className="platform-layout-child-main">
                              <button
                                type="button"
                                className="platform-layout-child-drag-trigger"
                                onMouseDown={(event) => handleChildDragStart(event, entry.group!.id, platformId)}
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
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>

        {groupEditorOpen && (
          <div className="platform-layout-group-editor-overlay" onClick={closeGroupEditor}>
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

                <label className="platform-layout-group-field">
                  <span>{t('platformLayout.groupDefault', '默认平台')}</span>
                  <select
                    value={groupDraftDefaultPlatformId}
                    onChange={(event) => setGroupDraftDefaultPlatformId(event.target.value as PlatformId)}
                    disabled={groupDraftPlatformIds.length === 0}
                  >
                    {groupDraftPlatformIds.map((platformId) => (
                      <option key={`group-default-${platformId}`} value={platformId}>
                        {getPlatformLabel(platformId, t)}
                      </option>
                    ))}
                  </select>
                </label>
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
          <div className="platform-layout-group-editor-overlay" onClick={closeChildEditor}>
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
      </div>
    </div>
  );
}
