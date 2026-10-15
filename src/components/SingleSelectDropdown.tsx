import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useEscCloseTopmost } from "../hooks/useEscClose";
import { Check, ChevronDown } from "lucide-react";
import "./SingleSelectDropdown.css";
import { dropdownLayer, dropdownPlacement, type DropdownPlacement } from "../utils/dropdownPlacement";

export interface SingleSelectOption {
  value: string;
  label: string;
}

interface SingleSelectDropdownProps {
  value: string;
  options: SingleSelectOption[];
  onChange: (value: string) => void;
  className?: string;
  menuClassName?: string;
  disabled?: boolean;
  ariaLabel?: string;
  placeholder?: string;
  menuPlacement?: "down" | "up";
  menuWidth?: number;
  menuMaxHeight?: number;
}

export function SingleSelectDropdown({
  value,
  options,
  onChange,
  className,
  menuClassName,
  disabled = false,
  ariaLabel,
  placeholder,
  menuPlacement = "down",
  menuWidth,
  menuMaxHeight = 280,
}: SingleSelectDropdownProps) {
  const [open, setOpen] = useState(false);
  const [menuStyle, setMenuStyle] = useState<DropdownPlacement | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);

  const closeMenu = () => { setOpen(false); triggerRef.current?.focus(); };
  useEscCloseTopmost(open, closeMenu, menuRef);
  useEffect(() => {
    if (open && menuStyle) {
      (menuRef.current?.querySelector<HTMLElement>('[aria-selected="true"]')
        ?? menuRef.current?.querySelector<HTMLElement>('[role="option"]'))?.focus();
    }
  }, [open, Boolean(menuStyle)]);

  const selectedOption = useMemo(
    () => options.find((option) => option.value === value) ?? null,
    [options, value],
  );

  useEffect(() => {
    if (!open) return;

    const updateMenuPosition = (event?: Event) => {
      const eventTarget = event?.target;
      if (
        event?.type === "scroll" &&
        eventTarget instanceof Node &&
        menuRef.current?.contains(eventTarget)
      ) {
        return;
      }
      const rect = triggerRef.current?.getBoundingClientRect();
      if (!rect) return;
      const layers: string[] = [];
      for (let ancestor: Element | null = triggerRef.current; ancestor; ancestor = ancestor.parentElement) {
        layers.push(window.getComputedStyle(ancestor).zIndex);
      }
      const nextStyle = dropdownPlacement(rect, { width: window.innerWidth, height: window.innerHeight },
        menuPlacement, menuWidth, menuMaxHeight, dropdownLayer(layers));
      setMenuStyle((prev) =>
        prev &&
        prev.top === nextStyle.top &&
        prev.bottom === nextStyle.bottom &&
        prev.left === nextStyle.left &&
        prev.width === nextStyle.width &&
        prev.maxHeight === nextStyle.maxHeight &&
        prev.zIndex === nextStyle.zIndex
          ? prev
          : nextStyle,
      );
    };

    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (!target) return;
      if (rootRef.current?.contains(target)) return;
      if (menuRef.current?.contains(target)) return;
      setOpen(false);
    };

    updateMenuPosition();
    // 延迟绑定，避免与打开触发器同一次 pointer/click 立刻关掉菜单。
    const timer = window.setTimeout(() => {
      document.addEventListener("mousedown", handlePointerDown);
    }, 0);
    window.addEventListener("resize", updateMenuPosition);
    window.addEventListener("scroll", updateMenuPosition, true);

    return () => {
      window.clearTimeout(timer);
      document.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("resize", updateMenuPosition);
      window.removeEventListener("scroll", updateMenuPosition, true);
    };
  }, [menuMaxHeight, menuPlacement, menuWidth, open]);

  useEffect(() => {
    if (!disabled) return;
    setOpen(false);
  }, [disabled]);

  const currentLabel = selectedOption?.label ?? placeholder ?? "";

  return (
    <div
      ref={rootRef}
      className={[
        "single-select-dropdown",
        disabled ? "disabled" : "",
        className ?? "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <button
        ref={triggerRef}
        type="button"
        className={`single-select-dropdown-trigger${open ? " open" : ""}`}
        aria-label={ariaLabel}
        aria-haspopup="listbox"
        aria-expanded={open}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            setOpen(true);
          }
        }}
        onClick={() => {
          if (disabled) return;
          setOpen((prev) => !prev);
        }}
        disabled={disabled}
      >
        <span className="single-select-dropdown-value" title={currentLabel}>
          {currentLabel}
        </span>
        <span className="single-select-dropdown-arrow">
          <ChevronDown size={16} />
        </span>
      </button>

      {open && menuStyle
        ? createPortal(
            <div
              ref={menuRef}
              className={[
                "single-select-dropdown-menu",
                menuClassName ?? "",
              ]
                .filter(Boolean)
                .join(" ")}
              style={{
                position: "fixed",
                top: menuStyle.top !== undefined ? `${menuStyle.top}px` : "auto",
                bottom: menuStyle.bottom !== undefined ? `${menuStyle.bottom}px` : "auto",
                left: `${menuStyle.left}px`,
                width: `${menuStyle.width}px`,
                maxHeight: `${menuStyle.maxHeight}px`,
                zIndex: menuStyle.zIndex,
              }}
              onKeyDown={(event) => {
                const items = Array.from(menuRef.current?.querySelectorAll<HTMLElement>('[role="option"]') ?? []);
                const index = items.indexOf(document.activeElement as HTMLElement);
                if (event.key === "Tab") { event.preventDefault(); closeMenu(); }
                if (!items.length) return;
                const next = event.key === "Home" ? 0 : event.key === "End" ? items.length - 1
                  : event.key === "ArrowDown" ? (index + 1) % items.length
                  : event.key === "ArrowUp" ? (index - 1 + items.length) % items.length : -1;
                if (next >= 0) { event.preventDefault(); items[next].focus(); }
              }}
              role="listbox"
              aria-label={ariaLabel}
            >
              {options.map((option) => {
                const active = option.value === value;
                return (
                  <button
                    key={option.value}
                    type="button"
                    className={`single-select-dropdown-item${active ? " active" : ""}`}
                    onClick={() => {
                      onChange(option.value);
                      closeMenu();
                    }}
                    role="option"
                    aria-selected={active}
                  >
                    <span
                      className="single-select-dropdown-item-label"
                      title={option.label}
                    >
                      {option.label}
                    </span>
                    <span className="single-select-dropdown-item-check">
                      {active ? <Check size={15} /> : null}
                    </span>
                  </button>
                );
              })}
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}
