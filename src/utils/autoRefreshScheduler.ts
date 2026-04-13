export interface AutoRefreshSchedulerTask {
  key: string;
  label: string;
  intervalMs: number;
  run: () => Promise<void>;
  shouldSkip?: () => boolean;
  initialDelayMs?: number;
}

export interface AutoRefreshSchedulerOptions {
  tickMs?: number;
  maxConcurrent?: number;
}

export interface AutoRefreshSchedulerHandle {
  start: () => void;
  stop: () => void;
}

const DEFAULT_TICK_MS = 5_000;
const DEFAULT_MAX_CONCURRENT = 1;
const INITIAL_DELAY_WINDOW_RATIO = 0.8;
const MIN_INITIAL_DELAY_RATIO = 0.05;

interface RuntimeTask extends AutoRefreshSchedulerTask {
  nextRunAt: number;
  running: boolean;
}

function clampIntervalMs(intervalMs: number): number {
  return Math.max(intervalMs, DEFAULT_TICK_MS);
}

function stableHash(value: string): number {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = ((hash << 5) - hash + value.charCodeAt(index)) >>> 0;
  }
  return hash >>> 0;
}

function buildInitialDelayMs(task: AutoRefreshSchedulerTask, tickMs: number): number {
  if (typeof task.initialDelayMs === 'number' && Number.isFinite(task.initialDelayMs)) {
    return Math.max(tickMs, Math.floor(task.initialDelayMs));
  }

  const intervalMs = clampIntervalMs(task.intervalMs);
  const maxSpreadMs = Math.max(Math.floor(intervalMs * INITIAL_DELAY_WINDOW_RATIO), tickMs);
  const minDelayMs = Math.min(
    maxSpreadMs,
    Math.max(Math.floor(intervalMs * MIN_INITIAL_DELAY_RATIO), tickMs),
  );

  if (maxSpreadMs <= minDelayMs) {
    return minDelayMs;
  }

  const spreadRange = maxSpreadMs - minDelayMs + 1;
  return minDelayMs + (stableHash(task.key) % spreadRange);
}

export function createAutoRefreshScheduler(
  tasks: AutoRefreshSchedulerTask[],
  options: AutoRefreshSchedulerOptions = {},
): AutoRefreshSchedulerHandle {
  const tickMs = Math.max(1_000, options.tickMs ?? DEFAULT_TICK_MS);
  const maxConcurrent = Math.max(1, options.maxConcurrent ?? DEFAULT_MAX_CONCURRENT);

  let stopped = false;
  let timerId: number | null = null;
  let activeCount = 0;

  const runtimeTasks: RuntimeTask[] = tasks
    .filter((task) => task.intervalMs > 0)
    .map((task) => ({
      ...task,
      nextRunAt: Date.now() + buildInitialDelayMs(task, tickMs),
      running: false,
    }));

  const scheduleDueTasks = () => {
    if (stopped || activeCount >= maxConcurrent) {
      return;
    }

    const now = Date.now();
    const dueTasks = runtimeTasks
      .filter((task) => !task.running && task.nextRunAt <= now)
      .sort((left, right) => {
        if (left.nextRunAt !== right.nextRunAt) {
          return left.nextRunAt - right.nextRunAt;
        }
        return left.key.localeCompare(right.key);
      });

    for (const task of dueTasks) {
      if (stopped || activeCount >= maxConcurrent) {
        break;
      }

      if (task.shouldSkip?.()) {
        task.nextRunAt = Date.now() + clampIntervalMs(task.intervalMs);
        continue;
      }

      task.running = true;
      task.nextRunAt = Date.now() + clampIntervalMs(task.intervalMs);
      activeCount += 1;

      void Promise.resolve()
        .then(() => task.run())
        .finally(() => {
          task.running = false;
          activeCount = Math.max(0, activeCount - 1);
          if (!stopped) {
            scheduleDueTasks();
          }
        });
    }
  };

  return {
    start() {
      if (stopped || timerId !== null || runtimeTasks.length === 0) {
        return;
      }
      scheduleDueTasks();
      timerId = window.setInterval(scheduleDueTasks, tickMs);
    },
    stop() {
      stopped = true;
      if (timerId !== null) {
        window.clearInterval(timerId);
        timerId = null;
      }
    },
  };
}
