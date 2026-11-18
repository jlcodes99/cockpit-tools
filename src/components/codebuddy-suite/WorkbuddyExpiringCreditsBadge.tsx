import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { AlarmClock } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { WorkbuddyAccount } from '../../types/workbuddy';
import { EXPIRING_CREDITS_HORIZON_MS, getWorkbuddyExpiringCredits } from '../../utils/workbuddyCredits';

const TICK_MS = 30_000;
const MAX_TIMEOUT_MS = 2_147_483_647;
const TOOLTIP_WIDTH = 272;
const VIEWPORT_PADDING = 12;
const ANCHOR_GAP = 8;

type Placement = { left: number; top: number; bottom?: undefined } | { left: number; top?: undefined; bottom: number };

/**
 * 卡片上的「24 小时内到期」小标签：有即将到期的积分包时才出现，
 * 悬停或键盘聚焦时用浮层列出每个积分包的剩余数量与到期时间。
 */
export function WorkbuddyExpiringCreditsBadge({
  account,
  international = false,
  onClick,
}: {
  account: WorkbuddyAccount;
  international?: boolean;
  onClick?: () => void;
}) {
  const { t, i18n } = useTranslation();
  const locale = i18n.resolvedLanguage ?? i18n.language;
  const [now, setNow] = useState(() => Date.now());
  const [open, setOpen] = useState(false);
  const [placement, setPlacement] = useState<Placement | null>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const tooltip = useRef<HTMLDivElement>(null);
  const tooltipId = useId();

  const expiring = useMemo(
    () => getWorkbuddyExpiringCredits(account, now, EXPIRING_CREDITS_HORIZON_MS, international),
    [account, international, now],
  );
  const hasExpiring = expiring.packages.length > 0;

  // 已到期：每 30 秒走一次，保证浮层时间和「是否还在窗口内」及时更新。
  // 尚未到期：只在「进入 24 小时窗口」的那一刻唤醒一次，避免每张卡片常驻定时器。
  useEffect(() => {
    const tick = () => setNow(Date.now());
    window.addEventListener('focus', tick);
    const timer = hasExpiring ? window.setInterval(tick, TICK_MS) : 0;
    return () => {
      if (timer) window.clearInterval(timer);
      window.removeEventListener('focus', tick);
    };
  }, [hasExpiring]);

  useEffect(() => {
    if (hasExpiring || expiring.nextExpiryAt == null) return;
    const delay = Math.max(0, expiring.nextExpiryAt - now - EXPIRING_CREDITS_HORIZON_MS) + 1_000;
    const timer = window.setTimeout(() => setNow(Date.now()), Math.min(delay, MAX_TIMEOUT_MS));
    return () => window.clearTimeout(timer);
  }, [hasExpiring, expiring.nextExpiryAt, now]);

  useEffect(() => {
    if (!hasExpiring) setOpen(false);
  }, [hasExpiring]);

  useLayoutEffect(() => {
    if (!open) return;
    const anchor = trigger.current;
    if (!anchor) return;
    const update = () => {
      const rect = anchor.getBoundingClientRect();
      // 锚点滚出视口后不再展示浮层，避免 fixed 定位停在边缘。
      if (rect.bottom < 0 || rect.top > window.innerHeight || rect.right < 0 || rect.left > window.innerWidth) {
        setOpen(false);
        return;
      }
      const height = tooltip.current?.offsetHeight ?? 0;
      const left = Math.max(
        VIEWPORT_PADDING,
        Math.min(rect.left, window.innerWidth - TOOLTIP_WIDTH - VIEWPORT_PADDING),
      );
      // 上方空间不足时翻到下方，避免浮层被窗口裁掉。
      const above = rect.top - height - ANCHOR_GAP >= VIEWPORT_PADDING;
      const next: Placement = above
        ? { left, bottom: window.innerHeight - rect.top + ANCHOR_GAP }
        : { left, top: rect.bottom + ANCHOR_GAP };
      setPlacement((previous) => (
        previous && previous.left === next.left
          && previous.top === next.top && previous.bottom === next.bottom
          ? previous : next
      ));
    };
    update();
    window.addEventListener('scroll', update, true);
    window.addEventListener('resize', update);
    return () => {
      window.removeEventListener('scroll', update, true);
      window.removeEventListener('resize', update);
    };
  }, [open, placement, expiring.packages.length]);

  const number = useMemo(
    () => new Intl.NumberFormat(locale, { maximumFractionDigits: 2 }),
    [locale],
  );
  const date = useMemo(
    () => new Intl.DateTimeFormat(locale, { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }),
    [locale],
  );

  if (!hasExpiring) return null;

  return (
    <>
      <button
        ref={trigger}
        type="button"
        className="workbuddy-expiring-badge"
        aria-describedby={open ? tooltipId : undefined}
        onMouseEnter={() => setOpen(true)}
        onMouseLeave={() => setOpen(false)}
        onFocus={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onClick={() => {
          if (!onClick) {
            // 触屏没有 hover：点击切换浮层，键盘用户也能看到明细。
            setOpen((value) => !value);
            return;
          }
          setOpen(false);
          onClick();
        }}
      >
        <AlarmClock size={12} aria-hidden="true" />
        <span>{t('workbuddy.credits.expiringSoon', { amount: number.format(expiring.total) })}</span>
      </button>
      {open && placement && createPortal(
        <div
          id={tooltipId}
          ref={tooltip}
          role="tooltip"
          className="workbuddy-expiring-tooltip"
          style={{ left: placement.left, top: placement.top, bottom: placement.bottom }}
        >
          <div className="workbuddy-expiring-tooltip-title">{t('workbuddy.credits.expiringSoonTitle')}</div>
          <ul className="workbuddy-expiring-tooltip-list">
            {expiring.packages.map((item, index) => (
              <li key={`${item.packageCode}-${index}`}>
                <span className="workbuddy-expiring-tooltip-name">
                  {item.name || t(`workbuddy.credits.${item.group}`)}
                </span>
                <span className="workbuddy-expiring-tooltip-amount">{number.format(item.left)}</span>
                <span className="workbuddy-expiring-tooltip-time">
                  {t('workbuddy.credits.expiresAt', { time: date.format(item.expireAt) })}
                </span>
              </li>
            ))}
          </ul>
          <div className="workbuddy-expiring-tooltip-total">
            {t('workbuddy.credits.expiringSoonSummary', {
              amount: number.format(expiring.total),
              packageCount: expiring.packages.length,
            })}
          </div>
        </div>,
        document.body,
      )}
    </>
  );
}
