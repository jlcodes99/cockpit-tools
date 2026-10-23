import { resolveCodexAccountReferenceDisplayName } from "../utils/codexAccountDisplayName";
import { useState } from "react";
import { CodexOutputSpeed } from "../components/codex/CodexOutputSpeed";
import { aggregateOutputSpeed, formatOutputSpeed } from "../utils/codexOutputSpeed";
import { Check, CircleAlert, Columns3, Route, Zap } from "lucide-react";
import { SingleSelectDropdown } from "../components/SingleSelectDropdown";
import { PaginationControls } from "../components/PaginationControls";
import { CodexRequestProxyLabel } from "../components/codex/CodexRequestProxyLabel";
import { CodexRequestDetailModal } from "../components/codex/CodexRequestDetailModal";
import { authorizationFallbackReasonKey, authorizationFallbackSummaryKey, hasRequestFirstResponse, recordedServiceTierAdaptation, requestAuthorizationRouteKey, requestServiceTierPresentation, requestedServiceTierLabelKey } from "../utils/codexRequestDiagnostics";
import { resolveCodexApiServiceLogModelPair } from "../utils/codexApiServiceLogModel";
import type { CodexLocalAccessUsageEvent } from "../types/codexLocalAccess";
import type { RequestLogKindFilter, RequestLogStatusFilter } from "./CodexApiServicePage";
import type { CodexApiServiceViewProps } from "./CodexApiServiceView";
import "./CodexApiRequestLogs.css";
import "../components/codex/CodexAuthorizationFallback.css";

const INTERNAL_API_KEY_ID = "__cockpit_internal__";

const LOG_COLUMNS_STORAGE_KEY = "codex.apiService.logColumns.v1";
const LOG_COLUMNS = [
  { id: "time", width: 140 },
  { id: "model", width: 220 },
  { id: "result", width: 130 },
  { id: "source", width: 185 },
  { id: "account", width: 190 },
  { id: "tokens", width: 170 },
  { id: "cost", width: 115 },
  { id: "performance", width: 165 },
  { id: "proxy", width: 190 },
  { id: "requestId", width: 155 },
] as const;
type LogColumn = typeof LOG_COLUMNS[number]["id"];
const DEFAULT_LOG_COLUMNS: LogColumn[] = LOG_COLUMNS
  .filter(({ id }) => id !== "proxy" && id !== "requestId")
  .map(({ id }) => id);

function readLogColumns(): LogColumn[] {
  try {
    const stored: unknown = JSON.parse(localStorage.getItem(LOG_COLUMNS_STORAGE_KEY) || "null");
    if (Array.isArray(stored)) {
      const selected = LOG_COLUMNS.filter(({ id }) => stored.includes(id)).map(({ id }) => id);
      return selected.some((id) => id === "model" || id === "result") ? selected : ["model", ...selected];
    }
  } catch {
    // 偏好损坏或存储不可用时，列表仍按默认列展示。
  }
  return DEFAULT_LOG_COLUMNS;
}

type Props = Pick<CodexApiServiceViewProps,
  "accountDisplayNames" |
  "openTimeoutSettings" |
  "cleanRequestLogErrorDetail" |
  "clearRequestLogFilters" |
  "codexInstances" |
  "formatCompactNumber" |
  "formatDateTime" |
  "formatLatencyMs" |
  "formatUsdCost" |
  "hasRequestLogFilters" |
  "maskAccountText" |
  "normalizeRequestLogPageSize" |
  "REQUEST_LOG_PAGE_SIZE_OPTIONS" |
  "requestKindLabel" |
  "requestLogAccountQuery" |
  "requestLogApiKeyQuery" |
  "requestLogCurrentPage" |
  "requestLogError" |
  "requestLogErrorQuery" |
  "requestLogEvents" |
  "requestLogInstanceOptions" |
  "requestLogInstanceQuery" |
  "requestLogKindFilter" |
  "requestLogKindOptions" |
  "requestLogLoaded" |
  "requestLogLoading" |
  "requestLogModelQuery" |
  "requestLogPageSize" |
  "requestLogRangeEnd" |
  "requestLogRangeStart" |
  "requestLogStatusFilter" |
  "requestLogStatusOptions" |
  "requestLogTotal" |
  "requestLogTotalPages" |
  "resolveClientInstanceLabel" |
  "setRequestLogAccountQuery" |
  "setRequestLogApiKeyQuery" |
  "setRequestLogErrorQuery" |
  "setRequestLogInstanceQuery" |
  "setRequestLogKindFilter" |
  "setRequestLogModelQuery" |
  "setRequestLogPage" |
  "setRequestLogPageSize" |
  "setRequestLogStatusFilter" |
  "t" |
  "truncateRequestLogErrorDetail"
