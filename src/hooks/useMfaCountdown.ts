import { useEffect, useState } from 'react';
import { getMfaTimeRemaining } from '../utils/mfaVault';

/** Tick only while a TOTP preview is visible; drafts may persist after closing. */
export function useMfaCountdown(active: boolean): number {
  const [remaining, setRemaining] = useState(getMfaTimeRemaining);
  useEffect(() => {
    if (!active) return;
    // Resuming a dialog uses wall time, not the last hidden countdown value.
    setRemaining(getMfaTimeRemaining());
    const timer = window.setInterval(() => setRemaining(getMfaTimeRemaining()), 1000);
    return () => window.clearInterval(timer);
  }, [active]);
  return remaining;
}
