import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown, Zap } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { CodexAppSpeed } from "../../types/codex";

type SpeedMenuPlacement = "top" | "bottom";

type SpeedMenuPosition = {
  top: number;
  left: number;
  width: number;
  placement: SpeedMenuPlacement;
};

interface CodexSpeedSelectProps {
  value?: CodexAppSpeed | null;
  onChange: (speed: CodexAppSpeed) => void | Promise<void>;
  disabled?: boolean;
  busy?: boolean;
  compact?: boolean;
  className?: string;
  preferredPlacement?: SpeedMenuPlacement;
  ariaLabel?: string;
}

const SPEED_MENU_WIDTH = 206;
const SPEED_MENU_HEIGHT = 122;
const SPEED_MENU_COMPACT_WIDTH = 180;
const SPEED_MENU_COMPACT_HEIGHT = 104;
const SPEED_MENU_GAP = 5;
const SPEED_MENU_MARGIN = 8;
const SPEED_MENU_Z_INDEX = 10030;

function resolveSpeedMenuPosition(
  trigger: HTMLElement | null,
  preferredPlacement: SpeedMenuPlacement,
  compact: boolean,
): SpeedMenuPosition | null {
  if (!trigger) return null;
  const rect = trigger.getBoundingClientRect();
  const menuWidth = compact ? SPEED_MENU_COMPACT_WIDTH : SPEED_MENU_WIDTH;
  const menuHeight = compact ? SPEED_MENU_COMPACT_HEIGHT : SPEED_MENU_HEIGHT;
  const width = Math.max(menuWidth, rect.width);
  let placement = preferredPlacement;
  let top =
    preferredPlacement === "top"
      ? rect.top - menuHeight - SPEED_MENU_GAP
      : rect.bottom + SPEED_MENU_GAP;

  if (top < SPEED_MENU_MARGIN) {
    placement = "bottom";
    top = rect.bottom + SPEED_MENU_GAP;
  }
  if (top + menuHeight > window.innerHeight - SPEED_MENU_MARGIN) {
    placement = "top";
    top = rect.top - menuHeight - SPEED_MENU_GAP;
  }

  const maxLeft = Math.max(
    SPEED_MENU_MARGIN,
    window.innerWidth - width - SPEED_MENU_MARGIN,
  );
  const left = Math.min(
    Math.max(SPEED_MENU_MARGIN, rect.right - width),
    maxLeft,
  );

  return {
    top: Math.max(SPEED_MENU_MARGIN, top),
    left,
    width,
    placement,
  };
}

export function CodexSpeedSelect({
  value,
  onChange,
  disabled = false,
  busy = false,
  compact = false,
  className = "",
  preferredPlacement = "top",
  ariaLabel,
}: CodexSpeedSelectProps) {
  const { t } = useTranslation();
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<SpeedMenuPosition | null>(null);
  const speed = value ?? "standard";

  const options = useMemo(
    () => [
      {
        value: "standard" as CodexAppSpeed,
        label: t("codex.speed.standard", "标准"),
        desc: t("codex.speed.standardDesc", "默认速度，常规用量"),
      },
      {
        value: "fast" as CodexAppSpeed,
        label: t("codex.speed.fast", "快速"),
        desc: t("codex.speed.fastDesc", "1.5 倍速，用量增加"),
      },
    ],
    [t],
  );

  const selected = options.find((item) => item.value === speed) ?? options[0];
  const selectedTitle = `${t("codex.speed.title", "速度")}：${selected.label} - ${
    selected.desc
  }`;

  const updatePosition = useCallback(() => {
    setPosition(resolveSpeedMenuPosition(triggerRef.current, preferredPlacement, compact));
  }, [compact, preferredPlacement]);

  useEffect(() => {
    if (!open) return;
    updatePosition();

    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) {
        return;
      }
      setOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpen(false);
      }
    };

    document.addEventListener("mousedown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
    };
  }, [open, updatePosition]);

  useEffect(() => {
    if (disabled || busy) {
      setOpen(false);
    }
  }, [busy, disabled]);

  const handleSelect = (nextSpeed: CodexAppSpeed) => {
    setOpen(false);
    if (nextSpeed === speed || disabled || busy) return;
    void onChange(nextSpeed);
  };

  return (
    <span className={`codex-speed-select ${className}`.trim()}>
      <button
        ref={triggerRef}
        type="button"
        className={`codex-speed-trigger ${speed} ${compact ? "compact" : ""} ${
          open ? "open" : ""
        }`}
        onClick={() => {
          if (disabled || busy) return;
          setOpen((prev) => !prev);
        }}
        disabled={disabled || busy}
        title={selectedTitle}
        aria-label={ariaLabel || t("codex.speed.title", "速度")}
      >
        {speed === "fast" && <Zap size={12} />}
        <span>{selected.label}</span>
        {!compact && <ChevronDown size={12} className="codex-speed-caret" />}
      </button>
      {open && position
        ? createPortal(
            <div
              ref={menuRef}
              className={`codex-speed-menu placement-${position.placement} ${
                compact ? "compact" : ""
              }`}
              style={{
                position: "fixed",
                top: `${position.top}px`,
                left: `${position.left}px`,
                width: `${position.width}px`,
                zIndex: SPEED_MENU_Z_INDEX,
              }}
            >
              {!compact && (
                <div className="codex-speed-menu-title">
                  {t("codex.speed.title", "速度")}
                </div>
              )}
              <div className="codex-speed-menu-options">
                {options.map((option) => (
                  <button
                    key={option.value}
                    type="button"
                    className={`codex-speed-option ${
                      option.value === speed ? "active" : ""
                    }`}
                    onClick={() => handleSelect(option.value)}
                  >
                    <span className="codex-speed-option-text">
                      <span className="codex-speed-option-label">
                        {option.label}
                      </span>
                      <span className="codex-speed-option-desc">
                        {option.desc}
                      </span>
                    </span>
                    {option.value === speed && <Check size={15} />}
                  </button>
                ))}
              </div>
            </div>,
            document.body,
          )
        : null}
    </span>
  );
}
