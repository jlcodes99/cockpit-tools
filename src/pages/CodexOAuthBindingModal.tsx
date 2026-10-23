import { useEffect, useRef, useState } from "react";
import { useModalFocusTrap } from "../hooks/useModalFocusTrap";
import { useEscCloseTopmost } from "../hooks/useEscClose";
import {
  ArrowDown,
  ArrowDownWideNarrow,
  ArrowUp,
  CircleAlert,
  Clock,
  Pencil,
  Search,
  X,
} from "lucide-react";
import { AccountTagFilterDropdown } from "../components/AccountTagFilterDropdown";
import { ModalErrorMessage } from "../components/ModalErrorMessage";
import { MultiSelectFilterDropdown } from "../components/MultiSelectFilterDropdown";
import { PaginationControls } from "../components/PaginationControls";
import { SingleSelectFilterDropdown } from "../components/SingleSelectFilterDropdown";
import {
  isCodexAgentIdentityAccount,
  isCodexWebSessionAccount,
} from "../types/codex";
import { isCodexOAuthBindingEligibleAccount } from "../utils/codexLocalAccessAccounts";
import { parseCodexSwitchAuthFailure } from "../utils/codexSwitchAuthFailure";
import type { CodexAccountsViewProps } from "./CodexAccountsView";
import "./CodexOAuthBindingModal.css";

/**
 * 账号级 OAuth 绑定弹框。
 * 从 CodexAccountsOverviewPanel 拆出：账号总览与启动预览共用同一个弹框，
 * 由 CodexAccountsView 渲染，任意页签下都能正常显示。
 */
