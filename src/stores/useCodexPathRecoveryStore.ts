import { create } from 'zustand';

export const CODEX_PATH_RECOVERY_CANCELLED = 'CODEX_START_CANCELLED';

export interface CodexPathRecoveryRequest {
  id: number;
  key: string;
  retry: () => Promise<void>;
  continueWithoutDesktop?: () => Promise<void>;
  cancel: () => void;
}

export interface CodexPathRecoveryOptions<T> {
  /** Include the operation and its target/options, never just the platform name. */
  key: string;
  withoutDesktop?: () => Promise<T>;
}

interface RecoveryEntry {
  id: number;
  key: string;
  operation: () => Promise<unknown>;
  withoutDesktop?: () => Promise<unknown>;
  promise: Promise<unknown>;
  resolve: (value: unknown) => void;
  reject: (error: unknown) => void;
  settled: boolean;
  attempt?: Promise<void>;
}

export const useCodexPathRecoveryStore = create<{ request: CodexPathRecoveryRequest | null }>(() => ({
  request: null,
}));

const operations = new Map<string, RecoveryEntry>();
let sequence = 0;

export function isCodexPathMissingError(error: unknown): boolean {
  const value = typeof error === 'string'
    ? error
    : error && typeof error === 'object' && 'message' in error && typeof error.message === 'string'
      ? error.message
      : '';
  const message = value.trim().replace(/^(?:Error:\s*)+/, '');
  return message === 'APP_PATH_NOT_FOUND:codex'
    || message === 'Codex API Service 已激活，但客户端启动失败: APP_PATH_NOT_FOUND:codex'
    || message === '定位 Codex 运行实例失败: APP_PATH_NOT_FOUND:codex';
}

/** Legacy backend events must not open a second dialog while a managed invoke is in flight. */
export function isCodexPathRecoveryManaged(): boolean {
  return operations.size > 0;
}

export function codexPathRecoveryKey(command: string, args: Record<string, unknown> = {}): string {
  return JSON.stringify([command, Object.keys(args).sort().map((key) => [key, args[key]])]);
}

function remove(entry: RecoveryEntry): void {
  if (operations.get(entry.key) === entry) operations.delete(entry.key);
  if (useCodexPathRecoveryStore.getState().request?.id === entry.id) {
    useCodexPathRecoveryStore.setState({ request: null });
  }
}

function cancel(entry: RecoveryEntry): void {
  if (entry.settled) return;
  entry.settled = true;
  entry.reject(new Error(CODEX_PATH_RECOVERY_CANCELLED));
  // A running native request cannot be withdrawn here. Keep its single-flight
  // reservation until it settles so another click cannot start it twice.
  if (!entry.attempt) remove(entry);
  else if (useCodexPathRecoveryStore.getState().request?.id === entry.id) {
    useCodexPathRecoveryStore.setState({ request: null });
  }
}

function finish(entry: RecoveryEntry, value: unknown): void {
  if (!entry.settled) {
    entry.settled = true;
    entry.resolve(value);
  }
  remove(entry);
}

function retry(entry: RecoveryEntry, operation: () => Promise<unknown>): Promise<void> {
  if (entry.settled || useCodexPathRecoveryStore.getState().request?.id !== entry.id) {
    return Promise.reject(new Error(CODEX_PATH_RECOVERY_CANCELLED));
  }
  if (entry.attempt) return entry.attempt;
  const attempt = Promise.resolve().then(() => {
    if (entry.settled || useCodexPathRecoveryStore.getState().request?.id !== entry.id) {
      throw new Error(CODEX_PATH_RECOVERY_CANCELLED);
    }
    return operation();
  }).then((value) => finish(entry, value));
  entry.attempt = attempt.finally(() => {
    entry.attempt = undefined;
    if (entry.settled) remove(entry);
  });
  return entry.attempt;
}

function show(entry: RecoveryEntry): void {
  if (entry.settled) return;
  const current = useCodexPathRecoveryStore.getState().request;
  if (current && current.id !== entry.id) {
    // Do not replace an account/instance's pending transaction or queue an
    // implicit launch after the user closes a different recovery dialog.
    cancel(entry);
    return;
  }
  useCodexPathRecoveryStore.setState({ request: {
    id: entry.id,
    key: entry.key,
    retry: () => retry(entry, entry.operation),
    continueWithoutDesktop: entry.withoutDesktop
      ? () => retry(entry, entry.withoutDesktop!)
      : undefined,
    cancel: () => cancel(entry),
  } });
}

function createEntry<T>(operation: () => Promise<T>, options: CodexPathRecoveryOptions<T>): RecoveryEntry {
  let resolve!: (value: unknown) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<unknown>((yes, no) => { resolve = yes; reject = no; });
  const entry: RecoveryEntry = {
    id: ++sequence, key: options.key, operation, withoutDesktop: options.withoutDesktop,
    promise, resolve, reject, settled: false,
  };
  operations.set(entry.key, entry);
  return entry;
}

/** An event has already reported a failure; wait for an explicit repair action. */
export function requestCodexPathRecovery<T>(
  operation: () => Promise<T>, options: CodexPathRecoveryOptions<T>,
): Promise<T> {
  const existing = operations.get(options.key);
  if (existing) return existing.promise as Promise<T>;
  if (useCodexPathRecoveryStore.getState().request) {
    return Promise.reject(new Error(CODEX_PATH_RECOVERY_CANCELLED));
  }
  const entry = createEntry(operation, options);
  show(entry);
  return entry.promise as Promise<T>;
}

/** Resolve only the original operation's real result, including after a repair. */
export function withCodexPathRecovery<T>(
  operation: () => Promise<T>, options: CodexPathRecoveryOptions<T>,
): Promise<T> {
  const existing = operations.get(options.key);
  if (existing) return existing.promise as Promise<T>;
  if (useCodexPathRecoveryStore.getState().request) {
    return Promise.reject(new Error(CODEX_PATH_RECOVERY_CANCELLED));
  }
  const entry = createEntry(operation, options);
  void Promise.resolve().then(operation).then(
    (value) => finish(entry, value),
    (error: unknown) => {
      if (isCodexPathMissingError(error)) show(entry);
      else {
        entry.settled = true;
        entry.reject(error);
        remove(entry);
      }
    },
  );
  return entry.promise as Promise<T>;
}
