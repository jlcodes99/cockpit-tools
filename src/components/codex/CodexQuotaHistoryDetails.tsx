import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { PaginationControls } from "../PaginationControls";
import type { QuotaHistoryPoint } from "./codexQuotaHistoryChart";

const PAGE_SIZES = [20, 50, 100] as const;

function difference(current: number | null, previous: number | null | undefined) {
  return current === null || previous == null ? null : current - previous;
}

export function CodexQuotaHistoryDetails({ points, onlyChanges, onOnlyChangesChange }: {
  points: QuotaHistoryPoint[];
  onlyChanges: boolean;
  onOnlyChangesChange: (value: boolean) => void;
}) {
  const { t, i18n } = useTranslation();
  const lang = i18n.resolvedLanguage || i18n.language;
  const [pageSize, setPageSize] = useState<number>(PAGE_SIZES[0]);
  const [page, setPage] = useState(1);
  const rows = useMemo(() => points.map((point, index) => {
    const previous = points[index - 1];
    return {
      point,
      shortChange: difference(point.shortRemaining, previous?.shortRemaining),
      weeklyChange: difference(point.weeklyRemaining, previous?.weeklyRemaining),
    };
  }).reverse(), [points]);
  const visibleRows = useMemo(() => onlyChanges
    ? rows.filter(({ shortChange, weeklyChange }) => Boolean(shortChange || weeklyChange))
    : rows, [rows, onlyChanges]);
  const totalPages = Math.max(1, Math.ceil(visibleRows.length / pageSize));
  const currentPage = Math.min(page, totalPages);
  const start = (currentPage - 1) * pageSize;
  const end = Math.min(start + pageSize, visibleRows.length);
  const date = new Intl.DateTimeFormat(lang, { dateStyle: "medium", timeStyle: "short" });
  const percent = new Intl.NumberFormat(lang, { style: "percent" });
  const signed = new Intl.NumberFormat(lang, { signDisplay: "exceptZero" });
  const quotaCell = (remaining: number | null, change: number | null) => (
    <td>
      <div className="codex-quota-history__detail-value">
        <strong data-missing={remaining === null}>
          {remaining === null ? t("common.unknown") : percent.format(remaining / 100)}
        </strong>
        <span className="codex-quota-history__delta"
          data-direction={change === null || change === 0 ? undefined : change > 0 ? "up" : "down"}
          title={t("codex.quotaHistory.changeFromPrevious")}>
          {change === null ? "—" : t("codex.quotaHistory.points", { value: signed.format(change) })}
        </span>
      </div>
    </td>
  );

  return (
    <div className="codex-quota-history__table-panel">
      <div className="codex-quota-history__table-toolbar">
        <button type="button" role="switch" aria-checked={onlyChanges}
          className="codex-quota-history__control codex-quota-history__change-toggle"
          onClick={() => { setPage(1); onOnlyChangesChange(!onlyChanges); }}>
          <span className="codex-quota-history__switch-track" aria-hidden="true" />
          {t("codex.quotaHistory.onlyChanges")}
        </button>
      </div>
      {visibleRows.length === 0 ? <p className="codex-quota-history__table-empty" role="status">
        {t(points.length === 0 ? "codex.quotaHistory.empty" : "codex.quotaHistory.noChanges")}
      </p> : <>
        <table aria-label={t("codex.quotaHistory.details")} className="codex-quota-history__table">
          <thead>
            <tr>
              <th scope="col" aria-sort="descending">{t("codex.quotaHistory.sampledAt")}</th>
              <th scope="col">{t("codex.thresholds.shortCycle")}</th>
              <th scope="col">{t("codex.thresholds.weekly")}</th>
            </tr>
          </thead>
          <tbody>
            {visibleRows.slice(start, end).map(({ point, shortChange, weeklyChange }) => (
              <tr key={point.bucketStart}>
                <td><time dateTime={new Date(point.observedAt).toISOString()}>{date.format(point.observedAt)}</time></td>
                {quotaCell(point.shortRemaining, shortChange)}
                {quotaCell(point.weeklyRemaining, weeklyChange)}
              </tr>
            ))}
          </tbody>
        </table>
        <PaginationControls totalItems={visibleRows.length} currentPage={currentPage} totalPages={totalPages}
          pageSize={pageSize} pageSizeOptions={PAGE_SIZES} rangeStart={start + 1} rangeEnd={end}
          canGoPrevious={currentPage > 1} canGoNext={currentPage < totalPages}
          onPageSizeChange={(size) => { setPageSize(size); setPage(1); }}
          onPreviousPage={() => setPage(currentPage - 1)} onNextPage={() => setPage(currentPage + 1)} />
      </>}
    </div>
  );
}
