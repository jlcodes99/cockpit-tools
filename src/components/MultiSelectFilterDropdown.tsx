import { useEffect, useRef, useState } from 'react';
import { Filter } from 'lucide-react';
import { useDropdownPanelPlacement } from '../hooks/useDropdownPanelPlacement';
import './AccountFilterDropdown.css'

export interface MultiSelectFilterOption {
  value: string;
  label: string;
}

interface MultiSelectFilterDropdownProps {
  options: MultiSelectFilterOption[];
  selectedValues: string[];
  allLabel: string;
  filterLabel: string;
  clearLabel: string;
  emptyLabel: string;
  ariaLabel: string;
  onToggleValue: (value: string) => void;
  onClear: () => void;
}

export function MultiSelectFilterDropdown({
  options,
  selectedValues,
  allLabel,
  filterLabel,
  clearLabel,
  emptyLabel,
  ariaLabel,
  onToggleValue,
  onClear,
}: MultiSelectFilterDropdownProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const selectedCount = selectedValues.length;
  const { panelPlacement, panelRef, scrollContainerStyle } = useDropdownPanelPlacement(
    rootRef,
    open,
    options.length,
  );

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (!target) return;
      if (rootRef.current?.contains(target)) return;
      setOpen(false);
    };
    document.addEventListener('mousedown', onPointerDown);
    return () => document.removeEventListener('mousedown', onPointerDown);
  }, [open]);

  return (
    <div className="tag-filter multi-filter account-filter-surface" ref={rootRef}>
      <button
        type="button"
        className={`tag-filter-btn ${selectedCount > 0 ? 'active' : ''}`}
        onClick={() => setOpen((prev) => !prev)}
        aria-label={ariaLabel}
      >
        <Filter size={14} />
        {selectedCount > 0 ? `${filterLabel}(${selectedCount})` : allLabel}
      </button>
      {open && (
        <div
          ref={panelRef}
          className={`tag-filter-panel ${panelPlacement === 'top' ? 'open-top' : ''}`}
        >
          <div className="tag-filter-options" style={scrollContainerStyle}>
            <label className={`tag-filter-option ${selectedCount === 0 ? 'selected' : ''}`}>
              <input type="checkbox" checked={selectedCount === 0} onChange={onClear} />
              <span className="tag-filter-name">{allLabel}</span>
            </label>
            {options.length === 0 ? (
              <div className="tag-filter-empty">{emptyLabel}</div>
            ) : (
              options.map((option) => (
                <label
                  key={option.value}
                  className={`tag-filter-option ${selectedValues.includes(option.value) ? 'selected' : ''}`}
                >
                  <input
                    type="checkbox"
                    checked={selectedValues.includes(option.value)}
                    onChange={() => onToggleValue(option.value)}
                  />
                  <span className="tag-filter-name">{option.label}</span>
                </label>
              ))
            )}
          </div>
          {selectedCount > 0 && (
            <>
              <div className="tag-filter-divider" />
              <button type="button" className="tag-filter-clear" onClick={onClear}>
                {clearLabel}
              </button>
            </>
          )}
        </div>
      )}
    </div>
  );
}
