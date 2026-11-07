import { useEffect, useRef, useState } from 'react';

/** Sample a hot revision stream without postponing refresh forever while writes
 * continue. Range/filter changes can still trigger their own immediate query. */
export function useSampledRevision(revision: number | undefined, intervalMs = 1000) {
  const latest = useRef(revision);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [sampled, setSampled] = useState(revision);
  useEffect(() => {
    latest.current = revision;
    if (timer.current !== null) return;
    timer.current = setTimeout(() => {
      timer.current = null;
      setSampled(latest.current);
    }, intervalMs);
  }, [revision, intervalMs]);
  useEffect(() => () => {
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
  }, []);
  return sampled;
}