export function CodexOAuthBindingModal(props: CodexAccountsViewProps) {
  const {
    clearFilterTypes,
    clearTagFilter,
    closeOAuthBindingModal,
    closeOAuthBindingQuotaReserveEditor,
    codexAccountSortOptions,
    confirmOAuthBindingQuotaReserveEditor,
    filterTypes,
    handleClearOAuthBinding,
    handleOAuthBindingQuotaReserveToggle,
    handleReauthorizeOAuthBinding,
    handleSubmitOAuthBinding,
    isLocalAccessOAuthBinding,
    maskAccountText,
    oauthAccounts,
    oauthBindingAccount,
    oauthBindingAvailableTags,
    oauthBindingEligibleAccounts,
    oauthBindingError,
    oauthBindingErrorScrollKey,
    oauthBindingFilteredAccounts,
    oauthBindingHasExistingBinding,
    oauthBindingHourlyReserveDraft,
    oauthBindingHourlyReserveInputRef,
    oauthBindingPagination,
    oauthBindingQuotaReserve,
    oauthBindingQuotaReserveEditorOpen,
    oauthBindingQuotaReserveFieldErrors,
    oauthBindingSaving,
    oauthBindingSelectedAccountId,
    oauthBindingTargetActive,
    oauthBindingTargetKind,
    oauthBindingTierCounts,
    oauthBindingTierFilterOptions,
    oauthBindingWeeklyReserveDraft,
    oauthBindingWeeklyReserveInputRef,
    openCodexAddModal,
    openOAuthBindingQuotaReserveEditor,
    resolvePresentation,
    resolveSubscriptionPresentation,
    searchQuery,
    selectedOAuthBindingAccount,
    setOauthBindingError,
    setOauthBindingHourlyReserveDraft,
    setOauthBindingQuotaReserveFieldErrors,
    setOauthBindingSelectedAccountId,
    setOauthBindingWeeklyReserveDraft,
    setSearchQuery,
    setSortBy,
    setSortDirection,
    sortBy,
    sortDirection,
    t,
    tagFilter,
    toggleFilterTypeValue,
    toggleTagFilterValue,
    validateOAuthBindingQuotaReserveField,
  } = props;

  const bindingDialogRef = useRef<HTMLDivElement>(null);
  const reserveDialogRef = useRef<HTMLDivElement>(null);
  const bindingOpen = Boolean(oauthBindingTargetActive);
  const reserveOpen = Boolean(oauthBindingQuotaReserveEditorOpen && isLocalAccessOAuthBinding);
  const [marginDraft, setMarginDraft] = useState("3");
  const [marginError, setMarginError] = useState<string | null>(null);
  const marginInputRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!reserveOpen) return;
    setMarginDraft(String(oauthBindingQuotaReserve?.safetyMarginPercent ?? 3));
    setMarginError(null);
  }, [reserveOpen, oauthBindingQuotaReserve?.safetyMarginPercent]);
  const confirmReserve = () => {
    setOauthBindingQuotaReserveFieldErrors({});
    setMarginError(null);
    const margin = Number(marginDraft);
    if (!/^\d+$/.test(marginDraft) || !Number.isInteger(margin) || margin < 1 || margin > 20) {
      setMarginError(t("codex.localAccess.oauthBinding.safetyMarginInvalid"));
      marginInputRef.current?.scrollIntoView({ block: "nearest" });
      marginInputRef.current?.focus();
      return;
    }
    setMarginError(null);
    confirmOAuthBindingQuotaReserveEditor({ safetyMarginPercent: margin });
  };
  useModalFocusTrap(bindingDialogRef, bindingOpen, true, !reserveOpen);
  useModalFocusTrap(reserveDialogRef, reserveOpen);
  useEscCloseTopmost(bindingOpen && !reserveOpen && !oauthBindingSaving, closeOAuthBindingModal, bindingDialogRef);
  useEscCloseTopmost(reserveOpen, closeOAuthBindingQuotaReserveEditor, reserveDialogRef);

  if (!oauthBindingTargetActive && !oauthBindingQuotaReserveEditorOpen) {
    return null;
  }

  return (
    <>
      {oauthBindingTargetActive && (
        <div className="modal-overlay">
          <div
            className="modal-content codex-add-modal codex-oauth-binding-modal"
            ref={bindingDialogRef}
            role="dialog"
            aria-modal={!reserveOpen}
            aria-hidden={reserveOpen || undefined}
            aria-label={t("codex.api.oauthBinding.title", "绑定 OAuth 账号")}
            tabIndex={-1}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-header">
              <h2>{t("codex.api.oauthBinding.title", "绑定 OAuth 账号")}</h2>
              <button
                className="modal-close"
                onClick={closeOAuthBindingModal}
                aria-label={t("common.close", "关闭")}
                disabled={oauthBindingSaving}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage
                message={oauthBindingError}
                scrollKey={oauthBindingErrorScrollKey}
              />
              {parseCodexSwitchAuthFailure(oauthBindingError) && (
                <div className="codex-oauth-binding-reauthorize">
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={handleReauthorizeOAuthBinding}
                    disabled={oauthBindingSaving}
                  >
                    {t("common.reauthorize", "重新授权")}
                  </button>
                </div>
              )}
              <div className="add-section">
                <div className="codex-oauth-binding-context">
                  <p className="section-desc codex-oauth-binding-desc">
                    {oauthBindingTargetKind === "local_access"
                      ? t(
                          "codex.localAccess.oauthBinding.desc",
                          "可选绑定。只要 OAuth 账号带 refresh_token 即可选择；未绑定时 API 服务按原 API Key 逻辑运行；绑定后登录态使用 OAuth 账号，Provider 使用当前 API 服务配置。",
                        )
                      : t(
                          "codex.api.oauthBinding.desc",
                          "可选绑定。只要 OAuth 账号带 refresh_token 即可选择；未绑定时该账号按原 API Key 逻辑切换；绑定后登录态使用 OAuth 账号，Provider 使用当前 API Key 账号配置。",
                        )}
                  </p>
                  <div className="section-desc codex-oauth-binding-current-target">
                    {oauthBindingTargetKind === "local_access"
                      ? t("codex.localAccess.oauthBinding.currentService", {
                          defaultValue: "API 服务：{{name}}",
                          name: t("codex.localAccess.title", "API 服务"),
                        })
                      : oauthBindingAccount
                        ? t("codex.api.oauthBinding.currentAccount", {
                            defaultValue: "API Key 账号：{{name}}",
                            name: maskAccountText(
                              resolvePresentation(oauthBindingAccount)
                                .displayName,
                            ),
                          })
                        : null}
                  </div>
                </div>
                <div className="codex-oauth-binding-picker">
                  <div className="codex-oauth-binding-picker-header">
                    <label>
                      {t(
                        "codex.api.oauthBinding.selectLabel",
                        "选择 OAuth 账号",
                      )}
                    </label>
                    <div className="codex-oauth-binding-picker-controls">
                      {isLocalAccessOAuthBinding && (
                        <div className="codex-oauth-binding-quota-control">
                          <label
                            className="codex-oauth-binding-gateway-toggle codex-oauth-binding-quota-toggle"
                            title={t(
                              "codex.localAccess.oauthBinding.quotaReserveDesc",
                              "API 服务仅在 5 小时和周剩余额度均高于保留值时使用该 OAuth 账号。",
                            )}
                          >
                            <input
                              type="checkbox"
                              checked={Boolean(oauthBindingQuotaReserve)}
                              onChange={(event) =>
                                handleOAuthBindingQuotaReserveToggle(
                                  event.target.checked,
                                )
                              }
                              disabled={oauthBindingSaving}
                            />
                            <span
                              className="codex-oauth-binding-checkbox-ui"
                              aria-hidden="true"
                            />
                            <span>
                              {t(
                                "codex.localAccess.oauthBinding.quotaReserveToggle",
                                "保留 OAuth 额度",
                              )}
                            </span>
                          </label>
                          {oauthBindingQuotaReserve && (
                            <button
                              type="button"
                              className="btn codex-oauth-binding-quota-summary"
                              onClick={openOAuthBindingQuotaReserveEditor}
                              disabled={oauthBindingSaving}
                              title={`${t(
                                "codex.localAccess.oauthBinding.quotaReserveHourlyLabel",
                                "5 小时保留",
                              )} ${oauthBindingQuotaReserve.hourlyPercent}% · ${t(
                                "codex.localAccess.oauthBinding.quotaReserveWeeklyLabel",
                                "周保留",
                              )} ${oauthBindingQuotaReserve.weeklyPercent}%`}
                              aria-label={`${t("instances.actions.edit", "编辑")} ${t(
                                "codex.localAccess.oauthBinding.quotaReserveToggle",
                                "保留 OAuth 额度",
                              )}`}
                            >
                              <span>
                                {t("codex.instances.quota.hourly", "5小时")}{" "}
                                <strong>{oauthBindingQuotaReserve.hourlyPercent}%</strong>
                              </span>
                              <span>
                                {t("codex.instances.quota.weekly", "周")}{" "}
                                <strong>{oauthBindingQuotaReserve.weeklyPercent}%</strong>
                              </span>
                              <Pencil size={12} />
                            </button>
                          )}
                        </div>
                      )}
                    </div>
                  </div>
                  {oauthAccounts.length === 0 ? (
                    <div className="add-status error">
                      <CircleAlert size={16} />
                      <span>
                        {t(
                          "codex.api.oauthBinding.empty",
                          "暂无 OAuth 账号，请先添加 OAuth 授权账号。",
                        )}
                      </span>
                    </div>
                  ) : (
                    <>
                      {oauthBindingEligibleAccounts.length === 0 && (
                        <div className="add-status error">
                          <CircleAlert size={16} />
                          <span>
                            {t(
                              "codex.api.oauthBinding.emptyEligible",
                              "没有带 refresh_token 的 OAuth 账号，请重新 OAuth 授权或添加符合条件的 OAuth 账号。",
                            )}
                          </span>
                        </div>
                      )}
                      <div className="codex-oauth-binding-toolbar">
                        <div className="search-box codex-oauth-binding-search">
                          <Search size={16} className="search-icon" />
                          <input
                            type="text"
                            placeholder={t(
                              "common.shared.search",
                              "搜索账号...",
                            )}
                            value={searchQuery}
                            onChange={(event) =>
                              setSearchQuery(event.target.value)
                            }
                            disabled={oauthBindingSaving}
                          />
                        </div>
                        <MultiSelectFilterDropdown
                          options={oauthBindingTierFilterOptions}
                          selectedValues={filterTypes}
                          allLabel={t("common.shared.filter.all", {
                            count: oauthBindingTierCounts.all,
                          })}
                          filterLabel={t("common.shared.filterLabel", "筛选")}
                          clearLabel={t("accounts.clearFilter", "清空筛选")}
                          emptyLabel={t("common.none", "暂无")}
                          ariaLabel={t("common.shared.filterLabel", "筛选")}
                          onToggleValue={toggleFilterTypeValue}
                          onClear={clearFilterTypes}
                        />
                        <AccountTagFilterDropdown
                          availableTags={oauthBindingAvailableTags}
                          selectedTags={tagFilter}
                          onToggleTag={toggleTagFilterValue}
                          onClear={clearTagFilter}
                        />
                        <SingleSelectFilterDropdown
                          value={sortBy}
                          options={codexAccountSortOptions}
                          ariaLabel={t("common.shared.sortLabel", "排序")}
                          icon={<ArrowDownWideNarrow size={14} />}
                          disabled={oauthBindingSaving}
                          onChange={setSortBy}
                        />
                        {sortBy !== "custom" && (
                          <button
                            type="button"
                            className="sort-direction-btn"
                            onClick={() =>
                              setSortDirection((prev) =>
                                prev === "desc" ? "asc" : "desc",
                              )
                            }
                            disabled={oauthBindingSaving}
                            title={
                              sortDirection === "desc"
                                ? t(
                                    "common.shared.sort.descTooltip",
                                    "当前：降序，点击切换为升序",
                                  )
                                : t(
                                    "common.shared.sort.ascTooltip",
                                    "当前：升序，点击切换为降序",
                                  )
                            }
                            aria-label={t(
                              "common.shared.sort.toggleDirection",
                              "切换排序方向",
                            )}
                          >
                            {sortDirection === "desc" ? (
                              <ArrowDown size={15} />
                            ) : (
                              <ArrowUp size={15} />
                            )}
                          </button>
                        )}
                      </div>
                      {oauthBindingFilteredAccounts.length === 0 ? (
                        <div className="group-account-empty">
                          <span>
                            {t("common.shared.noMatch.title", "没有匹配的账号")}
                          </span>
                        </div>
                      ) : (
                        <div className="codex-oauth-binding-list">
                          {oauthBindingPagination.pageItems.map((account) => {
                            const presentation = resolvePresentation(account);
                            const subscriptionInfo =
                              resolveSubscriptionPresentation(account);
                            const selected =
                              oauthBindingSelectedAccountId === account.id;
                            const eligible =
                              isCodexOAuthBindingEligibleAccount(account);
                            const rowDisabled = oauthBindingSaving || !eligible;
                            const emailText = maskAccountText(
                              account.email ||
                                account.account_name ||
                                presentation.displayName ||
                                account.id,
                            );
                            return (
                              <label
                                key={account.id}
                                className={`codex-oauth-binding-row ${selected ? "is-selected" : ""}`}
                                aria-label={emailText}
                                aria-disabled={rowDisabled}
                                title={
                                  eligible
                                    ? emailText
                                    : isCodexAgentIdentityAccount(account)
                                      ? t(
                                          "codex.agentIdentityRegistration.oauthBindingUnsupported",
                                          "Agent Identity 账号仅用于 API 服务，不能作为 OAuth 绑定账号。",
                                        )
                                      : isCodexWebSessionAccount(account)
                                        ? t(
                                            "codex.webSessionImport.oauthBindingUnsupported",
                                            "Web Session 账号仅支持查看额度，不能作为 OAuth 绑定账号。",
                                          )
                                        : t(
                                            "codex.api.oauthBinding.validationSubscriptionRequired",
                                            "只能绑定带 refresh_token 的 OAuth 账号",
                                          )
                                }
                                onClick={(event) => {
                                  if (rowDisabled) {
                                    event.preventDefault();
                                    return;
                                  }
                                  setOauthBindingSelectedAccountId(account.id);
                                  setOauthBindingError(null);
                                }}
                              >
                                <input
                                  type="radio"
                                  name="codex-oauth-binding-account"
                                  checked={selected}
                                  onChange={() => {
                                    setOauthBindingSelectedAccountId(
                                      account.id,
                                    );
                                    setOauthBindingError(null);
                                  }}
                                  disabled={rowDisabled}
                                />
                                <div className="codex-oauth-binding-row-main">
                                  <span
                                    className="codex-oauth-binding-row-name"
                                    title={emailText}
                                  >
                                    {emailText}
                                  </span>
                                  <span
                                    className={`tier-badge codex-oauth-binding-row-plan ${presentation.planClass || "unknown"}`}
                                    title={presentation.planLabel}
                                  >
                                    {presentation.planLabel}
                                  </span>
                                  <span
                                    className={`codex-oauth-binding-row-term ${subscriptionInfo.tone}`}
                                    title={subscriptionInfo.titleText}
                                  >
                                    <Clock size={12} />
                                    <span>
                                      {t("codex.subscription.label", "有效期")}
                                    </span>
                                    <strong>
                                      {subscriptionInfo.valueText}
                                    </strong>
                                    <span>{subscriptionInfo.detailText}</span>
                                  </span>
                                </div>
                              </label>
                            );
                          })}
                        </div>
                      )}
                      <PaginationControls
                        totalItems={oauthBindingPagination.totalItems}
                        currentPage={oauthBindingPagination.currentPage}
                        totalPages={oauthBindingPagination.totalPages}
                        pageSize={oauthBindingPagination.pageSize}
                        pageSizeOptions={oauthBindingPagination.pageSizeOptions}
                        rangeStart={oauthBindingPagination.rangeStart}
                        rangeEnd={oauthBindingPagination.rangeEnd}
                        canGoPrevious={oauthBindingPagination.canGoPrevious}
                        canGoNext={oauthBindingPagination.canGoNext}
                        onPageSizeChange={oauthBindingPagination.setPageSize}
                        onPreviousPage={oauthBindingPagination.goToPreviousPage}
                        onNextPage={oauthBindingPagination.goToNextPage}
                      />
                    </>
                  )}
                </div>
                <div className="api-key-edit-actions">
                  {oauthAccounts.length === 0 && (
                    <button
                      className="btn btn-secondary"
                      onClick={() => {
                        closeOAuthBindingModal();
                        openCodexAddModal("oauth");
                      }}
                      disabled={oauthBindingSaving}
                    >
                      {t("codex.addModal.oauth", "OAuth 授权")}
                    </button>
                  )}
                  {oauthBindingHasExistingBinding && (
                    <button
                      className="btn btn-secondary codex-oauth-binding-clear"
                      onClick={() => void handleClearOAuthBinding()}
                      disabled={oauthBindingSaving}
                    >
                      {t("codex.api.oauthBinding.clearAction", "解除绑定")}
                    </button>
                  )}
                  <button
                    className="btn btn-secondary"
                    onClick={closeOAuthBindingModal}
                    disabled={oauthBindingSaving}
                  >
                    {t("common.cancel")}
                  </button>
                  <button
                    className="btn btn-primary"
                    onClick={() => void handleSubmitOAuthBinding()}
                    disabled={
                      oauthBindingSaving ||
                      !selectedOAuthBindingAccount ||
                      oauthBindingEligibleAccounts.length === 0
                    }
                  >
                    {oauthBindingSaving
                      ? t("common.saving", "保存中...")
                      : t("common.save")}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>
      )}
      {oauthBindingQuotaReserveEditorOpen && isLocalAccessOAuthBinding && (
        <div className="modal-overlay codex-oauth-binding-quota-overlay">
          <div
            className="modal-content codex-add-modal codex-oauth-binding-quota-modal"
            ref={reserveDialogRef}
            role="dialog"
            aria-modal="true"
            aria-label={t("codex.localAccess.oauthBinding.quotaReserveToggle", "保留 OAuth 额度")}
            tabIndex={-1}
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>
                {t(
                  "codex.localAccess.oauthBinding.quotaReserveToggle",
                  "保留 OAuth 额度",
                )}
              </h2>
              <button
                type="button"
                className="modal-close"
                onClick={closeOAuthBindingQuotaReserveEditor}
                aria-label={t("common.close", "关闭")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="add-section">
                <p className="section-desc codex-oauth-binding-quota-desc">
                  {t(
                    "codex.localAccess.oauthBinding.quotaReserveDesc",
                    "API 服务仅在 5 小时和周剩余额度均高于保留值时使用该 OAuth 账号。",
                  )}
                </p>
                <div className="codex-oauth-binding-quota-fields">
                  <label className="codex-oauth-binding-quota-field">
                    <span>
                      {t(
                        "codex.localAccess.oauthBinding.quotaReserveHourlyLabel",
                        "5 小时保留",
                      )}
                    </span>
                    <div className="codex-oauth-binding-quota-input-wrap">
                      <input
                        ref={oauthBindingHourlyReserveInputRef}
                        className={
                          oauthBindingQuotaReserveFieldErrors.hourlyPercent
                            ? "codex-account-note-input has-error"
                            : "codex-account-note-input"
                        }
                        type="text"
                        inputMode="numeric"
                        pattern="[0-9]*"
                        maxLength={3}
                        value={oauthBindingHourlyReserveDraft}
                        onChange={(event) => {
                          if (!/^\d*$/.test(event.target.value)) return;
                          setOauthBindingHourlyReserveDraft(event.target.value);
                          setOauthBindingQuotaReserveFieldErrors((prev) => ({
                            ...prev,
                            hourlyPercent: undefined,
                          }));
                        }}
                        onBlur={() =>
                          validateOAuthBindingQuotaReserveField(
                            "hourlyPercent",
                            oauthBindingHourlyReserveDraft,
                          )
                        }
                      />
                      <span aria-hidden="true">%</span>
                    </div>
                    {oauthBindingQuotaReserveFieldErrors.hourlyPercent && (
                      <span className="codex-account-note-field-error codex-oauth-binding-quota-error">
                        {oauthBindingQuotaReserveFieldErrors.hourlyPercent}
                      </span>
                    )}
                  </label>
                  <label className="codex-oauth-binding-quota-field">
                    <span>
                      {t(
                        "codex.localAccess.oauthBinding.quotaReserveWeeklyLabel",
                        "周保留",
                      )}
                    </span>
                    <div className="codex-oauth-binding-quota-input-wrap">
                      <input
                        ref={oauthBindingWeeklyReserveInputRef}
                        className={
                          oauthBindingQuotaReserveFieldErrors.weeklyPercent
                            ? "codex-account-note-input has-error"
                            : "codex-account-note-input"
                        }
                        type="text"
                        inputMode="numeric"
                        pattern="[0-9]*"
                        maxLength={3}
                        value={oauthBindingWeeklyReserveDraft}
                        onChange={(event) => {
                          if (!/^\d*$/.test(event.target.value)) return;
                          setOauthBindingWeeklyReserveDraft(event.target.value);
                          setOauthBindingQuotaReserveFieldErrors((prev) => ({
                            ...prev,
                            weeklyPercent: undefined,
                          }));
                        }}
                        onBlur={() =>
                          validateOAuthBindingQuotaReserveField(
                            "weeklyPercent",
                            oauthBindingWeeklyReserveDraft,
                          )
                        }
                      />
                      <span aria-hidden="true">%</span>
                    </div>
                    {oauthBindingQuotaReserveFieldErrors.weeklyPercent && (
                      <span className="codex-account-note-field-error codex-oauth-binding-quota-error">
                        {oauthBindingQuotaReserveFieldErrors.weeklyPercent}
                      </span>
                    )}
                  </label>
                </div>
                <div className="codex-oauth-binding-protection">
                  <p className="section-desc">{t("codex.localAccess.oauthBinding.strictReserveDesc")}</p>
                  <label className="codex-oauth-binding-quota-field">
                    <span>{t("codex.localAccess.oauthBinding.safetyMargin")}</span>
                    <div className="codex-oauth-binding-quota-input-wrap">
                      <input ref={marginInputRef} type="text" inputMode="numeric" pattern="[0-9]*" maxLength={2}
                        className={`codex-account-note-input${marginError ? " has-error" : ""}`}
                        value={marginDraft} aria-invalid={Boolean(marginError)}
                        onChange={(event) => { if (/^\d*$/.test(event.target.value)) { setMarginDraft(event.target.value); setMarginError(null); } }} />
                      <span>{t("codex.localAccess.oauthBinding.safetyMarginUnit")}</span>
                    </div>
                    {marginError && <span className="codex-account-note-field-error">{marginError}</span>}
                  </label>
                </div>
                <div className="api-key-edit-actions">
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={closeOAuthBindingQuotaReserveEditor}
                  >
                    {t("common.cancel", "取消")}
                  </button>
                  <button
                    type="button"
                    className="btn btn-primary"
                    onClick={confirmReserve}
                  >
                    {t("common.confirm", "确认")}
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
