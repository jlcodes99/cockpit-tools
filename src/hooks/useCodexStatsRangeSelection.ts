import { useEffect, useMemo, useState } from "react";
import { readCodexStatsRangeSelection } from "../utils/codexStatsRangePreference";

function localDayStart(): number {
  return new Date().setHours(0, 0, 0, 0);
}

export function useCodexStatsRangeSelection(savedRange: string | null, active = true) {
  const [, setObservedDay] = useState(localDayStart);
  const today = localDayStart();
  const selection = useMemo(
    () => readCodexStatsRangeSelection(savedRange, new Date(today)),
    [savedRange, today],
  );
  const followsCalendar = selection.key !== "custom";

  useEffect(() => {
    if (!active || !followsCalendar) return;
    let timer: number | undefined;
    const refresh = () => {
      window.clearTimeout(timer);
      const midnight = localDayStart();
      setObservedDay(midnight);
      const nextDay = new Date(midnight).setHours(24, 0, 0, 0);
      timer = window.setTimeout(refresh, Math.max(1, nextDay - Date.now()));
    };
    const refreshWhenVisible = () => {
      if (document.visibilityState === "visible") refresh();
    };
    refresh();
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refreshWhenVisible);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refreshWhenVisible);
    };
  }, [active, followsCalendar]);

  return selection;
}
