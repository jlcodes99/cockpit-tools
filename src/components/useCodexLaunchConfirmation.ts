import { useEffect, useRef, useState } from 'react';
import { listInstancesForLaunchConfirmation } from '../services/codexInstanceService';
import { confirmedCodexLaunch } from '../utils/codexLaunchConfirmation';

interface ConfirmationTarget {
  instanceId: string;
  excludedPids?: number[];
  steps: { id: string; details: Record<string, unknown> }[];
}

/** Read-only reconciliation. Never writes credentials or launches/stops a client. */
export function useCodexLaunchConfirmation<T extends ConfirmationTarget>(
  target: T | null, onConfirmed: (target: T) => void,
) {
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<'notRunning' | 'checkFailed' | null>(null);
  const current = useRef(target);
  const callback = useRef(onConfirmed);
  const inFlight = useRef<Promise<boolean> | null>(null);
  current.current = target;
  callback.current = onConfirmed;

  const check = (manual = true): Promise<boolean> => {
    if (!target) return Promise.resolve(false);
    if (inFlight.current) return inFlight.current;
    setChecking(true);
    setError(null);
    const task = (async () => {
      let timeout: ReturnType<typeof setTimeout> | undefined;
      try {
        const instances = await Promise.race([
          listInstancesForLaunchConfirmation(),
          new Promise<never>((_, reject) => {
            timeout = setTimeout(() => reject(new Error('status query timed out')), 8_000);
          }),
        ]);
        if (current.current !== target) return false;
        const directory = target.steps.find((step) => step.id === 'checkInstance')?.details.userDataDir;
        const instance = confirmedCodexLaunch(instances, target.instanceId,
          typeof directory === 'string' ? directory : undefined, target.excludedPids);
        if (instance) {
          callback.current(target);
          return true;
        }
        if (manual) setError('notRunning');
      } catch {
        if (manual && current.current === target) setError('checkFailed');
      } finally {
        if (timeout) clearTimeout(timeout);
        inFlight.current = null;
        if (current.current === target) setChecking(false);
      }
      return false;
    })();
    inFlight.current = task;
    return task;
  };

  useEffect(() => {
    setChecking(false);
    setError(null);
    if (!target) return;
    current.current = target;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const deadline = Date.now() + 30_000;
    const poll = async () => {
      if (disposed || current.current !== target || Date.now() >= deadline) return;
      const confirmed = await check(false);
      if (!confirmed && !disposed && current.current === target && Date.now() < deadline) {
        timer = setTimeout(() => void poll(), 2_000);
      }
    };
    void poll();
    return () => {
      disposed = true;
      if (timer) clearTimeout(timer);
      // An outstanding native query may finish later, but cannot restore a closed dialog.
      if (current.current === target) current.current = null;
    };
  }, [target]);

  return { checking, error, check };
}
