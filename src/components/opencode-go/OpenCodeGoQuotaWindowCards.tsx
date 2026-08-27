import { useEffect, useMemo, useState } from 'react';
import type { OpenCodeGoQuotaSnapshot } from '../../types/openCodeGo';
import './OpenCodeGoQuotaWindowCards.css';
import {
  buildOpenCodeGoQuotaCardStates,
  type OpenCodeGoQuotaWindowId,
} from './openCodeGoQuotaCardState';

export interface OpenCodeGoQuotaWindowCardsProps {
  quota?: Partial<OpenCodeGoQuotaSnapshot> | null;
  loadingWindows?: readonly OpenCodeGoQuotaWindowId[];
  errors?: Partial<Record<OpenCodeGoQuotaWindowId, string | null>>;
  className?: string;
  nowMs?: number;
}

function classNames(...values: Array<string | undefined | false>): string {
  return values.filter(Boolean).join(' ');
}

/**
 * Renders every Go quota window independently so a missing, loading, or failed
 * window never suppresses usable sibling data.
 */
export function OpenCodeGoQuotaWindowCards({
  quota,
  loadingWindows,
  errors,
  className,
  nowMs: controlledNowMs,
}: OpenCodeGoQuotaWindowCardsProps) {
  const [liveNowMs, setLiveNowMs] = useState(() => Date.now());

  useEffect(() => {
    if (controlledNowMs != null) return;
    const timer = window.setInterval(() => setLiveNowMs(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, [controlledNowMs]);

  const nowMs = controlledNowMs ?? liveNowMs;
  const cards = useMemo(
    () =>
      buildOpenCodeGoQuotaCardStates({
        windows: quota ?? undefined,
        loadingWindows,
        errors,
        nowMs,
      }),
    [errors, loadingWindows, nowMs, quota],
  );

  return (
    <div
      className={classNames('opencode-go-quota-cards', className)}
      aria-live="polite"
    >
      {cards.map((card) => (
        <section
          key={card.id}
          className={classNames(
            'opencode-go-quota-card',
            `is-${card.status}`,
          )}
          aria-label={`${card.label}: ${card.percentageText}`}
        >
          <header>
            <span>{card.label}</span>
            <strong>{card.percentageText}</strong>
          </header>
          <div
            className="opencode-go-quota-track"
            role={card.status === 'ready' ? 'progressbar' : undefined}
            aria-label={
              card.status === 'ready'
                ? `${card.label} quota remaining`
                : undefined
            }
            aria-valuemin={card.status === 'ready' ? 0 : undefined}
            aria-valuemax={card.status === 'ready' ? 100 : undefined}
            aria-valuenow={card.remainingPercent ?? undefined}
          >
            <span style={{ width: `${card.remainingPercent ?? 0}%` }} />
          </div>
          <footer>
            <span>{card.resetText || '\u00a0'}</span>
            {card.resetsAt != null ? (
              <time dateTime={new Date(card.resetsAt * 1000).toISOString()}>
                {new Date(card.resetsAt * 1000).toLocaleString()}
              </time>
            ) : null}
          </footer>
        </section>
      ))}
    </div>
  );
}
