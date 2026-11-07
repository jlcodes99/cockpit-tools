import { useEffect, useLayoutEffect } from 'react';
import { acquireModalScrollLock } from '../utils/modalScrollLock';

const useIsomorphicLayoutEffect = typeof window === 'undefined' ? useEffect : useLayoutEffect;

export function useModalScrollLock(open: boolean): void {
  useIsomorphicLayoutEffect(() => {
    if (!open || typeof document === 'undefined') return;
    return acquireModalScrollLock(document);
  }, [open]);
}
