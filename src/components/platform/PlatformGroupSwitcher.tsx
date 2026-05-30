import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Check, ChevronDown, Pencil } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { PLATFORM_PAGE_MAP, type PlatformId } from '../../types/platform';
import type { Page } from '../../types/navigation';
import { renderPlatformIcon } from '../../utils/platformMeta';
import { setAntigravityRuntimeTargetFromPlatform } from '../../utils/antigravityRuntimeTarget';

export interface PlatformGroupSwitcherOption {
  platformId: PlatformId;
  label: string;
}

interface PlatformGroupSwitcherProps {
  currentPlatformId: PlatformId;
  currentLabel: string;
  options: PlatformGroupSwitcherOption[];
  currentGroupId?: string | null;
  activePlatformId?: PlatformId | null;
  extraOptions?: Array<{
    id: string;
    label: string;
    page: Page;
    icon?: ReactNode;
    active?: boolean;
  }>;
}

export function PlatformGroupSwitcher({
  currentPlatformId,
  currentLabel,
  options,
  currentGroupId = null,
  activePlatformId = currentPlatformId,
  extraOptions = [],
}: PlatformGroupSwitcherProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const dropdownRef = useRef<HTMLDivElement | null>(null);
  const [dropdownPosition, setDropdownPosition] = useState({ top: 0, left: 0 });

  const updateDropdownPosition = useCallback(() => {
    const trigger = triggerRef.current;
    if (!trigger) {
      return;
    }

    const rect = trigger.getBoundingClientRect();
    const dropdownWidth = dropdownRef.current?.offsetWidth ?? rect.width;
    const maxLeft = window.innerWidth - dropdownWidth - 8;

    setDropdownPosition({
      top: Math.round(rect.bottom + 8),
      left: Math.round(Math.max(8, Math.min(rect.left, maxLeft))),
    });
  }, []);

  useEffect(() => {
    if (!open) {
      return;
    }

    const handleMouseDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (containerRef.current?.contains(target) || dropdownRef.current?.contains(target)) {
        return;
      }
      setOpen(false);
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false);
      }
    };

    document.addEventListener('mousedown', handleMouseDown);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('mousedown', handleMouseDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [open]);

  useEffect(() => {
    if (!open) {
      return;
    }

    updateDropdownPosition();
    const raf = window.requestAnimationFrame(updateDropdownPosition);
    const handleViewportChange = () => updateDropdownPosition();
    window.addEventListener('resize', handleViewportChange);
    window.addEventListener('scroll', handleViewportChange, true);
    return () => {
      window.cancelAnimationFrame(raf);
      window.removeEventListener('resize', handleViewportChange);
      window.removeEventListener('scroll', handleViewportChange, true);
    };
  }, [open, updateDropdownPosition]);

  const handleSwitchPlatform = (nextPlatform: PlatformId) => {
    setOpen(false);
    if (nextPlatform === activePlatformId) {
      return;
    }

    setAntigravityRuntimeTargetFromPlatform(nextPlatform);
    const targetPage = PLATFORM_PAGE_MAP[nextPlatform];
    window.dispatchEvent(new CustomEvent('app-request-navigate', { detail: targetPage }));
  };

  const handleOpenPlatformLayout = () => {
    setOpen(false);
    window.dispatchEvent(
      new CustomEvent('app-open-platform-layout', {
        detail: { groupId: currentGroupId },
      }),
    );
  };

  const handleSwitchPage = (page: Page) => {
    setOpen(false);
    window.dispatchEvent(new CustomEvent('app-request-navigate', { detail: page }));
  };

  return (
    <div className="platform-group-switcher" ref={containerRef}>
      <button
        type="button"
        className={`platform-group-switcher-trigger ${open ? 'is-open' : ''}`}
        ref={triggerRef}
        onClick={() => setOpen((prev) => !prev)}
        aria-label={t('platformLayout.groupSwitchLabel', '切换同组平台')}
        aria-haspopup="listbox"
        aria-expanded={open}
      >
        <span className="platform-group-switcher-trigger-icon">
          {renderPlatformIcon(currentPlatformId, 16)}
        </span>
        <span className="platform-group-switcher-trigger-label">{currentLabel}</span>
        <ChevronDown size={16} className="platform-group-switcher-trigger-caret" />
      </button>

      {open &&
        createPortal(
          <div
            className="platform-group-switcher-dropdown"
            role="listbox"
            ref={dropdownRef}
            style={{
              top: dropdownPosition.top,
              left: dropdownPosition.left,
            }}
          >
            {options.map((item) => {
              const activeItem = activePlatformId === item.platformId;
              return (
                <button
                  key={`switch-${item.platformId}`}
                  type="button"
                  className={`platform-group-switcher-option ${activeItem ? 'is-active' : ''}`}
                  role="option"
                  aria-selected={activeItem}
                  onClick={() => handleSwitchPlatform(item.platformId)}
                >
                  <span className="platform-group-switcher-option-icon">
                    {renderPlatformIcon(item.platformId, 18)}
                  </span>
                  <span className="platform-group-switcher-option-label">{item.label}</span>
                  <span className="platform-group-switcher-option-check">
                    {activeItem ? <Check size={16} /> : null}
                  </span>
                </button>
              );
            })}

            {extraOptions.map((item) => (
              <button
                key={`switch-page-${item.id}`}
                type="button"
                className={`platform-group-switcher-option ${item.active ? 'is-active' : ''}`}
                role="option"
                aria-selected={item.active === true}
                onClick={() => handleSwitchPage(item.page)}
              >
                <span className="platform-group-switcher-option-icon">
                  {item.icon ?? renderPlatformIcon(currentPlatformId, 18)}
                </span>
                <span className="platform-group-switcher-option-label">{item.label}</span>
                <span className="platform-group-switcher-option-check">
                  {item.active ? <Check size={16} /> : null}
                </span>
              </button>
            ))}

            <div className="platform-group-switcher-divider" />

            <button
              type="button"
              className="platform-group-switcher-action"
              onClick={handleOpenPlatformLayout}
            >
              <span className="platform-group-switcher-action-icon">
                <Pencil size={16} />
              </span>
              <span className="platform-group-switcher-action-label">
                {t('accounts.groups.manageTitle', '分组管理')}
              </span>
            </button>
          </div>,
          document.body,
        )}
    </div>
  );
}
