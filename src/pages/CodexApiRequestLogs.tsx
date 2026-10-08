import { useState } from "react";
import { CircleAlert } from "lucide-react";
import { SingleSelectDropdown } from "../components/SingleSelectDropdown";
import { PaginationControls } from "../components/PaginationControls";
import { CodexRequestProxyLabel } from "../components/codex/CodexRequestProxyLabel";
import { CodexRequestDetailModal } from "../components/codex/CodexRequestDetailModal";
import { hasRequestFirstResponse } from "../utils/codexRequestDiagnostics";
import { resolveCodexApiServiceLogModelPair } from "../utils/codexApiServiceLogModel";
import type { CodexLocalAccessUsageEvent } from "../types/codexLocalAccess";
import type { RequestLogKindFilter, RequestLogStatusFilter } from "./CodexApiServicePage";
import type { CodexApiServiceViewProps } from "./CodexApiServiceView";

const INTERNAL_API_KEY_ID = "__cockpit_internal__";

type Props = Pick<CodexApiServiceViewProps,
  "accountDisplayNames" |
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
                <div className="codex-api-service-log-list">
                  {requestLogError && (
                    <div className="codex-api-service-message error">
                      <CircleAlert size={15} />
                      <span>{requestLogError}</span>
                    </div>
                  )}
                  {requestLogLoading && requestLogEvents.length === 0 && (
                    <div className="codex-api-service-empty">
                      {t("codex.apiService.logs.loading", "正在加载请求日志")}
                    </div>
                  )}
                  {requestLogEvents.map((event, index) => {
                    const fullErrorDetail = cleanRequestLogErrorDetail(
                      event.errorMessage,
                    );
                    const errorDetail =
                      truncateRequestLogErrorDetail(fullErrorDetail);
                    const accountDisplayName =
                      accountDisplayNames.get((event.accountId || "").trim()) ||
                      accountDisplayNames.get((event.email || "").trim()) ||
                      event.email ||
                      event.accountId ||
                      "-";
                    const serviceTier = (event.serviceTier || "")
                      .trim()
                      .toLowerCase();
                    const serviceTierIsFast = serviceTier === "priority";
                    const serviceTierLabel =
                      serviceTier === "priority"
                        ? t("codex.speed.fast", "快速")
                        : serviceTier === "standard"
                          ? t("codex.speed.standard", "标准")
                          : event.serviceTier
                            ? t("codex.apiService.logs.serviceTierValue", {
                                tier: event.serviceTier,
                                defaultValue: "Tier {{tier}}",
                              })
                            : "";
                    // 始终标出实际上游模型（与请求模型一致时也展示）；仅当日志未记录上游模型时回退为单行。
                    const { requestedModel, upstreamModel } =
                      resolveCodexApiServiceLogModelPair(event);
                    return (
                      <div
                        key={`${event.timestamp}-${event.requestId || event.apiKeyId}-${index}`}
                        className="codex-api-service-log-row"
                      >
                        <div>
                          <div className="codex-api-service-log-model">
                            <strong title={requestedModel}>
                              {requestedModel}
                            </strong>
                            {upstreamModel ? (
                              <span
                                className="codex-api-service-log-model-upstream"
                                title={t(
                                  "codex.apiService.logs.upstreamModel",
                                  "实际模型",
                                )}
                              >
                                ↳ {upstreamModel}
                              </span>
                            ) : null}
                          </div>
                          <span
                            className={`codex-api-service-pill ${event.success ? "success" : "error"}`}
                          >
                            {event.success
                              ? t("codex.localAccess.requestLogSuccess", "成功")
                              : t("codex.localAccess.requestLogFailed", "失败")}
                          </span>
                          {event.reasoningEffort ? (
                            <span
                              className="codex-api-service-pill muted"
                              title={t(
                                "codex.apiService.logs.reasoningEffort",
                                "思考强度",
                              )}
                            >
                              {t("codex.apiService.logs.reasoningEffortValue", {
                                effort: event.reasoningEffort,
                                defaultValue: "思考 {{effort}}",
                              })}
                            </span>
                          ) : null}
                          {serviceTierLabel ? (
                            <span
                              className={`codex-api-service-pill ${serviceTierIsFast ? "fast" : "muted"}`}
                              title={t(
                                "codex.apiService.logs.serviceTier",
                                "服务等级",
                              )}
                            >
                              {serviceTierLabel}
                            </span>
                          ) : null}
                        </div>
                        <div>
                          <span>{formatDateTime(event.timestamp)}</span>
                          <span>{requestKindLabel(event.requestKind, t)}</span>
                          <span>
                            {event.apiKeyId === INTERNAL_API_KEY_ID
                              ? t(
                                  "codex.localAccess.internalSchedulerLabel",
                                  "Internal scheduler",
                                )
                              : event.apiKeyLabel || event.apiKeyId || "-"}
                          </span>
                          <span
                            title={
                              event.clientInstanceId
                                ? `${resolveClientInstanceLabel(
                                    event.clientInstanceId,
                                    codexInstances,
                                    t,
                                  )} (${event.clientInstanceId})`
                                : undefined
                            }
                          >
                            {resolveClientInstanceLabel(
                              event.clientInstanceId,
                              codexInstances,
                              t,
                            )}
                          </span>
                          <span>
                            {maskAccountText(accountDisplayName)}
                          </span>
                          <CodexRequestProxyLabel route={event.proxyRoute} t={t} />
                          <span title={t("codex.requestDiagnostics.firstResponseHint")}>
                            {t("codex.requestDiagnostics.latencyPair", {
                              total: formatLatencyMs(event.latencyMs),
                              first: hasRequestFirstResponse(event.firstResponseMs) ? formatLatencyMs(event.firstResponseMs) : '—',
                            })}
                          </span>
                          <span>
                            {formatCompactNumber(event.totalTokens)} Tokens
                          </span>
                          <span>{formatUsdCost(event.estimatedCostUsd)}</span>
                          {event.requestId ? (
                            <span>
                              {t("codex.apiService.logs.requestIdShort", {
                                id: event.requestId,
                                defaultValue: "ID {{id}}",
                              })}
                            </span>
                          ) : null}
                          {event.httpStatus ? (
                            <span>
                              {t("codex.apiService.logs.httpStatus", {
                                status: event.httpStatus,
                                defaultValue: "HTTP {{status}}",
                              })}
                            </span>
                          ) : null}
                          {event.errorCategory ? (
                            <span>{event.errorCategory}</span>
                          ) : null}
                          {errorDetail ? (
                            <span
                              className="codex-api-service-log-error-detail"
                              title={fullErrorDetail}
                            >
                              {errorDetail}
                            </span>
                          ) : null}
                          {event.requestId && <button type="button" className="btn btn-secondary btn-sm" onClick={() => setSelectedRequest(event)}>
                            {t("codex.requestDiagnostics.details")}
                          </button>}
                        </div>
                      </div>
                    );
                  })}
                  {!requestLogLoading &&
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
                {selectedRequest && <CodexRequestDetailModal event={selectedRequest} maskAccountText={maskAccountText} onClose={() => setSelectedRequest(null)} />}
              </>
  );
}