>;

export function CodexApiRequestLogs(props: Props) {
  const {
    accountDisplayNames,
    cleanRequestLogErrorDetail,
    clearRequestLogFilters,
    codexInstances,
    formatCompactNumber,
    formatDateTime,
    formatLatencyMs,
    formatUsdCost,
    hasRequestLogFilters,
    maskAccountText,
    normalizeRequestLogPageSize,
    REQUEST_LOG_PAGE_SIZE_OPTIONS,
    requestKindLabel,
    requestLogAccountQuery,
    requestLogApiKeyQuery,
    requestLogCurrentPage,
    requestLogError,
    requestLogErrorQuery,
    requestLogEvents,
    requestLogInstanceOptions,
    requestLogInstanceQuery,
    requestLogKindFilter,
    requestLogKindOptions,
    requestLogLoaded,
    requestLogLoading,
    requestLogModelQuery,
    requestLogPageSize,
    requestLogRangeEnd,
    requestLogRangeStart,
    requestLogStatusFilter,
    requestLogStatusOptions,
    requestLogTotal,
    requestLogTotalPages,
    resolveClientInstanceLabel,
    setRequestLogAccountQuery,
    setRequestLogApiKeyQuery,
    setRequestLogErrorQuery,
    setRequestLogInstanceQuery,
    setRequestLogKindFilter,
    setRequestLogModelQuery,
    setRequestLogPage,
    setRequestLogPageSize,
    setRequestLogStatusFilter,
    t,
    truncateRequestLogErrorDetail
  } = props;
  const [selectedRequest, setSelectedRequest] = useState<CodexLocalAccessUsageEvent | null>(null);
  const [visibleColumns, setVisibleColumns] = useState<LogColumn[]>(readLogColumns);
  const [columnSettingsOpen, setColumnSettingsOpen] = useState(false);
  const columnVisible = (column: LogColumn) => visibleColumns.includes(column);
  const columnRequired = (column: LogColumn) => (column === "model" || column === "result") &&
    columnVisible(column) && !columnVisible(column === "model" ? "result" : "model");
  const activeColumns = LOG_COLUMNS.filter(({ id }) => columnVisible(id));
  const toggleColumn = (column: LogColumn) => {
    setVisibleColumns((current) => {
      if ((column === "model" || column === "result") && current.includes(column) &&
        !current.includes(column === "model" ? "result" : "model")) return current;
      const next = LOG_COLUMNS.filter(({ id }) => id === column ? !current.includes(id) : current.includes(id)).map(({ id }) => id);
      try {
        localStorage.setItem(LOG_COLUMNS_STORAGE_KEY, JSON.stringify(next));
      } catch {
        // 存储不可用不应阻止本次列切换。
      }
      return next;
    });
  };
  const pageSpeed = aggregateOutputSpeed(requestLogEvents);
  const pageSpeedValue = formatOutputSpeed(pageSpeed.tokensPerSecond);
  const initialLoading = requestLogLoading && !requestLogLoaded;
  return (
              <>
                <div className="codex-api-service-log-filters">
                  <label>
                    <span>
                      {t("codex.apiService.logs.modelFilter", "模型")}
                    </span>
                    <input
                      value={requestLogModelQuery}
                      onChange={(event) =>
                        setRequestLogModelQuery(event.target.value)
                      }
                      placeholder={t(
                        "codex.apiService.logs.modelPlaceholder",
                        "Model ID",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.accountFilter", "Account")}
                    </span>
                    <input
                      value={requestLogAccountQuery}
                      onChange={(event) =>
                        setRequestLogAccountQuery(event.target.value)
                      }
                      placeholder={t(
                        "codex.apiService.logs.accountPlaceholder",
                        "Email or account ID",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.apiKeyFilter", "API Key")}
                    </span>
                    <input
                      value={requestLogApiKeyQuery}
                      onChange={(event) =>
                        setRequestLogApiKeyQuery(event.target.value)
                      }
                      placeholder={t(
                        "codex.apiService.logs.apiKeyPlaceholder",
                        "Name or ID",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.instanceFilter", "Instance")}
                    </span>
                    <SingleSelectDropdown
                      value={requestLogInstanceQuery}
                      options={requestLogInstanceOptions}
                      onChange={setRequestLogInstanceQuery}
                      ariaLabel={t(
                        "codex.apiService.logs.instanceFilter",
                        "Instance",
                      )}
                      placeholder={t(
                        "codex.apiService.logs.allInstances",
                        "All Instances",
                      )}
                    />
                  </label>
                  <label>
                    <span>{t("codex.apiService.logs.kindFilter", "Type")}</span>
                    <SingleSelectDropdown
                      value={requestLogKindFilter}
                      options={requestLogKindOptions}
                      onChange={(value) =>
                        setRequestLogKindFilter(value as RequestLogKindFilter)
                      }
                      ariaLabel={t("codex.apiService.logs.kindFilter", "Type")}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.statusFilter", "Status")}
                    </span>
                    <SingleSelectDropdown
                      value={requestLogStatusFilter}
                      options={requestLogStatusOptions}
                      onChange={(value) =>
                        setRequestLogStatusFilter(
                          value as RequestLogStatusFilter,
                        )
                      }
                      ariaLabel={t(
                        "codex.apiService.logs.statusFilter",
                        "Status",
                      )}
                    />
                  </label>
                  <label>
                    <span>
                      {t("codex.apiService.logs.errorFilter", "Error")}
                    </span>
                    <input
                      value={requestLogErrorQuery}
                      onChange={(event) =>
                        setRequestLogErrorQuery(event.target.value)
                      }
                      placeholder={t(
                        "codex.apiService.logs.errorPlaceholder",
                        "Error category",
                      )}
                    />
                  </label>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={clearRequestLogFilters}
                    disabled={!hasRequestLogFilters}
                  >
                    {t("codex.apiService.logs.clearFilters", "Clear Filters")}
                  </button>
                </div>
                <div className="codex-request-log-toolbar">
                  {requestLogEvents.length > 0 && <div className="codex-output-speed-summary" title={t("common.outputSpeed.pageHint")}>
                    <span>{t("common.outputSpeed.pageAverage")} {pageSpeedValue == null ? "—" : t("common.outputSpeed.value", { value: pageSpeedValue })}</span>
                    <span>{t("common.outputSpeed.pageSamples", { count: pageSpeed.samples })}</span>
                  </div>}
                  <div
                    className="codex-request-log-column-picker"
                    onBlur={(event) => {
                      if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setColumnSettingsOpen(false);
                    }}
                    onKeyDown={(event) => {
                      if (event.key === "Escape") {
                        setColumnSettingsOpen(false);
                        event.currentTarget.querySelector<HTMLButtonElement>("button")?.focus();
                      }
                    }}
                  >
                    <button type="button" className="btn btn-secondary btn-sm" aria-expanded={columnSettingsOpen} aria-haspopup="true" onClick={() => setColumnSettingsOpen((open) => !open)}>
                      <Columns3 size={14} />{t("codex.apiService.logs.columnSettings")}
                    </button>
                    {columnSettingsOpen && <div className="codex-request-log-column-options" role="group" aria-label={t("codex.apiService.logs.columnSettings")}>
                      {LOG_COLUMNS.map(({ id }) => <button
                        type="button"
                        key={id}
                        className="codex-request-log-column-option"
                        aria-pressed={columnVisible(id)}
                        disabled={columnRequired(id)}
                        onClick={() => toggleColumn(id)}
                      >
                        <span className={`codex-request-log-column-check ${columnVisible(id) ? "checked" : ""}`} aria-hidden="true">{columnVisible(id) && <Check size={12} />}</span>
                        {t(`codex.apiService.logs.columns.${id}`)}
                      </button>)}
                    </div>}
                  </div>
                </div>
                <div className="codex-api-service-log-list">
                  {requestLogError && (
                    <div className="codex-api-service-message error">
                      <CircleAlert size={15} />
                      <span>{requestLogError}</span>
                    </div>
                  )}
                  {initialLoading && requestLogEvents.length === 0 && (
                    <div className="codex-api-service-empty">
                      {t("codex.apiService.logs.loading", "正在加载请求日志")}
                    </div>
                  )}
                  {requestLogEvents.length > 0 && <div className="codex-request-log-scroll" tabIndex={0} role="region" aria-label={t("codex.localAccess.requestLogTitle")} aria-busy={requestLogLoading}>
                    <table className="codex-request-log-table" style={{ minWidth: activeColumns.reduce((width, column) => width + column.width, 120) }}>
                      <colgroup>
                        {activeColumns.map(({ id, width }) => <col key={id} style={{ width }} />)}
                        <col style={{ width: 120 }} />
                      </colgroup>
                      <thead><tr>
                        {activeColumns.map(({ id }) => <th key={id} scope="col" title={t(`codex.apiService.logs.columns.${id}`)} className={id === "cost" ? "numeric" : undefined}>{t(`codex.apiService.logs.columns.${id}`)}</th>)}
                        <th scope="col" className="codex-request-log-actions" title={t("codex.apiService.logs.columns.actions")}>{t("codex.apiService.logs.columns.actions")}</th>
                      </tr></thead>
                      <tbody>{requestLogEvents.map((event, index) => {
                        const fullErrorDetail = cleanRequestLogErrorDetail(event.errorMessage);
                        const fallbackSummaryKey = authorizationFallbackSummaryKey(event.authorizationFallback);
                        const fallbackAccount = event.authorizationFallback ? maskAccountText(resolveCodexAccountReferenceDisplayName(event.authorizationFallback.fromAccountId, null, accountDisplayNames, t("common.apiKeyAccount"))) : "";
                        const errorDetail = truncateRequestLogErrorDetail(fullErrorDetail);
                        const accountDisplayName = resolveCodexAccountReferenceDisplayName(event.accountId, event.email, accountDisplayNames, t("common.apiKeyAccount")) || "—";
                        const maskedAccount = maskAccountText(accountDisplayName);
                        const tiers = requestServiceTierPresentation(event);
                        const adaptation = recordedServiceTierAdaptation(event.serviceTierAdaptation);
                        const originalTier = adaptation?.requestedTier ?? tiers.requested;
                        const originalTierKey = requestedServiceTierLabelKey(originalTier);
                        const requestedTier = originalTier.toLowerCase();
                        const speed = ["priority", "fast"].includes(requestedTier) ? "fast"
                          : requestedTier === "ultrafast" ? "ultrafast"
                          : ["standard", "default"].includes(requestedTier) ? "standard" : null;
                        // 与请求模型相同时也保留实际上游模型；旧记录缺失时不推断。
                        const { requestedModel, upstreamModel } = resolveCodexApiServiceLogModelPair(event);
                        const apiKeyLabel = event.apiKeyId === INTERNAL_API_KEY_ID
                          ? t("codex.localAccess.internalSchedulerLabel", "Internal scheduler")
                          : event.apiKeyLabel || event.apiKeyId || "—";
                        const instanceLabel = resolveClientInstanceLabel(event.clientInstanceId, codexInstances, t);
                        return <tr key={`${event.timestamp}-${event.requestId || event.apiKeyId}-${index}`}>
                          {columnVisible("time") && <td><span className="codex-request-log-time">{formatDateTime(event.timestamp)}</span></td>}
                          {columnVisible("model") && <td>
                            <div className="codex-request-log-model">
                              <strong title={requestedModel}>{requestedModel}</strong>
                              {upstreamModel && <span className="codex-request-log-truncate codex-request-log-upstream" title={`${t("codex.apiService.logs.upstreamModel", "实际模型")}: ${upstreamModel}`}>↳ {upstreamModel}</span>}
                              <div className="codex-request-log-tags">
                                <span className="codex-api-service-pill muted">{requestKindLabel(event.requestKind, t)}</span>
                                {event.reasoningEffort && <span className="codex-api-service-pill muted" title={t("codex.apiService.logs.reasoningEffort", "思考强度")}>{event.reasoningEffort}</span>}
                                {adaptation ? <span className="codex-request-log-speed standard adjusted" title={t("codex.apiService.logs.officialDefaultSpeed")}>
                                  {t("codex.apiService.logs.speedAdjusted", { tier: originalTierKey ? t(originalTierKey) : originalTier })}
                                </span> : speed && <span className={`codex-request-log-speed ${speed}`} title={t("codex.apiService.logs.serviceTierHint")}>
                                  {speed !== "standard" && <Zap size={12} aria-hidden="true" />}
                                  {t(`codex.speed.${speed}`)}
                                </span>}
                              </div>
                            </div>
                          </td>}
                          {columnVisible("result") && <td>
                            <div className="codex-request-log-stack">
                              <span className={`codex-api-service-pill ${event.success ? "success" : "error"}`}>{event.success ? t("codex.localAccess.requestLogSuccess", "成功") : t("codex.localAccess.requestLogFailed", "失败")}</span>
                              {fallbackSummaryKey && <span className="codex-api-service-pill muted codex-request-log-fallback" title={`${fallbackAccount} · ${t(authorizationFallbackReasonKey(event.authorizationFallback!.reason))}`}><Route size={12} aria-hidden="true" />{t(fallbackSummaryKey)}</span>}
                              {event.httpStatus ? <span>{t("codex.apiService.logs.httpStatus", { status: event.httpStatus, defaultValue: "HTTP {{status}}" })}</span> : null}
                              {(event.errorCategory || errorDetail) && <span className="codex-request-log-truncate codex-request-log-error" title={fullErrorDetail || event.errorCategory}>{event.errorCategory || errorDetail}</span>}
                            </div>
                          </td>}
                          {columnVisible("source") && <td><div className="codex-request-log-stack">
                            <span className="codex-request-log-truncate" title={`${t("codex.apiService.logs.apiKeyFilter", "API Key")}: ${apiKeyLabel}`}><span className="codex-request-log-label">{t("codex.apiService.logs.apiKeyFilter", "API Key")}</span> {apiKeyLabel}</span>
                            <span className="codex-request-log-truncate" title={event.clientInstanceId ? `${instanceLabel} (${event.clientInstanceId})` : instanceLabel}><span className="codex-request-log-label">{t("codex.apiService.logs.instanceFilter", "Instance")}</span> {instanceLabel}</span>
                            <span className="codex-request-log-truncate" title={t(requestAuthorizationRouteKey(event.accountId, event.authorizationRoute))}><span className="codex-request-log-label">{t("codex.requestDiagnostics.authorizationRoute")}</span> {t(requestAuthorizationRouteKey(event.accountId, event.authorizationRoute))}</span>
                          </div></td>}
                          {columnVisible("account") && <td><span className="codex-request-log-truncate" title={maskedAccount}>{maskedAccount}</span></td>}
                          {columnVisible("tokens") && <td><div className="codex-request-log-token-grid">
                            {([["tokenInput", event.inputTokens], ["tokenOutput", event.outputTokens], ["tokenCache", event.cachedTokens], ["tokenTotal", event.totalTokens]] as const).map(([key, count]) => <span key={key} className={key === "tokenTotal" ? "codex-request-log-token-total" : undefined}>
                              <span className="codex-request-log-label" title={t(`codex.apiService.logs.${key}`)}>{t(`codex.apiService.logs.${key}`)}</span><span className="codex-request-log-token-value" title={String(count)}>{formatCompactNumber(count)}</span>
                            </span>)}
                          </div></td>}
                          {columnVisible("cost") && <td className="numeric">{formatUsdCost(event.estimatedCostUsd)}</td>}
                          {columnVisible("performance") && <td><div className="codex-request-log-stack codex-request-log-performance">
                            <span title={t("codex.requestDiagnostics.firstResponseHint")}><span className="codex-request-log-label" title={t("codex.requestDiagnostics.firstResponse")}>{t("codex.requestDiagnostics.firstResponse")}</span><span>{hasRequestFirstResponse(event.firstResponseMs) ? formatLatencyMs(event.firstResponseMs) : "—"}</span></span>
                            <span><span className="codex-request-log-label" title={t("codex.requestDiagnostics.totalLatency")}>{t("codex.requestDiagnostics.totalLatency")}</span><span>{formatLatencyMs(event.latencyMs)}</span></span>
                            <span><span className="codex-request-log-label" title={t("common.outputSpeed.label")}>{t("common.outputSpeed.label")}</span><CodexOutputSpeed event={event} t={t} hideLabel /></span>
                          </div></td>}
                          {columnVisible("proxy") && <td><CodexRequestProxyLabel route={event.proxyRoute} t={t} /></td>}
                          {columnVisible("requestId") && <td><span className="codex-request-log-truncate codex-request-log-id" title={event.requestId}>{event.requestId || "—"}</span></td>}
                          <td className="codex-request-log-actions">{event.requestId && <button type="button" className="btn btn-secondary btn-sm" title={t("codex.requestDiagnostics.details")} onClick={() => setSelectedRequest(event)}><span className="codex-request-log-truncate">{t("codex.requestDiagnostics.details")}</span></button>}</td>
                        </tr>;
                      })}</tbody>
                    </table>
                  </div>}
                  {!initialLoading &&
                    !requestLogError &&
                    requestLogEvents.length === 0 && (
                      <div className="codex-api-service-empty">
                        {t("codex.localAccess.requestLogEmpty", "暂无请求日志")}
                      </div>
                    )}
                </div>
                <PaginationControls
                  totalItems={requestLogTotal}
                  currentPage={requestLogCurrentPage}
                  totalPages={requestLogTotalPages}
                  pageSize={requestLogPageSize}
                  pageSizeOptions={REQUEST_LOG_PAGE_SIZE_OPTIONS}
                  rangeStart={requestLogRangeStart}
                  rangeEnd={requestLogRangeEnd}
                  canGoPrevious={requestLogCurrentPage > 1}
                  canGoNext={requestLogCurrentPage < requestLogTotalPages}
                  onPageSizeChange={(pageSize) => {
                    setRequestLogPageSize(
                      normalizeRequestLogPageSize(pageSize),
                    );
                    setRequestLogPage(1);
                  }}
                  onPreviousPage={() =>
                    setRequestLogPage((page) => Math.max(1, page - 1))
                  }
                  onNextPage={() =>
                    setRequestLogPage((page) =>
                      Math.min(requestLogTotalPages, page + 1),
                    )
                  }
                />
                {selectedRequest && <CodexRequestDetailModal onOpenTimeoutSettings={(field) => { setSelectedRequest(null); props.openTimeoutSettings(field); }} accountDisplayNames={accountDisplayNames} event={selectedRequest} maskAccountText={maskAccountText} onClose={() => setSelectedRequest(null)} />}
              </>
  );
}
