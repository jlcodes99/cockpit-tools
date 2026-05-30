import { Settings, Rocket, GaugeCircle, LayoutGrid, SlidersHorizontal, FileText, ChevronDown, PanelLeftClose, PanelLeftOpen, ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useState, useRef, useCallback, useEffect, useLayoutEffect, useMemo, type CSSProperties } from 'react';
import { createPortal } from 'react-dom';
import { Page } from '../../types/navigation';
import { isMenuVisiblePlatform, PlatformId, PLATFORM_PAGE_MAP } from '../../types/platform';
import {
  resolveGroupChildIcon,
  resolveGroupChildName,
  parseGroupEntryId,
  parsePlatformEntryId,
  PlatformLayoutEntryId,
  PlatformLayoutGroup,
  resolveEntryDefaultPlatformId,
  resolveEntryIdForPlatform,
  usePlatformLayoutStore,
} from '../../stores/usePlatformLayoutStore';
import { ORIGINAL_SIDEBAR_ENTRY_LIMIT, useSideNavLayoutStore } from '../../stores/useSideNavLayoutStore';
import { useGlobalModal } from '../../hooks/useGlobalModal';
import { getPlatformLabel, renderPlatformIcon } from '../../utils/platformMeta';
import { useAntigravityRuntimeTarget } from '../../hooks/useAntigravityRuntimeTarget';
import { setAntigravityRuntimeTargetFromPlatform } from '../../utils/antigravityRuntimeTarget';

interface SideNavProps {
  page: Page;
  setPage: (page: Page) => void;
  onOpenPlatformLayout: () => void;
  easterEggClickCount: number;
  onEasterEggTriggerClick: () => void;
  hasBreakoutSession: boolean;
  updateActionState: 'hidden' | 'available' | 'downloading' | 'installing' | 'ready';
  updateProgress: number;
  onUpdateActionClick: () => void;
  updateRemindersEnabled: boolean;
  onOpenLogViewer: () => void;
}

interface FlyingRocket {
  id: number;
  x: number;
}

interface SideNavEntry {
  id: PlatformLayoutEntryId;
  label: string;
  hidden: boolean;
  targetPlatformId: PlatformId;
  platformIds: PlatformId[];
  group: PlatformLayoutGroup | null;
}

const PAGE_PLATFORM_MAP: Partial<Record<Page, PlatformId>> = {
  overview: 'antigravity',
  codex: 'codex',
  'codex-api-service': 'codex',
  zed: 'zed',
  'github-copilot': 'github-copilot',
  windsurf: 'windsurf',
  kiro: 'kiro',
  cursor: 'cursor',
  gemini: 'gemini',
  codebuddy: 'codebuddy',
  'codebuddy-cn': 'codebuddy_cn',
  qoder: 'qoder',
  trae: 'trae',
  workbuddy: 'workbuddy',
};

const APP_DISPLAY_NAME =
  import.meta.env.VITE_COCKPIT_TOOLS_PROFILE === 'dev' ? 'Cockpit Tools Dev' : 'Cockpit Tools';

const CLASSIC_NAV_MIN_SCALE = 0.5;
const CLASSIC_NAV_SCALE_EPSILON = 0.004;
const CLASSIC_NAV_SCROLL_EPSILON = 4;

function renderEntryIcon(entry: SideNavEntry, size: number) {
  if (entry.group && entry.group.iconKind === 'custom' && entry.group.iconCustomDataUrl) {
    return (
      <img
        src={entry.group.iconCustomDataUrl}
        alt={entry.label}
        className="side-nav-group-icon"
        style={{ width: size, height: size }}
      />
    );
  }

  if (entry.group) {
    const iconPlatform = entry.group.iconPlatformId ?? entry.targetPlatformId;
    return renderPlatformIcon(iconPlatform, size);
  }

  return renderPlatformIcon(entry.targetPlatformId, size);
}

