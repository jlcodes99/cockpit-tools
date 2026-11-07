export type LogLevelFilter = 'ALL' | 'INFO' | 'WARN' | 'ERROR';
export interface LogContentFilterRequest {
  content: string;
  level: LogLevelFilter;
  pattern: string;
  regex: boolean;
}
export interface LogContentFilterResult {
  content: string;
  error?: 'invalidRegex';
}
const ENTRY_LEVEL = /^\S+\s+(INFO|WARN|ERROR)\s/;

// Preserve multiline entries so a match in a stack trace retains its timestamp.
export function filterLogContent(request: LogContentFilterRequest): LogContentFilterResult {
  const pattern = request.pattern.trim();
  let matches = (_entry: string) => true;
  if (pattern) {
    if (request.regex) {
      try {
        const expression = new RegExp(pattern, 'i');
        matches = (entry) => expression.test(entry);
      } catch {
        return { content: '', error: 'invalidRegex' };
      }
    } else {
      const lower = pattern.toLowerCase();
      matches = (entry) => entry.toLowerCase().includes(lower);
    }
  }
  if (request.level === 'ALL' && !pattern) return { content: request.content };
  const result: string[] = [];
  let entry: string[] = [];
  let level: string | undefined;
  const flush = () => {
    const text = entry.join('\n');
    if (entry.length && (request.level === 'ALL' || request.level === level) && matches(text)) result.push(text);
    entry = [];
  };
  for (const line of request.content.split('\n')) {
    const nextLevel = line.match(ENTRY_LEVEL)?.[1];
    if (nextLevel) {
      flush();
      level = nextLevel;
    }
    entry.push(line);
  }
  flush();
  return { content: result.join('\n') };
}

export interface LogFilterWorker {
  onmessage: ((event: MessageEvent<LogContentFilterResult>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  postMessage: (request: LogContentFilterRequest) => void;
  terminate: () => void;
}

// Terminating the worker interrupts even catastrophic regular-expression backtracking.
export function startLogFilter(
  worker: LogFilterWorker,
  request: LogContentFilterRequest,
  onResult: (result: Omit<LogContentFilterResult, 'error'> & { error?: 'invalidRegex' | 'timeout' | 'unavailable' }) => void,
  timeoutMs = 1000,
): () => void {
  let active = true;
  const stop = () => {
    if (!active) return;
    active = false;
    clearTimeout(timer);
    worker.onmessage = null;
    worker.onerror = null;
    worker.terminate();
  };
  const finish = (result: Parameters<typeof onResult>[0]) => {
    if (!active) return;
    stop();
    onResult(result);
  };
  const timer = setTimeout(() => finish({ content: '', error: 'timeout' }), timeoutMs);
  worker.onmessage = (event) => finish(event.data);
  worker.onerror = () => finish({ content: '', error: 'unavailable' });
  try { worker.postMessage(request); } catch { finish({ content: '', error: 'unavailable' }); }
  return stop;
}
