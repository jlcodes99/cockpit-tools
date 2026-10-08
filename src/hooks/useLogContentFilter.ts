import { useEffect, useState } from 'react';
import { startLogFilter, type LogContentFilterRequest } from '../utils/logContentFilter';

export function useLogContentFilter(open: boolean, request: LogContentFilterRequest) {
  const [result, setResult] = useState<{
    content: string;
    error?: 'invalidRegex' | 'timeout' | 'unavailable';
    loading: boolean;
  }>({ content: '', loading: false });
  const { content, level, pattern, regex } = request;
  useEffect(() => {
    if (!open) return;
    if (level === 'ALL' && !pattern.trim()) {
      setResult({ content, loading: false });
      return;
    }
    setResult((previous) => ({ content: previous.content, loading: true }));
    let stop: (() => void) | undefined;
    const timer = setTimeout(() => {
      try {
        const worker = new Worker(new URL('../workers/logFilter.worker.ts', import.meta.url), { type: 'module' });
        stop = startLogFilter(worker, { content, level, pattern, regex }, (next) => {
          setResult({ ...next, loading: false });
        });
      } catch {
        setResult({ content: '', error: 'unavailable', loading: false });
      }
    }, 120);
    return () => { clearTimeout(timer); stop?.(); };
  }, [open, content, level, pattern, regex]);
  return result;
}