export function SideNav({
  page,
  setPage,
  onOpenPlatformLayout,
  easterEggClickCount,
  onEasterEggTriggerClick,
  hasBreakoutSession,
  updateActionState,
  updateProgress,
  onUpdateActionClick,
  updateRemindersEnabled,
  onOpenLogViewer,
}: SideNavProps) {
  const { t } = useTranslation();
  const { showModal } = useGlobalModal();
  const [flyingRockets, setFlyingRockets] = useState<FlyingRocket[]>([]);
  const [showMore, setShowMore] = useState(false);
  const [classicAdaptiveScale, setClassicAdaptiveScale] = useState(1);
  const [classicNavNeedsScroll, setClassicNavNeedsScroll] = useState(false);
  const [classicHandleTop, setClassicHandleTop] = useState<number | null>(null);
  const [morePopoverPosition, setMorePopoverPosition] = useState({
    top: 120,
    left: 210,
    maxHeight: 560,
  });
  const sideNavLayoutMode = useSideNavLayoutStore((state) => state.mode);
  const setSideNavLayoutMode = useSideNavLayoutStore((state) => state.setMode);
  const classicCollapsed = useSideNavLayoutStore((state) => state.classicCollapsed);
  const toggleClassicCollapsed = useSideNavLayoutStore((state) => state.toggleClassicCollapsed);
  const hideClassicSwitchPrompt = useSideNavLayoutStore((state) => state.hideClassicSwitchPrompt);
  const setHideClassicSwitchPrompt = useSideNavLayoutStore((state) => state.setHideClassicSwitchPrompt);
  const isClassicLayout = sideNavLayoutMode === 'classic';
  const isClassicCollapsed = isClassicLayout && classicCollapsed;
  const showClassicLabels = isClassicLayout && !classicCollapsed;
  const rocketIdRef = useRef(0);
  const classicSwitchDontAskAgainRef = useRef(false);
  const sideNavRef = useRef<HTMLElement>(null);
  const updateEntryRef = useRef<HTMLDivElement>(null);
  const brandRef = useRef<HTMLDivElement>(null);
  const navItemsRef = useRef<HTMLDivElement>(null);
  const bottomActionsRef = useRef<HTMLDivElement>(null);
  const logoRef = useRef<HTMLDivElement>(null);
  const morePopoverRef = useRef<HTMLDivElement>(null);
  const moreButtonRef = useRef<HTMLButtonElement>(null);

  const {
    orderedEntryIds,
    hiddenEntryIds,
    sidebarEntryIds,
    platformGroups,
  } = usePlatformLayoutStore();

  const antigravityRuntimeTarget = useAntigravityRuntimeTarget();
  const currentPlatformId = page === 'overview'
    ? antigravityRuntimeTarget
    : PAGE_PLATFORM_MAP[page] ?? null;
  const currentEntryId = useMemo(
    () => (currentPlatformId ? resolveEntryIdForPlatform(currentPlatformId, platformGroups) : null),
    [currentPlatformId, platformGroups],
  );

  const hiddenSet = useMemo(() => new Set(hiddenEntryIds), [hiddenEntryIds]);
  const sidebarSet = useMemo(() => new Set(sidebarEntryIds), [sidebarEntryIds]);

  const orderedEntries = useMemo<SideNavEntry[]>(() => {
    return orderedEntryIds
      .map((entryId) => {
        const platformId = parsePlatformEntryId(entryId);
        if (platformId) {
          if (!isMenuVisiblePlatform(platformId)) {
            return null;
          }
          return {
            id: entryId,
            label: getPlatformLabel(platformId, t),
            hidden: hiddenSet.has(entryId),
            targetPlatformId: platformId,
            platformIds: [platformId],
            group: null,
          };
        }

        const groupId = parseGroupEntryId(entryId);
        if (!groupId) {
          return null;
        }
        const group = platformGroups.find((item) => item.id === groupId);
        if (!group) {
          return null;
        }

        const visiblePlatformIds = group.platformIds.filter(isMenuVisiblePlatform);
        if (visiblePlatformIds.length === 0) {
          return null;
        }

        const resolvedTargetPlatformId = resolveEntryDefaultPlatformId(entryId, platformGroups);
        const targetPlatformId =
          resolvedTargetPlatformId && visiblePlatformIds.includes(resolvedTargetPlatformId)
            ? resolvedTargetPlatformId
            : visiblePlatformIds[0];
        if (!targetPlatformId) {
          return null;
        }

        return {
          id: entryId,
          label: group.name,
          hidden: hiddenSet.has(entryId),
          targetPlatformId,
          platformIds: visiblePlatformIds,
          group,
        };
      })
      .filter((entry): entry is SideNavEntry => !!entry);
  }, [orderedEntryIds, platformGroups, hiddenSet, t]);

  const sidebarVisibleEntries = useMemo(
    () => orderedEntries.filter((entry) => sidebarSet.has(entry.id) && !entry.hidden),
    [orderedEntries, sidebarSet],
  );

  const sidebarMenuEntries = useMemo(
    () => (
      isClassicLayout
        ? sidebarVisibleEntries
        : sidebarVisibleEntries.slice(0, ORIGINAL_SIDEBAR_ENTRY_LIMIT)
    ),
    [isClassicLayout, sidebarVisibleEntries],
  );

  const navigateToPlatform = useCallback((platformId: PlatformId) => {
    setAntigravityRuntimeTargetFromPlatform(platformId);
    setPage(PLATFORM_PAGE_MAP[platformId]);
  }, [setPage]);

  const classicScaleContentKey = useMemo(
    () => sidebarMenuEntries
      .map((entry) => `${entry.id}:${entry.platformIds.join(',')}`)
      .join('|'),
    [sidebarMenuEntries],
  );

  const sidebarMenuEntryIdSet = useMemo(
    () => new Set(sidebarMenuEntries.map((entry) => entry.id)),
    [sidebarMenuEntries],
  );

  const sidebarMenuPlatformIdSet = useMemo(
    () => new Set(sidebarMenuEntries.map((entry) => entry.targetPlatformId)),
    [sidebarMenuEntries],
  );

  const moreMenuEntries = useMemo<SideNavEntry[]>(
    () => {
      if (isClassicLayout) {
        return orderedEntries.filter((entry) => !sidebarMenuEntryIdSet.has(entry.id));
      }

      return orderedEntries
        .map((entry) => {
          const remainingPlatformIds = entry.platformIds.filter(
            (platformId) => !sidebarMenuPlatformIdSet.has(platformId),
          );
          if (remainingPlatformIds.length === 0) {
            return null;
          }
          const resolvedTargetPlatformId = remainingPlatformIds.includes(entry.targetPlatformId)
            ? entry.targetPlatformId
            : remainingPlatformIds[0];
          return {
            ...entry,
            targetPlatformId: resolvedTargetPlatformId,
            platformIds: remainingPlatformIds,
          };
        })
        .filter((entry): entry is SideNavEntry => !!entry);
    },
    [isClassicLayout, orderedEntries, sidebarMenuEntryIdSet, sidebarMenuPlatformIdSet],
  );

  const isMoreActive = !!currentEntryId && !sidebarMenuEntryIdSet.has(currentEntryId);
  const shouldLockActiveOnMore = showMore;

  const shouldShowUpdateEntry = updateActionState !== 'hidden'
    && (
      updateRemindersEnabled
      || updateActionState === 'downloading'
      || updateActionState === 'installing'
      || updateActionState === 'ready'
    );

  const recalculateClassicAdaptiveScale = useCallback(() => {
    if (!isClassicLayout || typeof window === 'undefined') {
      setClassicAdaptiveScale((prev) => (prev === 1 ? prev : 1));
      setClassicNavNeedsScroll((prev) => (prev ? false : prev));
      return;
    }

    const navElement = sideNavRef.current;
    const navItemsElement = navItemsRef.current;
    const brandElement = brandRef.current;
    const bottomActionsElement = bottomActionsRef.current;

    if (!navElement || !navItemsElement || !brandElement || !bottomActionsElement) {
      return;
    }

    const parsePixel = (value: string): number => {
      const parsed = Number.parseFloat(value);
      return Number.isFinite(parsed) ? parsed : 0;
    };

    const currentScale = classicAdaptiveScale > 0 ? classicAdaptiveScale : 1;
    const navStyles = window.getComputedStyle(navElement);
    const navPaddingTop = parsePixel(navStyles.paddingTop);
    const navPaddingBottom = parsePixel(navStyles.paddingBottom);
    const navGap = parsePixel(navStyles.rowGap || navStyles.gap || '0');

    const navItemsStyles = window.getComputedStyle(navItemsElement);
    const navItemsPaddingTop = parsePixel(navItemsStyles.paddingTop);
    const navItemsPaddingBottom = parsePixel(navItemsStyles.paddingBottom);
    const navItemElements = Array.from(navItemsElement.children).filter(
      (node): node is HTMLElement => node instanceof HTMLElement,
    );
    let navItemsContentHeight = 0;
    if (navItemElements.length > 0) {
      const firstRect = navItemElements[0].getBoundingClientRect();
      const lastRect = navItemElements[navItemElements.length - 1].getBoundingClientRect();
      navItemsContentHeight = Math.max(0, lastRect.bottom - firstRect.top);
    }
    const navItemsRequiredHeight = navItemsPaddingTop + navItemsContentHeight + navItemsPaddingBottom;

    if (navItemsRequiredHeight <= 0) {
      return;
    }

    const sections: number[] = [
      brandElement.offsetHeight,
      navItemsRequiredHeight,
      bottomActionsElement.offsetHeight,
    ];
    if (updateEntryRef.current) {
      sections.unshift(updateEntryRef.current.offsetHeight);
    }

    const sectionsHeight = sections.reduce((sum, height) => sum + height, 0);
    const sectionsGap = Math.max(0, sections.length - 1) * navGap;
    const requiredScaledHeight = navPaddingTop + navPaddingBottom + sectionsHeight + sectionsGap;
    if (requiredScaledHeight <= 0) {
      return;
    }

    const requiredBaseHeight = requiredScaledHeight / currentScale;
    const availableHeight = window.innerHeight;
    if (availableHeight <= 0) {
      return;
    }

    const fitScale = Math.min(1, availableHeight / requiredBaseHeight);
    let nextScale = requiredBaseHeight <= availableHeight + 0.5
      ? 1
      : Math.max(CLASSIC_NAV_MIN_SCALE, fitScale);

    const currentOverflow = navItemsElement.scrollHeight - navItemsElement.clientHeight;
    if (currentOverflow > CLASSIC_NAV_SCROLL_EPSILON && navItemsElement.scrollHeight > 0) {
      const overflowFitScale = currentScale * (navItemsElement.clientHeight / navItemsElement.scrollHeight);
      if (Number.isFinite(overflowFitScale) && overflowFitScale > 0) {
        nextScale = Math.max(CLASSIC_NAV_MIN_SCALE, Math.min(nextScale, overflowFitScale));
      }
    }

    const updateScrollNeed = () => {
      const target = navItemsRef.current;
      if (!target) {
        return;
      }
      const overflow = target.scrollHeight - target.clientHeight;
      const shouldScroll = overflow > CLASSIC_NAV_SCROLL_EPSILON;

      if (
        shouldScroll
        && target.scrollHeight > 0
        && classicAdaptiveScale > CLASSIC_NAV_MIN_SCALE + CLASSIC_NAV_SCALE_EPSILON
      ) {
        const overflowFitScale = classicAdaptiveScale * (target.clientHeight / target.scrollHeight);
        if (Number.isFinite(overflowFitScale) && overflowFitScale > 0) {
          const correctedScale = Math.max(CLASSIC_NAV_MIN_SCALE, overflowFitScale);
          if (classicAdaptiveScale - correctedScale > CLASSIC_NAV_SCALE_EPSILON) {
            setClassicAdaptiveScale(Number(correctedScale.toFixed(5)));
            return;
          }
        }
      }

      setClassicNavNeedsScroll((prev) => (prev === shouldScroll ? prev : shouldScroll));
    };

    if (Math.abs(nextScale - classicAdaptiveScale) > CLASSIC_NAV_SCALE_EPSILON) {
      setClassicAdaptiveScale(Number(nextScale.toFixed(5)));
      window.requestAnimationFrame(updateScrollNeed);
      return;
    }

    updateScrollNeed();
  }, [classicAdaptiveScale, isClassicLayout]);

  useLayoutEffect(() => {
    if (!isClassicLayout || typeof window === 'undefined') {
      return;
    }
    recalculateClassicAdaptiveScale();
    const raf = window.requestAnimationFrame(recalculateClassicAdaptiveScale);
    return () => {
      window.cancelAnimationFrame(raf);
    };
  }, [
    classicCollapsed,
    classicScaleContentKey,
    isClassicLayout,
    recalculateClassicAdaptiveScale,
    shouldShowUpdateEntry,
  ]);

  useEffect(() => {
    if (!isClassicLayout || typeof window === 'undefined') {
      return;
    }
    const onResize = () => {
      recalculateClassicAdaptiveScale();
    };
    window.addEventListener('resize', onResize);

    const resizeObserver = typeof ResizeObserver !== 'undefined'
      ? new ResizeObserver(() => {
        recalculateClassicAdaptiveScale();
      })
      : null;

    if (resizeObserver) {
      if (sideNavRef.current) {
        resizeObserver.observe(sideNavRef.current);
      }
      if (navItemsRef.current) {
        resizeObserver.observe(navItemsRef.current);
      }
    }

    const mutationObserver = typeof MutationObserver !== 'undefined' && navItemsRef.current
      ? new MutationObserver(() => {
        recalculateClassicAdaptiveScale();
      })
      : null;

    if (mutationObserver && navItemsRef.current) {
      mutationObserver.observe(navItemsRef.current, {
        childList: true,
        subtree: true,
      });
    }

    return () => {
      window.removeEventListener('resize', onResize);
      resizeObserver?.disconnect();
      mutationObserver?.disconnect();
    };
  }, [isClassicLayout, recalculateClassicAdaptiveScale]);

  const classicMainIconSize = Math.max(14, Math.round(20 * classicAdaptiveScale));
  const classicBrandLogoIconSize = Math.max(14, Math.round(20 * classicAdaptiveScale));
  const classicHandleIconSize = Math.max(12, Math.round(16 * classicAdaptiveScale));

  const classicScaleStyle = isClassicLayout
    ? ({ '--side-nav-classic-adaptive-scale': classicAdaptiveScale } as CSSProperties)
    : undefined;

  const classicHandleStyle = ({
    '--side-nav-classic-adaptive-scale': classicAdaptiveScale,
    ...(classicHandleTop == null ? {} : { top: `${classicHandleTop}px` }),
  } as CSSProperties);

  const handleClassicLayoutEntryClick = useCallback(() => {
    if (hideClassicSwitchPrompt) {
      setSideNavLayoutMode('classic');
      return;
    }
    classicSwitchDontAskAgainRef.current = false;
    showModal({
      title: t('nav.switchClassicLayoutTitle', '切换至经典侧边栏布局'),
      description: t(
        'nav.switchClassicLayoutDesc',
        '经典布局会展示完整平台导航并支持折叠。你仍可在“设置 > 通用 > 侧边栏布局”中随时切换回原始布局。',
      ),
      width: 'sm',
      closeOnOverlay: false,
      content: (
        <div className="side-nav-layout-switch-modal-content">
          <label className="side-nav-layout-switch-remember">
            <input
              type="checkbox"
              onChange={(event) => {
                classicSwitchDontAskAgainRef.current = event.target.checked;
              }}
            />
            <span>{t('nav.switchClassicLayoutDontAskAgain', '不再提示此引导')}</span>
          </label>
        </div>
      ),
      actions: [
        {
          id: 'classic-switch-cancel',
          label: t('common.cancel', '取消'),
          variant: 'secondary',
        },
        {
          id: 'classic-switch-confirm',
          label: t('nav.switchClassicLayoutConfirm', '立即切换'),
          variant: 'primary',
          onClick: () => {
            if (classicSwitchDontAskAgainRef.current) {
              setHideClassicSwitchPrompt(true);
            }
            setSideNavLayoutMode('classic');
          },
        },
      ],
    });
  }, [hideClassicSwitchPrompt, setHideClassicSwitchPrompt, setSideNavLayoutMode, showModal, t]);

  useLayoutEffect(() => {
    if (!isClassicLayout || typeof window === 'undefined') {
      setClassicHandleTop(null);
      return;
    }

    const updateClassicHandleTop = () => {
      const rect = logoRef.current?.getBoundingClientRect();
      if (!rect) return;
      setClassicHandleTop(rect.top + rect.height / 2);
    };

    updateClassicHandleTop();
    const rafId = window.requestAnimationFrame(updateClassicHandleTop);
    const resizeObserver = typeof ResizeObserver !== 'undefined' && logoRef.current
      ? new ResizeObserver(() => {
        updateClassicHandleTop();
      })
      : null;

    if (resizeObserver && logoRef.current) {
      resizeObserver.observe(logoRef.current);
    }

    window.addEventListener('resize', updateClassicHandleTop);

    return () => {
      window.cancelAnimationFrame(rafId);
      window.removeEventListener('resize', updateClassicHandleTop);
      resizeObserver?.disconnect();
    };
  }, [isClassicLayout, isClassicCollapsed, shouldShowUpdateEntry]);

  const handleLogoClick = useCallback(() => {
    if (hasBreakoutSession) {
      onEasterEggTriggerClick();
      return;
    }

    const newRocket: FlyingRocket = {
      id: rocketIdRef.current++,
      x: (Math.random() - 0.5) * 40,
    };

    setFlyingRockets((prev) => [...prev, newRocket]);

    setTimeout(() => {
      setFlyingRockets((prev) => prev.filter((rocket) => rocket.id !== newRocket.id));
    }, 1500);

    onEasterEggTriggerClick();
  }, [hasBreakoutSession, onEasterEggTriggerClick]);

  useEffect(() => {
    if (!showMore) return;
    const handleClickOutside = (event: MouseEvent) => {
      const target = event.target as Node;
      if (morePopoverRef.current?.contains(target)) return;
      if (moreButtonRef.current?.contains(target)) return;
      setShowMore(false);
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, [showMore]);

  useEffect(() => {
    if (!showMore || !isClassicLayout) return;

    const updatePopoverPosition = () => {
      const button = moreButtonRef.current;
      const nav = button?.closest('.side-nav');
      const popover = morePopoverRef.current;
      if (!button || !nav || !popover) return;

      const navRect = nav.getBoundingClientRect();
      const buttonRect = button.getBoundingClientRect();
      const safeTop = 52;
      const safeBottom = 16;
      const popoverHeight = Math.max(180, popover.offsetHeight);
      const preferredTop = buttonRect.top + buttonRect.height / 2 - popoverHeight / 2;
      const maxTop = Math.max(safeTop, window.innerHeight - safeBottom - popoverHeight);
      const top = Math.max(safeTop, Math.min(preferredTop, maxTop));
      const maxHeight = Math.max(180, window.innerHeight - top - safeBottom);

      setMorePopoverPosition({
        top,
        left: navRect.right - 1,
        maxHeight,
      });
    };

    updatePopoverPosition();
    const rafId = window.requestAnimationFrame(updatePopoverPosition);
    window.addEventListener('resize', updatePopoverPosition);
    const resizeObserver = typeof ResizeObserver !== 'undefined' && morePopoverRef.current
      ? new ResizeObserver(() => {
        updatePopoverPosition();
      })
      : null;
    if (resizeObserver && morePopoverRef.current) {
      resizeObserver.observe(morePopoverRef.current);
    }

    return () => {
      window.cancelAnimationFrame(rafId);
      window.removeEventListener('resize', updatePopoverPosition);
      resizeObserver?.disconnect();
    };
  }, [showMore, isClassicLayout]);

  const clampedUpdateProgress = Math.max(0, Math.min(100, Math.round(updateProgress)));
  const updateVisualState = updateActionState === 'ready'
    ? 'restart'
    : updateActionState === 'downloading' || updateActionState === 'installing'
      ? 'progress'
      : 'update';

  const morePopoverContent = showMore ? (
    <div
      className={`side-nav-more-popover${isClassicLayout ? ' side-nav-more-popover-classic' : ''}`}
      ref={morePopoverRef}
      style={
        isClassicLayout
          ? {
              position: 'fixed',
              top: `${morePopoverPosition.top}px`,
              left: `${morePopoverPosition.left}px`,
              maxHeight: `${morePopoverPosition.maxHeight}px`,
            }
          : undefined
      }
    >
      <div className="side-nav-more-title">{t('nav.morePlatforms', '更多平台')}</div>
      <div className="side-nav-more-list">
        {moreMenuEntries.map((entry) => {
          const active = isClassicLayout
            ? currentEntryId === entry.id
            : !!currentPlatformId && entry.platformIds.includes(currentPlatformId);
          const showGroupParent =
            !entry.group || !sidebarMenuEntryIdSet.has(entry.id);
          return (
            <div className="side-nav-more-group" key={entry.id}>
              {(isClassicLayout || showGroupParent) && (
                <button
                  className={`side-nav-more-item ${active ? 'active' : ''}`}
                  onClick={() => {
                    navigateToPlatform(entry.targetPlatformId);
                    setShowMore(false);
                  }}
                >
                  <span className="side-nav-more-item-icon">{renderEntryIcon(entry, 16)}</span>
                  <span className="side-nav-more-item-label">{entry.label}</span>
                  {entry.hidden && (
                    <span className="side-nav-more-item-badge">
                      {t('platformLayout.hiddenBadge', '已隐藏')}
                    </span>
                  )}
                </button>
              )}

              {!isClassicLayout && entry.group && entry.platformIds.length > (showGroupParent ? 1 : 0) && (
                <div className={`side-nav-more-sub-list${showGroupParent ? '' : ' is-flat'}`}>
                  {entry.platformIds.map((platformId) => {
                    const icon = resolveGroupChildIcon(entry.group!, platformId);
                    const label = resolveGroupChildName(
                      entry.group!,
                      platformId,
                      getPlatformLabel(platformId, t),
                    );
                    return (
                      <button
                        key={`${entry.id}:${platformId}`}
                        className={`${
                          showGroupParent ? 'side-nav-more-sub-item' : 'side-nav-more-item'
                        } ${currentPlatformId === platformId ? 'active' : ''}`}
                        onClick={() => {
                          navigateToPlatform(platformId);
                          setShowMore(false);
                        }}
                      >
                        <span className={showGroupParent ? 'side-nav-more-sub-item-icon' : 'side-nav-more-item-icon'}>
                          {icon.iconKind === 'custom' && icon.iconCustomDataUrl ? (
                            <img
                              src={icon.iconCustomDataUrl}
                              alt={label}
                              className="side-nav-group-icon"
                              style={{ width: showGroupParent ? 14 : 16, height: showGroupParent ? 14 : 16 }}
                            />
                          ) : (
                            renderPlatformIcon(icon.iconPlatformId, showGroupParent ? 14 : 16)
                          )}
                        </span>
                        <span className={showGroupParent ? 'side-nav-more-sub-item-label' : 'side-nav-more-item-label'}>
                          {label}
                        </span>
                      </button>
                    );
                  })}
                </div>
              )}
            </div>
          );
        })}
      </div>
      <button
        className="side-nav-more-manage"
        onClick={() => {
          setShowMore(false);
          onOpenPlatformLayout();
        }}
      >
        <SlidersHorizontal size={14} />
        <span>{t('platformLayout.openFromMore', '管理平台布局')}</span>
      </button>
    </div>
  ) : null;

  return (
    <>
      <nav
        ref={sideNavRef}
        style={classicScaleStyle}
        className={`side-nav${isClassicLayout ? ' side-nav-classic' : ''}${isClassicCollapsed ? ' side-nav-classic-collapsed' : ''}`}
      >
      {shouldShowUpdateEntry && (
        <div className="side-nav-update-entry" ref={updateEntryRef}>
          <button
            type="button"
            className={`side-nav-update-btn is-${updateVisualState}`}
            onClick={onUpdateActionClick}
            title={
              updateActionState === 'downloading'
                ? t('update_notification.downloading', '下载中...')
                : updateActionState === 'installing'
                  ? t('nav.quickUpdate.installing', '安装中')
                  : updateActionState === 'ready'
                    ? t('nav.quickUpdate.restart', '重启')
                    : t('nav.quickUpdate.update', '更新')
            }
            disabled={updateActionState === 'installing'}
          >
            {updateActionState === 'downloading' ? (
              <span className="side-nav-update-progress-lr">
                <span
                  className={`side-nav-update-progress-fill${clampedUpdateProgress >= 100 ? ' is-full' : ''}`}
                  style={{ width: `${clampedUpdateProgress}%` }}
                >
                  <span className="side-nav-update-progress-ripple side-nav-update-progress-ripple-a" />
                  <span className="side-nav-update-progress-ripple side-nav-update-progress-ripple-b" />
                </span>
                <span className="side-nav-update-progress-percent">{clampedUpdateProgress}%</span>
              </span>
            ) : updateActionState === 'installing' ? (
              <span className="side-nav-update-text">{t('nav.quickUpdate.installing', '安装中')}</span>
            ) : (
              <span className="side-nav-update-text">
                {updateActionState === 'ready'
                  ? t('nav.quickUpdate.restart', '重启')
                  : t('nav.quickUpdate.update', '更新')}
              </span>
            )}
          </button>
        </div>
      )}

      <div className="nav-brand" ref={brandRef} style={{ position: 'relative', zIndex: 10 }}>
        <div className="side-nav-brand-main">
          <div
            ref={logoRef}
            className={`brand-logo rocket-easter-egg${hasBreakoutSession ? ' rocket-easter-egg-active' : ''}`}
            onClick={handleLogoClick}
            title={hasBreakoutSession ? t('breakout.resumeGameNav', '继续游戏') : undefined}
          >
            <Rocket size={isClassicLayout ? classicBrandLogoIconSize : 20} />
            {hasBreakoutSession && <span className="rocket-session-indicator" aria-hidden="true" />}
            {!hasBreakoutSession && easterEggClickCount > 0 && (
              <span className="rocket-click-count">{easterEggClickCount}</span>
            )}
          </div>

          {isClassicLayout && !isClassicCollapsed && (
            <div className="side-nav-brand-title">{APP_DISPLAY_NAME}</div>
          )}
        </div>

        <div
          style={{
            position: 'absolute',
            top: 0,
            left: 0,
            width: '100%',
            height: '100%',
            pointerEvents: 'none',
          }}
        >
          {flyingRockets.map((rocket) => (
            <span
              key={rocket.id}
              className="flying-rocket"
              style={{ '--rocket-x': `${rocket.x}px` } as CSSProperties}
            >
              🚀
            </span>
          ))}
        </div>

      </div>

      <div
        className={`nav-items${isClassicLayout && !classicNavNeedsScroll ? ' nav-items-no-scroll' : ''}`}
        ref={navItemsRef}
      >
        <button
          className={`nav-item ${page === 'dashboard' && !shouldLockActiveOnMore ? 'active' : ''}`}
          onClick={() => setPage('dashboard')}
          title={t('nav.dashboard')}
        >
          <GaugeCircle size={isClassicLayout ? classicMainIconSize : 20} />
          {showClassicLabels ? (
            <span className="nav-item-text">{t('nav.dashboard')}</span>
          ) : !isClassicLayout ? (
            <span className="tooltip">{t('nav.dashboard')}</span>
          ) : null}
        </button>

        {sidebarMenuEntries.map((entry) => {
          const active = currentEntryId === entry.id && !shouldLockActiveOnMore;
          return (
            <button
              key={entry.id}
              className={`nav-item ${active ? 'active' : ''}`}
              onClick={() => navigateToPlatform(entry.targetPlatformId)}
              title={entry.label}
            >
              {renderEntryIcon(entry, isClassicLayout ? classicMainIconSize : 20)}
              {showClassicLabels ? (
                <span className="nav-item-text">{entry.label}</span>
              ) : !isClassicLayout ? (
                <span className="tooltip">{entry.label}</span>
              ) : null}
            </button>
          );
        })}

        <button
          ref={moreButtonRef}
          className={`nav-item ${showMore || isMoreActive ? 'active' : ''}`}
          onClick={() => setShowMore((prev) => !prev)}
          title={t('nav.morePlatforms', '更多平台')}
        >
          <LayoutGrid size={isClassicLayout ? classicMainIconSize : 20} />
          {showClassicLabels ? (
            <span className="nav-item-text">{t('nav.morePlatforms', '更多平台')}</span>
          ) : !isClassicLayout ? (
            <span className="tooltip">{t('nav.morePlatforms', '更多平台')}</span>
          ) : null}
        </button>

        {morePopoverContent && (
          isClassicLayout && typeof document !== 'undefined'
            ? createPortal(morePopoverContent, document.body)
            : morePopoverContent
        )}

      </div>

      {isClassicLayout && (
        <div className="nav-bottom-actions" ref={bottomActionsRef}>
          <button
            className={`nav-item ${page === '2fa' && !shouldLockActiveOnMore ? 'active' : ''}`}
            onClick={() => setPage('2fa')}
            title={t('nav.2faManager', '2FA / MFA 管理')}
          >
            <ShieldCheck size={isClassicLayout ? classicMainIconSize : 20} />
            {showClassicLabels ? (
              <span className="nav-item-text">{t('nav.2faManager', '2FA / MFA 管理')}</span>
            ) : null}
          </button>

          <button
            className="nav-item"
            onClick={onOpenLogViewer}
            title={t('nav.logs', '日志')}
          >
            <FileText size={isClassicLayout ? classicMainIconSize : 20} />
            {showClassicLabels ? (
              <span className="nav-item-text">{t('nav.logs', '日志')}</span>
            ) : null}
          </button>

          <button
            className={`nav-item ${page === 'settings' && !shouldLockActiveOnMore ? 'active' : ''}`}
            onClick={() => setPage('settings')}
            title={t('nav.settings')}
          >
            <Settings size={isClassicLayout ? classicMainIconSize : 20} />
            {showClassicLabels ? (
              <span className="nav-item-text">{t('nav.settings')}</span>
            ) : null}
          </button>
        </div>
      )}

      {!isClassicLayout && (
        <>
          <div className="nav-footer">
            <button
              className={`nav-item ${page === 'settings' && !shouldLockActiveOnMore ? 'active' : ''}`}
              onClick={() => setPage('settings')}
              title={t('nav.settings')}
            >
              <Settings size={20} />
              <span className="tooltip">{t('nav.settings')}</span>
            </button>
          </div>

          <button
            type="button"
            className="side-nav-layout-switch-trigger"
            onClick={handleClassicLayoutEntryClick}
            aria-label={t('nav.switchClassicLayoutEntry', '切换到经典布局')}
          >
            <ChevronDown size={16} />
            <span className="tooltip">{t('nav.switchClassicLayoutEntry', '切换到经典布局')}</span>
          </button>
        </>
      )}

      </nav>

      {isClassicLayout && (
        <button
          type="button"
          className={`side-nav-classic-handle${isClassicCollapsed ? ' side-nav-classic-handle-collapsed' : ''}`}
          onClick={toggleClassicCollapsed}
          style={classicHandleStyle}
          title={
            classicCollapsed
              ? t('nav.expandSidebar', '展开侧边栏')
              : t('nav.collapseSidebar', '收起侧边栏')
          }
          aria-label={
            classicCollapsed
              ? t('nav.expandSidebar', '展开侧边栏')
              : t('nav.collapseSidebar', '收起侧边栏')
          }
        >
          {classicCollapsed
            ? <PanelLeftOpen size={classicHandleIconSize} />
            : <PanelLeftClose size={classicHandleIconSize} />}
        </button>
      )}
    </>
  );
}
