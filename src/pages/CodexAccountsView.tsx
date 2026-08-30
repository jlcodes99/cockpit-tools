import { createPortal } from "react-dom";
import { RefreshCw, Download, X, Database, Copy, Check, CircleAlert, Minimize2 } from "lucide-react";
import { buildCodexAccountPresentation } from "../presentation/platformAccountPresentation";
import { CodexOverviewTabsHeader } from "../components/CodexOverviewTabsHeader";
import { CodexInstancesContent } from "./CodexInstancesPage";
import { CodexLaunchPreviewModal } from "../components/codex/CodexLaunchPreviewModal";
import { CodexSessionManager } from "../components/codex/CodexSessionManager";
import { CodexCliLaunchDialog } from "../components/codex/CodexCliLaunchDialog";
import { CodexWakeupContent } from "../components/codex/CodexWakeupContent";
import { CodexModelProviderManager } from "../components/codex/CodexModelProviderManager";
import type { useCodexAccountsPageController } from "./CodexAccountsPage";
import { CodexAccountsOverviewPanel } from "./CodexAccountsOverviewPanel";


export type CodexAccountsViewProps = ReturnType<typeof useCodexAccountsPageController>;

/** 渲染 CodexAccountsPage 的界面；业务状态与动作统一由 Controller 提供。 */
export function CodexAccountsView(props: CodexAccountsViewProps) {
  const {
    accounts,
    activeBatchImportCheckQuota,
    activeLaunchPreviewAccount,
    activeTab,
    batchImportBusy,
    batchImportCheckQuota,
    batchImportCounts,
    batchImportError,
    batchImportOpen,
    batchImportPreview,
    batchImportProgress,
    batchImportProgressCurrent,
    batchImportProgressPercent,
    batchImportProgressTotal,
    batchImportResult,
    batchImportSelectableIds,
    batchImportSelectableIdSet,
    batchImportSelectedCountLabel,
    batchImportSelectedIds,
    batchImportSelectedSelectableCount,
    batchImportTagsInput,
    batchImportVisibleItems,
    buildAccountLaunchPreviewActions,
    buildAccountLaunchPreviewSummary,
    buildLocalAccessLaunchPreviewActions,
    buildLocalAccessLaunchPreviewSummary,
    clearBatchImportSelection,
    cliLaunchModal,
    closeCliLaunchModal,
    closeExternalImportProgressModal,
    deepSeekStart,
    externalImportPercent,
    externalImportProgress,
    externalImportRunning,
    externalImportStepIndex,
    externalImportSteps,
    externalImportSyncError,
    fetchAccounts,
    fetchCurrentAccount,
    fullQuotaWakeupOpenRequest,
    handleBatchImportCheckQuotaChange,
    handleCancelBatchImport,
    handleChooseCodexCliWorkingDir,
    handleCloseBatchImport,
    handleConfirmBatchImport,
    handleCopyCodexCliCommand,
    handleCopyExternalImportErrors,
    handleExecuteCodexCli,
    handleExecuteLaunchPreview,
    handleExecuteLocalAccessLaunchPreview,
    handleResumeBatchImport,
    handleSyncImportedToApiServiceChange,
    handleViewExternalImportAccounts,
    launchPreviewInstanceId,
    launchPreviewInstanceLabel,
    launchPreviewInstanceOptions,
    localAccessCollection,
    localAccessLaunchPreviewOpen,
    localAccessSaving,
    maskAccountText,
    overviewLayoutMode,
    prepareCodexCliLaunch,
    renderApiKeyUsageDetailModal,
    renderCockpitApiServicePanel,
    renderQuotaErrorDetailModal,
    selectAllBatchImportAccounts,
    selectedTerminal,
    selectReadyBatchImportAccounts,
    setActiveTab,
    setBatchImportOpen,
    setBatchImportTagsInput,
    setLaunchPreviewAccount,
    setLaunchPreviewInstanceId,
    setLocalAccessLaunchPreviewOpen,
    setManagedProviders,
    setSelectedTerminal,
    setWakeupPresetManagerSignal,
    sortedAccountsForInstances,
    syncImportedToApiService,
    t,
    terminalOptions,
    toggleBatchImportItem,
    updateCodexCliWorkingDir,
    wakeupPresetManagerSignal,
  } = props;
  return (
    <div
      className={`codex-accounts-page codex-accounts-page--${overviewLayoutMode}`}
    >
      <CodexOverviewTabsHeader
        active={activeTab}
        onTabChange={setActiveTab}
        tabs={["overview", "providers", "wakeup", "instances", "sessions"]}
      />

      {batchImportOpen &&
        createPortal(
          <div className="modal-overlay codex-batch-import-overlay">
            <div
              className="modal-content codex-batch-import-modal"
              onClick={(event) => event.stopPropagation()}
            >
              <div className="modal-header">
                <div>
                  <h2>{t("codex.batchImport.title", "Codex 批量导入")}</h2>
                  <p className="codex-batch-import-subtitle">
                    {batchImportResult
                      ? t("codex.batchImport.resultSubtitle", "导入结果")
                      : batchImportProgress?.phase === "importing" ||
                          batchImportProgress?.phase === "finalizing"
                        ? t(
                            "codex.batchImport.importSubtitle",
                            "正在写入选中的账号",
                          )
                        : batchImportBusy
                          ? activeBatchImportCheckQuota
                            ? t(
                                "codex.batchImport.scanSubtitle",
                                "正在逐条解析并检查账号",
                              )
                            : t(
                                "codex.batchImport.parseSubtitle",
                                "正在解析账号文件",
                              )
                          : batchImportPreview
                            ? t(
                                "codex.batchImport.previewSubtitle",
                                "选择要写入的账号",
                              )
                            : activeBatchImportCheckQuota
                              ? t(
                                  "codex.batchImport.scanSubtitle",
                                  "正在逐条解析并检查账号",
                                )
                              : t(
                                  "codex.batchImport.parseSubtitle",
                                  "正在解析账号文件",
                                )}
                  </p>
                </div>
                <div className="codex-batch-import-header-actions">
                  {batchImportBusy && (
                    <button
                      type="button"
                      className="btn btn-secondary compact"
                      onClick={() => setBatchImportOpen(false)}
                    >
                      <Minimize2 size={14} />
                      {t("codex.batchImport.runInBackground", "后台执行")}
                    </button>
                  )}
                  <button
                    className="modal-close"
                    onClick={() => void handleCloseBatchImport()}
                  >
                    <X size={18} />
                  </button>
                </div>
              </div>

              <div className="codex-batch-import-body">
                {batchImportError && (
                  <div className="codex-batch-import-error">
                    <CircleAlert size={16} />
                    <span>{batchImportError}</span>
                  </div>
                )}

                {!batchImportResult && (
                  <div className="codex-batch-import-progress-panel">
                    <div className="codex-batch-import-progress-head">
                      <span>
                        {batchImportProgress?.phase === "cancelling"
                          ? t("codex.batchImport.cancelling", "正在取消...")
                          : batchImportBusy
                            ? batchImportProgress?.phase === "importing" ||
                              batchImportProgress?.phase === "finalizing"
                              ? t("codex.batchImport.importing", "导入中")
                              : activeBatchImportCheckQuota
                                ? t("codex.batchImport.scanning", "扫描中")
                                : t("codex.batchImport.parsing", "解析中")
                            : batchImportPreview?.status === "cancelled"
                              ? t("codex.batchImport.cancelled", "已取消")
                              : batchImportPreview
                                ? activeBatchImportCheckQuota
                                  ? t("codex.batchImport.scanDone", "扫描完成")
                                  : t("codex.batchImport.parseDone", "解析完成")
                                : activeBatchImportCheckQuota
                                  ? t("codex.batchImport.scanning", "扫描中")
                                  : t("codex.batchImport.parsing", "解析中")}
                      </span>
                      <strong>
                        {batchImportProgressCurrent}/{batchImportProgressTotal}
                      </strong>
                    </div>
                    <div className="codex-batch-import-progress-track">
                      <div
                        className="codex-batch-import-progress-fill"
                        style={{
                          width: `${batchImportProgressPercent}%`,
                        }}
                      />
                    </div>
                    {batchImportProgress?.currentLabel && (
                      <div className="codex-batch-import-current">
                        {t("codex.batchImport.current", "当前")}：
                        {maskAccountText(batchImportProgress.currentLabel)}
                      </div>
                    )}
                  </div>
                )}

                {batchImportResult ? (
                  <div className="codex-batch-import-result">
                    {batchImportResult.cancelled && (
                      <div className="codex-batch-import-cancelled-note">
                        {t(
                          "codex.batchImport.importCancelledSummary",
                          "导入已取消，已处理 {{processed}}/{{total}} 个账号。",
                        )
                          .replace(
                            "{{processed}}",
                            String(batchImportResult.processed),
                          )
                          .replace(
                            "{{total}}",
                            String(batchImportResult.total),
                          )}
                      </div>
                    )}
                    <div className="codex-batch-import-stat-grid">
                      <div>
                        <span>{t("codex.batchImport.imported", "已导入")}</span>
                        <strong>{batchImportResult.imported.length}</strong>
                      </div>
                      <div>
                        <span>{t("codex.batchImport.failed", "失败")}</span>
                        <strong>{batchImportResult.failed.length}</strong>
                      </div>
                    </div>
                    {batchImportResult.failed.length > 0 && (
                      <div className="codex-batch-import-list compact">
                        {batchImportResult.failed.map((item) => (
                          <div
                            className="codex-batch-import-row"
                            key={item.email}
                          >
                            <div>
                              <strong>{maskAccountText(item.email)}</strong>
                              <small>{item.error}</small>
                            </div>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                ) : batchImportPreview ? (
                  <>
                    <div className="codex-batch-import-stat-grid">
                      <div>
                        <span>
                          {t("codex.batchImport.groups.ready", "可导入")}
                        </span>
                        <strong>{batchImportCounts.ready}</strong>
                      </div>
                      <div>
                        <span>
                          {t("codex.batchImport.groups.quotaFailed", "异常")}
                        </span>
                        <strong>{batchImportCounts.quotaFailed}</strong>
                      </div>
                      <div>
                        <span>
                          {t("codex.batchImport.groups.existing", "已存在")}
                        </span>
                        <strong>{batchImportCounts.existing}</strong>
                      </div>
                      <div>
                        <span>
                          {t("codex.batchImport.groups.invalid", "无效账号")}
                        </span>
                        <strong>{batchImportCounts.invalid}</strong>
                      </div>
                    </div>

                    <div className="codex-batch-import-toolbar">
                      <div className="codex-batch-import-toolbar-main">
                        <span>{batchImportSelectedCountLabel}</span>
                        <label className="codex-batch-import-check-toggle">
                          <input
                            type="checkbox"
                            checked={batchImportCheckQuota}
                            disabled={batchImportBusy}
                            onChange={(event) =>
                              void handleBatchImportCheckQuotaChange(
                                event.target.checked,
                              )
                            }
                          />
                          <span className="codex-batch-import-check-switch" />
                          <span>
                            {t(
                              "codex.batchImport.checkQuotaToggle",
                              "导入前检测账号",
                            )}
                          </span>
                        </label>
                        <label className="codex-batch-import-check-toggle">
                          <input
                            type="checkbox"
                            checked={syncImportedToApiService}
                            disabled={batchImportBusy}
                            onChange={(event) =>
                              handleSyncImportedToApiServiceChange(
                                event.target.checked,
                              )
                            }
                          />
                          <span className="codex-batch-import-check-switch" />
                          <span>
                            {t(
                              "codex.importApiService.toggle",
                              "同步加入 API 服务",
                            )}
                          </span>
                        </label>
                      </div>
                      <div className="codex-batch-import-actions">
                        <button
                          type="button"
                          className="btn btn-secondary compact"
                          disabled={
                            batchImportBusy ||
                            batchImportSelectableIds.length === 0
                          }
                          onClick={selectAllBatchImportAccounts}
                        >
                          {t(
                            "codex.batchImport.selectAllAccounts",
                            "选择全部账号",
                          )}
                        </button>
                        <button
                          type="button"
                          className="btn btn-secondary compact"
                          disabled={
                            batchImportBusy ||
                            batchImportCounts.ready +
                              batchImportCounts.existing ===
                              0
                          }
                          onClick={selectReadyBatchImportAccounts}
                        >
                          {t("codex.batchImport.selectReady", "选择正常账号")}
                        </button>
                        <button
                          type="button"
                          className="btn btn-secondary compact"
                          disabled={
                            batchImportBusy ||
                            batchImportSelectedSelectableCount === 0
                          }
                          onClick={clearBatchImportSelection}
                        >
                          {t("codex.batchImport.clearSelection", "取消选择")}
                        </button>
                      </div>
                    </div>

                    <div className="codex-batch-import-tags-row">
                      <label
                        className="codex-batch-import-tags-label"
                        htmlFor="codex-batch-import-tags"
                      >
                        {t("codex.batchImport.bulkTagsLabel", "导入后批量打标")}
                      </label>
                      <input
                        id="codex-batch-import-tags"
                        type="text"
                        className="codex-batch-import-tags-input"
                        value={batchImportTagsInput}
                        disabled={batchImportBusy}
                        onChange={(event) =>
                          setBatchImportTagsInput(event.target.value)
                        }
                        placeholder={t(
                          "codex.batchImport.bulkTagsPlaceholder",
                          "可选，多个标签用逗号或空格分隔",
                        )}
                      />
                    </div>

                    <div className="codex-batch-import-list">
                      {[...batchImportVisibleItems].reverse().map((item) => {
                        const selectable = batchImportSelectableIdSet.has(
                          item.itemId,
                        );
                        const checked =
                          selectable &&
                          batchImportSelectedIds.includes(item.itemId);
                        return (
                          <label
                            className={`codex-batch-import-row status-${item.status}`}
                            key={item.itemId}
                          >
                            <input
                              type="checkbox"
                              checked={checked}
                              disabled={!selectable || batchImportBusy}
                              onChange={() =>
                                toggleBatchImportItem(item.itemId)
                              }
                            />
                            <div className="codex-batch-import-row-main">
                              <div className="codex-batch-import-row-title">
                                <strong>{maskAccountText(item.label)}</strong>
                                <span>{item.accountType}</span>
                              </div>
                              <div className="codex-batch-import-row-meta">
                                <span>{item.source}</span>
                                {item.provider && <span>{item.provider}</span>}
                                {item.status === "ready" && (
                                  <span>
                                    {activeBatchImportCheckQuota
                                      ? t(
                                          "codex.batchImport.quotaOk",
                                          "账号正常",
                                        )
                                      : t(
                                          "codex.batchImport.groups.ready",
                                          "可导入",
                                        )}
                                  </span>
                                )}
                                {item.status === "quota_failed" && (
                                  <span>
                                    {t("codex.batchImport.quotaFailed", "异常")}
                                  </span>
                                )}
                                {item.status === "existing" && (
                                  <span>
                                    {t(
                                      "codex.batchImport.groups.existing",
                                      "已存在",
                                    )}
                                  </span>
                                )}
                                {item.status === "invalid" && (
                                  <span>
                                    {t(
                                      "codex.batchImport.groups.invalid",
                                      "无效账号",
                                    )}
                                  </span>
                                )}
                              </div>
                              {(item.quotaError || item.error) && (
                                <small className="codex-batch-import-row-error">
                                  {item.quotaError || item.error}
                                </small>
                              )}
                            </div>
                          </label>
                        );
                      })}
                    </div>
                  </>
                ) : (
                  <div className="codex-batch-import-empty">
                    <RefreshCw size={18} className="loading-spinner" />
                    {t("codex.batchImport.preparing", "正在准备导入任务...")}
                  </div>
                )}
              </div>

              <div className="modal-footer codex-batch-import-footer">
                {batchImportResult ? (
                  <button
                    className="btn btn-primary"
                    onClick={() => void handleCloseBatchImport()}
                  >
                    {t("common.shared.close", "关闭")}
                  </button>
                ) : (
                  <>
                    <button
                      className="btn btn-secondary"
                      onClick={() =>
                        batchImportBusy
                          ? void handleCancelBatchImport()
                          : void handleCloseBatchImport()
                      }
                      disabled={batchImportProgress?.phase === "cancelling"}
                    >
                      {batchImportBusy
                        ? batchImportProgress?.phase === "importing" ||
                          batchImportProgress?.phase === "finalizing"
                          ? t("codex.batchImport.cancelImport", "取消导入")
                          : activeBatchImportCheckQuota
                            ? t("codex.batchImport.cancelScan", "取消扫描")
                            : t("codex.batchImport.cancelParse", "取消解析")
                        : t("common.shared.close", "关闭")}
                    </button>
                    {!batchImportBusy &&
                      batchImportPreview?.status === "cancelled" && (
                        <button
                          className="btn btn-secondary"
                          onClick={() => void handleResumeBatchImport()}
                        >
                          <RefreshCw size={16} />
                          {activeBatchImportCheckQuota
                            ? t("codex.batchImport.resumeScan", "继续扫描")
                            : t("codex.batchImport.resumeParse", "继续解析")}
                        </button>
                      )}
                    {!batchImportBusy && (
                      <button
                        className="btn btn-primary"
                        onClick={() => void handleConfirmBatchImport()}
                        disabled={
                          !batchImportPreview ||
                          batchImportSelectedSelectableCount === 0
                        }
                      >
                        <Download size={16} />
                        {activeBatchImportCheckQuota
                          ? t(
                              "codex.batchImport.importChecked",
                              "导入已检测账号",
                            )
                          : t(
                              "codex.batchImport.directImport",
                              "不检测，直接导入",
                            )}
                        {batchImportSelectedSelectableCount > 0
                          ? ` (${batchImportSelectedSelectableCount})`
                          : ""}
                      </button>
                    )}
                    {!batchImportBusy && (
                      <button
                        className="btn btn-success"
                        onClick={() =>
                          void handleConfirmBatchImport({
                            addToApiService: true,
                          })
                        }
                        disabled={
                          localAccessSaving ||
                          !batchImportPreview ||
                          batchImportSelectedSelectableCount === 0
                        }
                      >
                        {localAccessSaving ? (
                          <RefreshCw size={16} className="loading-spinner" />
                        ) : (
                          <Database size={16} />
                        )}
                        {activeBatchImportCheckQuota
                          ? t(
                              "codex.batchImport.importCheckedAndAddToApiService",
                              "导入已检测账号并添加到 API 服务",
                            )
                          : t(
                              "codex.batchImport.directImportAndAddToApiService",
                              "直接导入并添加到 API 服务",
                            )}
                        {batchImportSelectedSelectableCount > 0
                          ? ` (${batchImportSelectedSelectableCount})`
                          : ""}
                      </button>
                    )}
                  </>
                )}
              </div>
            </div>
          </div>,
          document.body,
        )}

      {externalImportProgress.visible && (
        <div className="modal-overlay codex-external-import-overlay">
          <div
            className="modal-content codex-external-import-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>
                {t("common.shared.externalImport.titleCodex", "Codex 批量导入")}
              </h2>
              {!externalImportRunning && (
                <button
                  className="modal-close"
                  onClick={closeExternalImportProgressModal}
                  aria-label={t("common.close", "关闭")}
                >
                  <X />
                </button>
              )}
            </div>
            <div className="codex-external-import-body">
              <div className="codex-external-import-main">
                <div className="codex-external-import-primary">
                  <div
                    className={`codex-external-import-status is-${externalImportProgress.status}`}
                  >
                    {externalImportRunning ? (
                      <RefreshCw size={18} className="loading-spinner" />
                    ) : externalImportProgress.status === "success" ? (
                      <Check size={18} />
                    ) : (
                      <CircleAlert size={18} />
                    )}
                    <span>{externalImportProgress.message}</span>
                  </div>

                  <div className="codex-external-import-progress-card">
                    <div className="codex-external-import-progress-head">
                      <span>{externalImportPercent}%</span>
                      <strong>
                        {externalImportProgress.current > 0 &&
                        externalImportProgress.total > 0
                          ? `${externalImportProgress.current}/${externalImportProgress.total}`
                          : ""}
                      </strong>
                    </div>
                    <div className="codex-external-import-progress-track">
                      <div
                        className="codex-external-import-progress-fill"
                        style={{ width: `${externalImportPercent}%` }}
                      />
                    </div>
                  </div>
                </div>

                <div className="codex-external-import-side">
                  <div className="codex-external-import-stats">
                    <div>
                      <span>
                        {t("common.shared.externalImport.total", "总数")}
                      </span>
                      <strong>{externalImportProgress.total}</strong>
                    </div>
                    <div>
                      <span>
                        {t("common.shared.externalImport.success", "成功")}
                      </span>
                      <strong>{externalImportProgress.success}</strong>
                    </div>
                    <div>
                      <span>
                        {t("common.shared.externalImport.failed", "失败")}
                      </span>
                      <strong>{externalImportProgress.failed}</strong>
                    </div>
                  </div>
                </div>
              </div>

              <div className="codex-external-import-steps">
                {externalImportSteps.map((label, index) => {
                  const isDone = externalImportStepIndex > index;
                  const isActive = externalImportStepIndex === index;
                  return (
                    <div
                      key={label}
                      className={`codex-external-import-step ${isDone ? "is-done" : ""} ${isActive ? "is-active" : ""}`}
                    >
                      <span>{isDone ? <Check size={13} /> : index + 1}</span>
                      <strong>{label}</strong>
                    </div>
                  );
                })}
              </div>

              {externalImportProgress.failures.length > 0 && (
                <div className="codex-external-import-errors">
                  <div className="codex-external-import-errors-head">
                    <strong>
                      {t("common.shared.externalImport.errorsTitle", "失败项")}
                    </strong>
                    <button
                      className="btn btn-secondary btn-sm"
                      onClick={handleCopyExternalImportErrors}
                    >
                      <Copy size={13} />
                      {t("common.shared.externalImport.copyErrors", "复制错误")}
                    </button>
                  </div>
                  <div className="codex-external-import-error-list">
                    {externalImportProgress.failures.map((item) => (
                      <div
                        key={`${item.index}-${item.label}`}
                        className="codex-external-import-error"
                      >
                        <span>
                          {item.index}. {item.label}
                        </span>
                        <small>{item.error}</small>
                      </div>
                    ))}
                  </div>
                </div>
              )}
              {externalImportSyncError && (
                <div className="codex-import-api-service-error" role="alert">
                  <CircleAlert size={16} />
                  <span>
                    {t(
                      "codex.importApiService.syncFailed",
                      "账号已导入，但加入 API 服务失败：{{error}}",
                    ).replace("{{error}}", externalImportSyncError)}
                  </span>
                </div>
              )}
            </div>
            {!externalImportRunning && (
              <div className="modal-footer codex-external-import-footer">
                <button
                  className="btn btn-secondary"
                  onClick={closeExternalImportProgressModal}
                >
                  {t("common.close", "关闭")}
                </button>
                <button
                  className="btn btn-primary"
                  onClick={handleViewExternalImportAccounts}
                >
                  {t(
                    "common.shared.externalImport.viewAccounts",
                    "查看 Codex 账号",
                  )}
                </button>
              </div>
            )}
          </div>
        </div>
      )}

      {renderCockpitApiServicePanel()}
      {renderApiKeyUsageDetailModal()}
      {renderQuotaErrorDetailModal()}

      {activeTab === "overview" && <CodexAccountsOverviewPanel {...props} />}

      {cliLaunchModal && (
        <CodexCliLaunchDialog
          subjectLabel={t("instances.columns.account", "账号")}
          subjectValue={cliLaunchModal.accountLabel}
          workingDir={{
            value: cliLaunchModal.workingDir,
            error: cliLaunchModal.workingDirError,
            onChange: updateCodexCliWorkingDir,
            onBlur: () => {
              if (
                cliLaunchModal.workingDir.trim() &&
                !cliLaunchModal.instanceId &&
                !cliLaunchModal.preparing &&
                !cliLaunchModal.executing
              ) {
                void prepareCodexCliLaunch(cliLaunchModal);
              }
            },
            onChoose: () => void handleChooseCodexCliWorkingDir(),
          }}
          terminal={selectedTerminal}
          terminalOptions={terminalOptions}
          onTerminalChange={setSelectedTerminal}
          command={cliLaunchModal.terminalCommand}
          commandPlaceholder={
            cliLaunchModal.preparing
              ? t("common.loading", "加载中...")
              : t("codex.cli.selectWorkingDir", "选择 Codex CLI 工作目录")
          }
          preparing={cliLaunchModal.preparing}
          copied={cliLaunchModal.copied}
          executing={cliLaunchModal.executing}
          successMessage={cliLaunchModal.executeMessage}
          errorMessage={cliLaunchModal.executeError}
          onClose={closeCliLaunchModal}
          onCopy={() => void handleCopyCodexCliCommand()}
          onExecute={() => void handleExecuteCodexCli()}
          showCancelButton
        />
      )}
      {activeLaunchPreviewAccount && (
        <CodexLaunchPreviewModal
          account={activeLaunchPreviewAccount}
          accountLabel={maskAccountText(
            buildCodexAccountPresentation(activeLaunchPreviewAccount, t)
              .displayName ||
              activeLaunchPreviewAccount.email ||
              activeLaunchPreviewAccount.id,
          )}
          summary={buildAccountLaunchPreviewSummary(activeLaunchPreviewAccount)}
          actions={buildAccountLaunchPreviewActions(activeLaunchPreviewAccount)}
          instanceId={launchPreviewInstanceId}
          instanceLabel={launchPreviewInstanceLabel}
          instanceOptions={launchPreviewInstanceOptions}
          onInstanceChange={setLaunchPreviewInstanceId}
          onClose={() => setLaunchPreviewAccount(null)}
          onExecute={handleExecuteLaunchPreview}
        />
      )}
      {localAccessLaunchPreviewOpen && (
        <CodexLaunchPreviewModal
          accountLabel={t("codex.localAccess.title", "API 服务")}
          accountMetaLabel={t("codex.apiSwitchNotice.type.apiKey", "API 密钥")}
          summary={buildLocalAccessLaunchPreviewSummary()}
          actions={buildLocalAccessLaunchPreviewActions()}
          instanceId={launchPreviewInstanceId}
          instanceLabel={launchPreviewInstanceLabel}
          instanceOptions={launchPreviewInstanceOptions}
          onInstanceChange={setLaunchPreviewInstanceId}
          mode="apiService"
          onClose={() => setLocalAccessLaunchPreviewOpen(false)}
          onExecute={handleExecuteLocalAccessLaunchPreview}
        />
      )}
      {deepSeekStart.modal}

      {activeTab === "instances" && (
        <CodexInstancesContent
          accountsForSelect={sortedAccountsForInstances}
          resolveLaunchPreviewSummary={buildAccountLaunchPreviewSummary}
          resolveLaunchPreviewActions={buildAccountLaunchPreviewActions}
          localAccessLaunchPreviewSummary={
            localAccessCollection
              ? buildLocalAccessLaunchPreviewSummary()
              : undefined
          }
          localAccessLaunchPreviewActions={
            localAccessCollection
              ? buildLocalAccessLaunchPreviewActions()
              : undefined
          }
        />
      )}

      {activeTab === "sessions" && <CodexSessionManager />}

      {activeTab === "providers" && (
        <CodexModelProviderManager
          accounts={accounts}
          onProvidersChanged={setManagedProviders}
          onManageModelPresets={() => {
            setActiveTab("wakeup");
            setWakeupPresetManagerSignal((value) => value + 1);
          }}
        />
      )}

      {activeTab === "wakeup" && (
        <CodexWakeupContent
          accounts={accounts}
          openPresetManagerSignal={wakeupPresetManagerSignal}
          onRefreshAccounts={async () => {
            await fetchAccounts();
            await fetchCurrentAccount();
          }}
        />
      )}

      {activeTab !== "wakeup" && fullQuotaWakeupOpenRequest && (
        <CodexWakeupContent
          accounts={accounts}
          openTestRequest={fullQuotaWakeupOpenRequest}
          modalOnly
          onRefreshAccounts={async () => {
            await fetchAccounts();
            await fetchCurrentAccount();
          }}
        />
      )}
    </div>
  );
}
