import { useCallback, useEffect, useRef, useState } from 'react';

const READ_DEADLINE_MS = 5000;

/** An editor may close or retry while a native read is still outstanding. */
export function useAccountFormLoad() {
  const [status, setStatus] = useState<'ready' | 'loading' | 'failed'>('ready');
  const attempt = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const retryRead = useRef<(() => void) | undefined>(undefined);

  const cancel = useCallback(() => {
    attempt.current += 1;
    clearTimeout(timer.current);
    retryRead.current = undefined;
  }, []);
  useEffect(() => cancel, [cancel]);

  const reset = useCallback(() => {
    cancel();
    setStatus('ready');
  }, [cancel]);

  const start = useCallback(<T,>(
    read: () => Promise<T>,
    onRead: (value: T) => void,
    onError: (error?: unknown) => void,
  ) => {
    cancel();
    let pending: Promise<T> | undefined;
    const run = () => {
      const current = ++attempt.current;
      clearTimeout(timer.current);
      onError();
      setStatus('loading');
      pending ??= Promise.resolve().then(read).finally(() => { pending = undefined; });
      const fail = (error?: unknown) => {
        if (current !== attempt.current) return;
        attempt.current += 1;
        clearTimeout(timer.current);
        setStatus('failed');
        onError(error ?? new Error('account_read_timeout'));
      };
      timer.current = setTimeout(fail, READ_DEADLINE_MS);
      void pending.then(value => {
        if (current !== attempt.current) return;
        clearTimeout(timer.current);
        onRead(value);
        setStatus('ready');
      }).catch(fail);
    };
    retryRead.current = run;
    run();
  }, [cancel]);

  const retry = useCallback(() => retryRead.current?.(), []);
  return { status, start, reset, retry };
}
