import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useDropdownPanelPlacement } from '../hooks/useDropdownPanelPlacement';
import './AccountFilterDropdown.css';

export interface SingleSelectFilterOption {
  value: string;
  label: string;
}

interface SingleSelectFilterDropdownProps {
  value: string;
  options: SingleSelectFilterOption[];
  ariaLabel: string;
  placeholder?: string;
  icon?: ReactNode;
  disabled?: boolean;
  onChange: (value: string) => void;
}

export function SingleSelectFilterDropdown({
  value,
  options,
  ariaLabel,
  placeholder,
  icon,
  disabled = false,
  onChange,
}: SingleSelectFilterDropdownProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const { panelPlacement, panelRef, scrollContainerStyle } = useDropdownPanelPlacement(
    rootRef,
    open,
    options.length,
  );

  const selectedOption = useMemo(
    () => options.find((option) => option.value === value) ?? null,
    [options, value],
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

  useEffect(() => {
    if (!disabled) return;
    setOpen(false);
  }, [disabled]);

  return (
    <div className="tag-filter single-filter account-filter-surface" ref={rootRef}>
      <button
        type="button"
        className={`tag-filter-btn single-filter-btn ${open ? 'active' : ''}`}
        onClick={() => {
          if (disabled) return;
          setOpen((prev) => !prev);
        }}
        aria-label={ariaLabel}
        aria-expanded={open}
        disabled={disabled}
      >
        {icon ? <span className="single-filter-icon">{icon}</span> : null}
        <span className="single-filter-label" title={selectedOption?.label ?? placeholder ?? ''}>
          {selectedOption?.label ?? placeholder ?? ''}
        </span>
      </button>
      {open && (
        <div
          ref={panelRef}
          className={`tag-filter-panel single-filter-panel ${panelPlacement === 'top' ? 'open-top' : ''}`}
        >
          <div className="tag-filter-options" style={scrollContainerStyle}>
            {options.map((option) => {
              const active = option.value === value;
              return (
                <button
                  key={option.value}
                  type="button"
                  className={`tag-filter-option single-filter-option ${active ? 'selected' : ''}`}
                  onClick={() => {
                    onChange(option.value);
                    setOpen(false);
                  }}
                >
                  <span className="tag-filter-name">{option.label}</span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}
