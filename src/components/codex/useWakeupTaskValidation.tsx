import { useCallback, useEffect, useRef, useState } from 'react';
import { scrollElementIntoView } from '../../utils/reducedMotion';
import './CodexWakeupValidation.css';

type TaskField = 'name' | 'accounts' | 'model' | 'reasoning' | 'weeklyDays';

/** Field validation stays beside the field; backend failures use the modal's fixed error area. */
export function useWakeupTaskValidation(draft: unknown, open: boolean) {
  const dialog = useRef<HTMLDivElement>(null);
  const frame = useRef<number | undefined>(undefined);
  const [error, setError] = useState<{ field: TaskField; message: string } | null>(null);
  useEffect(() => {
    setError(null);
    return () => { if (frame.current !== undefined) window.cancelAnimationFrame(frame.current); };
  }, [draft, open]);
  const report = useCallback((field: TaskField, message: string) => {
    setError({ field, message });
    if (frame.current !== undefined) window.cancelAnimationFrame(frame.current);
    frame.current = window.requestAnimationFrame(() => {
      const group = dialog.current?.querySelector<HTMLElement>(`[data-wakeup-field="${field}"]`);
      if (!group) return;
      scrollElementIntoView(group, { block: 'nearest' });
      (group.querySelector<HTMLElement>('input:not(:disabled), button:not(:disabled), select:not(:disabled)') ?? group).focus({ preventScroll: true });
    });
  }, []);
  const clear = useCallback(() => {
    setError(null);
    if (frame.current !== undefined) window.cancelAnimationFrame(frame.current);
  }, []);
  const message = (field: TaskField) => error?.field === field
    ? <p id={`wakeup-field-${field}-error`} className="codex-wakeup-field-error" role="alert">{error.message}</p>
    : null;
  return { dialog, report, clear, message, invalid: (field: TaskField) => error?.field === field };
}
