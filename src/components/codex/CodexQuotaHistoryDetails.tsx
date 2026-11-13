import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { PaginationControls } from "../PaginationControls";
import type { QuotaHistory } from "./codexQuotaHistoryChart";

const PAGE_SIZES = [20, 50, 100] as const;

function difference(current: number | null, previous: number | null | undefined) {
  if (current === null || previous == null) return null;
  const rounded = Math.round((current - previous) * 10) / 10;
  return rounded === 0 ? 0 : rounded;
}

export function CodexQuotaHistoryDetails({ history, onlyChanges, onOnlyChangesChange }: {
  history: QuotaHistory;
  onlyChanges: boolean;
  onOnlyChangesChange: (value: boolean) => void;
}) {
  const { t, i18n } = useTranslation();
  const { points, shortConsumed, weeklyConsumed } = history;
  const lang = i18n.resolvedLanguage || i18n.language;
  const [pageSize, setPageSize] = useState<number>(PAGE_SIZES[0]);
  const [page, setPage] = useState(1);
  const rows = useMemo(() => points.map((point, index) => {
    const previous = points[index - 1];
    return {
      point,
      isBaseline: index === 0,
      shortChange: difference(point.shortRemaining, previous?.shortRemaining),
      weeklyChange: difference(point.weeklyRemaining, previous?.weeklyRemaining),
    };
  }).reverse(), [points]);
  const visibleRows = useMemo(() => onlyChanges
    ? rows.filter(({ isBaseline, shortChange, weeklyChange }) => isBaseline || Boolean(shortChange || weeklyChange))
    : rows, [rows, onlyChanges]);
  const totalPages = Math.max(1, Math.ceil(visibleRows.length / pageSize));
  const currentPage = Math.min(page, totalPages);
  const start = (currentPage - 1) * pageSize;
  const end = Math.min(start + pageSize, visibleRows.length);
  const date = new Intl.DateTimeFormat(lang, { dateStyle: "medium", timeStyle: "short" });
  const percent = new Intl.NumberFormat(lang, { style: "percent", maximumFractionDigits: 1 });
  const deltaPercent = new Intl.NumberFormat(lang, { style: "percent", signDisplay: "exceptZero", maximumFractionDigits: 1 });
  const remainingCell = (remaining: number | null) => (
    <td>
      <strong className="codex-quota-history__remaining" data-missing={remaining === null}>
        {remaining === null ? t("codex.quotaHistory.noValue") : percent.format(remaining / 100)}
      </strong>
    </td>
  );
  const deltaCell = (change: number | null) => {
    const hasChange = change !== null && change !== 0;
    return (
      <td>
        <span className="codex-quota-history__delta"
          data-direction={!hasChange ? undefined : change > 0 ? "up" : "down"}
          title={t("codex.quotaHistory.changeFromPrevious")}>
          {hasChange ? deltaPercent.format(change / 100) : "--"}
        </span>
      </td>
    );
  };

  return (
    <div className="codex-quota-history__table-panel">
      <div className="codex-quota-history__table-toolbar">
        <dl className="codex-quota-history__consumption">
          {([
            ["shortConsumed", shortConsumed],
            ["weeklyConsumed", weeklyConsumed],
          ] as const).map(([label, consumed]) => (
            <div key={label}>
              <dt>{t(`codex.quotaHistory.${label}`)}</dt>
              <dd>{consumed === null ? t("codex.quotaHistory.noValue") : deltaPercent.format(-consumed / 100)}</dd>
            </div>
          ))}
        </dl>
        <button type="button" role="switch" aria-checked={onlyChanges}
          className="codex-quota-history__control codex-quota-history__change-toggle"
          onClick={() => { setPage(1); onOnlyChangesChange(!onlyChanges); }}>
          <span className="codex-quota-history__switch-track" aria-hidden="true" />
          {t("codex.quotaHistory.onlyChanges")}
        </button>
      </div>
      {visibleRows.length === 0 ? <p className="codex-quota-history__table-empty" role="status">
        {t("codex.quotaHistory.empty")}
      </p> : <>
        <div className="codex-quota-history__table-scroll">
          <table aria-label={t("codex.quotaHistory.details")} className="codex-quota-history__table">
            <thead>
              <tr>
                <th scope="col" aria-sort="descending">{t("codex.quotaHistory.recordedAt")}</th>
                <th scope="col">{t("codex.quotaHistory.shortRemaining")}</th>
                <th scope="col">{t("codex.quotaHistory.shortDelta")}</th>
                <th scope="col">{t("codex.quotaHistory.weeklyRemaining")}</th>
                <th scope="col">{t("codex.quotaHistory.weeklyDelta")}</th>
              </tr>
            </thead>
            <tbody>
              {visibleRows.slice(start, end).map(({ point, shortChange, weeklyChange }) => (
                <tr key={point.bucketStart}>
                  <td><time dateTime={new Date(point.observedAt).toISOString()}>{date.format(point.observedAt)}</time></td>
                  {remainingCell(point.shortRemaining)}
                  {deltaCell(shortChange)}
                  {remainingCell(point.weeklyRemaining)}
                  {deltaCell(weeklyChange)}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <PaginationControls totalItems={visibleRows.length} currentPage={currentPage} totalPages={totalPages}
          pageSize={pageSize} pageSizeOptions={PAGE_SIZES} rangeStart={start + 1} rangeEnd={end}
          canGoPrevious={currentPage > 1} canGoNext={currentPage < totalPages}
          onPageSizeChange={(size) => { setPageSize(size); setPage(1); }}
          onPreviousPage={() => setPage(currentPage - 1)} onNextPage={() => setPage(currentPage + 1)} />
      </>}
    </div>
  );
}
