import { useEffect, useMemo, useRef, useState, type PointerEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import { CodexQuotaHistoryDetails } from "./CodexQuotaHistoryDetails";
import { CodexQuotaHistorySettings } from "./CodexQuotaHistorySettings";
import { useCodexAccountStore } from "../../stores/useCodexAccountStore";
import { isCodexApiKeyAccount, type CodexAccount } from "../../types/codex";
import { subscribeTauriEvent } from "../../utils/tauriEventListener";
import {
  nearestQuotaHistoryPoint, QUOTA_PLOT, quotaHistoryDateRange, quotaHistorySeries, quotaHistoryTicks,
  type QuotaHistoryPoint, type QuotaHistoryRange,
} from "./codexQuotaHistoryChart";
import "./CodexQuotaHistoryPage.css";

const PERIODS: QuotaHistoryRange[] = ["today", "thisWeek", "thisMonth"];
const MINUTE = 60_000;
const SERIES = [
  { key: "shortRemaining", label: "codex.thresholds.shortCycle", css: "short" },
  { key: "weeklyRemaining", label: "codex.thresholds.weekly", css: "weekly" },
] as const;

export function CodexQuotaHistoryPage({ accounts, resolveDisplayName }: {
  accounts: CodexAccount[];
  resolveDisplayName: (account: CodexAccount) => string;
}) {
  const { t, i18n } = useTranslation();
  const lang = i18n.resolvedLanguage || i18n.language;
  const currentAccountId = useCodexAccountStore((state) => state.currentAccount?.id);
  const refreshQuota = useCodexAccountStore((state) => state.refreshQuota);
  const [preferredAccountId, setPreferredAccountId] = useState<string>();
  const [view, setView] = useState<"chart" | "details">("chart");
  const [onlyChanges, setOnlyChanges] = useState(false);
  const [period, setPeriod] = useState<QuotaHistoryRange>("today");
  const [offset, setOffset] = useState(0);
  const [interval, setInterval] = useState(5);
  const [intervalInput, setIntervalInput] = useState("5");
  const intervalValid = /^\d+$/.test(intervalInput) && Number(intervalInput) >= 1 && Number(intervalInput) <= 1440;
  const [retry, setRetry] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshError, setRefreshError] = useState<{ accountId: string; message: string }>();
  const [result, setResult] = useState<{ key: string; points?: QuotaHistoryPoint[] }>();
  const eligible = accounts.filter((account) => !isCodexApiKeyAccount(account));
  const selectedAccount = eligible.find((account) => account.id === preferredAccountId)
    ?? eligible.find((account) => account.id === currentAccountId) ?? eligible[0];
  const accountId = selectedAccount?.id;
  const { from, to } = quotaHistoryDateRange(period, offset);
  const key = JSON.stringify([accountId, selectedAccount?.created_at, selectedAccount?.user_id,
    selectedAccount?.account_id, selectedAccount?.organization_id, from, to, interval, retry]);

  useEffect(() => {
    if (!accountId) return;
    let current = true;
    let sequence = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = async () => {
      const request = ++sequence;
      try {
        const points = await invoke<QuotaHistoryPoint[]>("get_codex_quota_history", {
          accountId, from, to, intervalMinutes: interval,
        });
        if (current && request === sequence) setResult({ key, points });
      } catch (error) {
        if (current && request === sequence) {
          console.warn("[Codex quota history] Query failed", error);
          setResult({ key });
        }
      }
    };
    const stop = subscribeTauriEvent<string[]>("codex:quota-history-recorded", ({ payload }) => {
      if (offset !== 0 || !payload.includes(accountId)) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => { void read(); }, 250);
    });
    const stopCleanup = subscribeTauriEvent("codex:quota-history-cleaned", () => {
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => { void read(); }, 250);
    });
    timer = setTimeout(() => { void read(); }, 100);
    return () => {
      current = false;
      if (timer) clearTimeout(timer);
      stop();
      stopCleanup();
    };
  }, [accountId, from, to, offset, interval, key]);

  const visible = result?.key === key ? result : undefined;
  const refresh = async () => {
    if (!accountId || refreshing) return;
    setRefreshError(undefined);
    if (offset !== 0) {
      setRetry((value) => value + 1);
      return;
    }
    setRefreshing(true);
    try {
      await refreshQuota(accountId);
      setRetry((value) => value + 1);
    } catch (error) {
      setRefreshError({ accountId, message: t("codex.refreshFailed", { error: String(error) }) });
    } finally {
      setRefreshing(false);
    }
  };
  const start = new Date(from);
  const end = new Date(to - 1);
  const fullDate = new Intl.DateTimeFormat(lang, { dateStyle: "short" });
  const rangeLabel = period === "today" ? new Intl.DateTimeFormat(lang, { dateStyle: "medium" }).format(start)
    : period === "thisMonth" ? new Intl.DateTimeFormat(lang, { year: "numeric", month: "long" }).format(start)
    : `${fullDate.format(start)} – ${start.getFullYear() === end.getFullYear()
      ? new Intl.DateTimeFormat(lang, { month: "numeric", day: "numeric" }).format(end) : fullDate.format(end)}`;
  return (
    <section className="codex-quota-history">
      <header className="codex-quota-history__header">
        <h2>{t("codex.quotaHistory.title")}</h2>
        {accountId && <div className="codex-quota-history__header-controls">
          <SingleSelectDropdown value={accountId} onChange={setPreferredAccountId}
            className="codex-quota-history__account"
            ariaLabel={t("codex.quotaHistory.account")}
            options={eligible.map((account) => ({ value: account.id, label: resolveDisplayName(account) }))} />
          <div className="codex-quota-history__periods" role="group" aria-label={t("codex.quotaHistory.title")}>
            <button type="button" className="codex-quota-history__control" aria-pressed={view === "chart"}
              onClick={() => setView("chart")}>{t("codex.quotaHistory.chart")}</button>
            <button type="button" className="codex-quota-history__control" aria-pressed={view === "details"}
              onClick={() => setView("details")}>{t("codex.quotaHistory.details")}</button>
          </div>
        </div>}
      </header>
      <CodexQuotaHistorySettings />
      {!accountId ? <p className="codex-quota-history__message">{t("codex.quotaHistory.noAccount")}</p> : <>
        <div className="codex-quota-history__filters">
          <div className="codex-quota-history__date-controls">
            <div className="codex-quota-history__periods">
              {PERIODS.map((value) => (
                <button type="button" key={value} className="codex-quota-history__control"
                  aria-pressed={period === value} onClick={() => { setPeriod(value); setOffset(0); }}>
                  {value === "today" ? t("codex.localAccess.statsRange.daily")
                    : value === "thisWeek" ? t("codex.localAccess.statsRange.weekly")
                    : t("codex.localAccess.statsRange.monthly")}
                </button>
              ))}
            </div>
            <div className="codex-quota-history__range">
              <button type="button" className="codex-quota-history__control" aria-label={t("pagination.prev")}
                onClick={() => setOffset((value) => value - 1)}>
                <ChevronLeft size={16} />
              </button>
              <span>{rangeLabel}</span>
              <button type="button" className="codex-quota-history__control" aria-label={t("pagination.next")}
                disabled={offset === 0} onClick={() => setOffset((value) => Math.min(0, value + 1))}>
                <ChevronRight size={16} />
              </button>
            </div>
            <button type="button" className="codex-quota-history__control codex-quota-history__refresh"
              aria-label={t("common.refresh")} title={t("common.refresh")}
              disabled={refreshing} aria-busy={refreshing} onClick={() => { void refresh(); }}>
              <RefreshCw size={15} />
            </button>
          </div>
          <label className="codex-quota-history__interval">
            <span>{t("codex.quotaHistory.interval")}</span>
            <input type="text" inputMode="numeric" value={intervalInput}
              aria-invalid={!intervalValid}
              aria-describedby={intervalValid ? undefined : "codex-quota-history-interval-error"}
              onChange={(event) => {
                const value = event.target.value;
                setIntervalInput(value);
                const minutes = Number(value);
                if (/^\d+$/.test(value) && minutes >= 1 && minutes <= 1440) setInterval(minutes);
              }} />
            {!intervalValid && <span id="codex-quota-history-interval-error" role="alert">
              {t("codex.quotaHistory.intervalInvalid")}
            </span>}
          </label>
        </div>
        {offset === 0 && refreshError?.accountId === accountId && <p role="alert" className="codex-quota-history__error">
          {refreshError.message}
        </p>}
        {!visible ? <p role="status" className="codex-quota-history__message">{t("common.loading")}</p>
          : !visible.points ? <div role="alert" className="codex-quota-history__message">
            {t("codex.quotaHistory.readFailed")}
            <button type="button" className="btn btn-secondary" onClick={() => setRetry((value) => value + 1)}>{t("common.retry")}</button>
          </div>
          : view === "details" ? <CodexQuotaHistoryDetails key={key} points={visible.points}
            onlyChanges={onlyChanges} onOnlyChangesChange={setOnlyChanges} />
          : visible.points.length === 0 ? <p className="codex-quota-history__message">{t("codex.quotaHistory.empty")}</p>
          : <QuotaHistoryChart key={key} points={visible.points} period={period} interval={interval} />}
      </>}
    </section>
  );
}

