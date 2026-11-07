/** Keep only the latest value, with a bounded delay even under continuous updates. */
export function createCoalescedWriter<T>(write: (value: T) => void, delayMs = 500) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let pending: { value: T } | undefined;
  const flush = () => {
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    const latest = pending;
    pending = undefined;
    if (latest) write(latest.value);
  };
  return {
    schedule(value: T) {
      pending = { value };
      // Do not restart the timer: a busy refresh must still persist periodically.
      if (timer === undefined) timer = setTimeout(flush, delayMs);
    },
    flush,
  };
}
