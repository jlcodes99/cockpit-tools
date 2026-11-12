const INITIAL_DELAY_MS = 6000;
const PLATFORM_INTERVAL_MS = 1200;
let accountPrefetch: Promise<unknown> | undefined;

export function prefetchDashboardAccounts(tasks: ReadonlyArray<() => Promise<unknown>>) {
  accountPrefetch ??= Promise.allSettled(tasks.map((task, index) =>
    new Promise<void>((resolve) => {
      window.setTimeout(resolve, INITIAL_DELAY_MS + index * PLATFORM_INTERVAL_MS);
    }).then(task),
  ));
  return accountPrefetch;
}