function QuotaHistoryChart({ points, period, interval }: {
  points: QuotaHistoryPoint[];
  period: QuotaHistoryRange;
  interval: number;
}) {
  const { t, i18n } = useTranslation();
  const lang = i18n.resolvedLanguage || i18n.language;
  const [active, setActive] = useState<number | null>(null);
  const chartRef = useRef<SVGSVGElement>(null);
  const [width, setWidth] = useState(QUOTA_PLOT.width);
  const { left, right, top, bottom } = QUOTA_PLOT;
  const height = width < 440 ? 204 : QUOTA_PLOT.height;
  useEffect(() => {
    const chart = chartRef.current;
    if (!chart) return;
    const resize = () => {
      const measured = Math.round(chart.getBoundingClientRect().width);
      if (measured > 0) setWidth(Math.max(left + right + 1, measured));
    };
    resize();
    const observer = new ResizeObserver(resize);
    observer.observe(chart);
    return () => observer.disconnect();
  }, [left, right]);
  const xWidth = width - left - right;
  const first = points[0].observedAt;
  const last = points[points.length - 1].observedAt;
  const x = (value: number) => left + (last === first ? 0.5 : (value - first) / (last - first)) * xWidth;
  const y = (remaining: number) => top + (1 - remaining / 100) * (height - top - bottom);
  const selected = points[Math.min(active ?? points.length - 1, points.length - 1)];
  const series = useMemo(() => SERIES.map((line) => ({
    ...line, ...quotaHistorySeries(points, line.key, x, y, interval * MINUTE),
  })), [points, interval, width, height, first, last]);
  const percent = new Intl.NumberFormat(lang, { style: "percent" });
  const detailTime = new Intl.DateTimeFormat(lang, { dateStyle: "medium", timeStyle: "short" });
  const ticks = quotaHistoryTicks(first, last, period, width < 440 ? 2 : 4, lang);
  const track = (event: PointerEvent<SVGSVGElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    const position = (event.clientX - bounds.left) / bounds.width * width;
    setActive(nearestQuotaHistoryPoint(points, first + (position - left) / xWidth * (last - first)));
  };
  return (
    <figure className="codex-quota-history__figure">
      <figcaption className="codex-quota-history__details">
        <div className="codex-quota-history__metrics">
          {SERIES.map((line) => {
            const remaining = selected[line.key];
            return <div key={line.key} className={`codex-quota-history__metric codex-quota-history__${line.css}`}>
              <span><i aria-hidden="true" />{t(line.label)}</span>
              <strong data-missing={remaining === null}>
                {remaining === null ? t("common.unknown") : percent.format(remaining / 100)}
              </strong>
            </div>;
          })}
        </div>
        <time dateTime={new Date(selected.observedAt).toISOString()}>{detailTime.format(selected.observedAt)}</time>
      </figcaption>
      <svg ref={chartRef} viewBox={`0 0 ${width} ${height}`} style={{ height }}
        aria-label={t("codex.quotaHistory.title")} role="img" tabIndex={0}
        onPointerMove={track} onPointerLeave={() => setActive(null)}
        onKeyDown={(event) => {
          if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
          event.preventDefault();
          const change = event.key === "ArrowLeft" ? -1 : 1;
          setActive((index) => Math.min(points.length - 1, Math.max(0, (index ?? points.length - 1) + change)));
        }}>
        {[0, 25, 50, 75, 100].map((value) => <g key={value}>
          <line x1={left} x2={width - right} y1={y(value)} y2={y(value)}
            className={`codex-quota-history__grid${value === 0 ? " codex-quota-history__baseline" : ""}`} />
          <text x={left - 7} y={y(value) + 4} textAnchor="end">{percent.format(value / 100)}</text>
        </g>)}
        {ticks.map(({ value, label }) => <text key={value} x={x(value)} y={height - bottom + 25} textAnchor="middle">{label}</text>)}
        {active !== null && <line x1={x(selected.observedAt)} x2={x(selected.observedAt)} y1={top} y2={y(0)}
          className="codex-quota-history__cursor" />}
        {series.map((line) => {
          const remaining = selected[line.key];
          return <g key={line.key} className={`codex-quota-history__${line.css}`}>
            <path d={line.path} />
            {line.singles.map((point, index) => <circle key={index} cx={point.x} cy={point.y} r={2.5} />)}
            {remaining !== null && <circle cx={x(selected.observedAt)} cy={y(remaining)} r={4.5}
              className="codex-quota-history__point" />}
          </g>;
        })}
      </svg>
    </figure>
  );
}
