import {
  ArrowDown,
  ArrowDownWideNarrow,
  ArrowUp,
  Clock,
  Search,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useModalFocusTrap } from "../../hooks/useModalFocusTrap";
import { useEscCloseTopmost } from "../../hooks/useEscClose";
import { usePagination } from "../../hooks/usePagination";
import { ModalErrorMessage } from "../ModalErrorMessage";
import { MultiSelectFilterDropdown } from "../MultiSelectFilterDropdown";
import { PaginationControls } from "../PaginationControls";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import { useCodexImageModelCatalog } from "./useCodexImageModelCatalog";
import { SingleSelectFilterDropdown } from "../SingleSelectFilterDropdown";
import {
  getCodexSubscriptionPresentation,
  isCodexApiKeyAccount,
  type CodexAccount,
} from "../../types/codex";
import { buildCodexAccountPresentation } from "../../presentation/platformAccountPresentation";
import {
  isPrivacyModeEnabledByDefault,
  maskSensitiveValue,
} from "../../utils/privacy";
import { eligibleImageAccountIds, isImageGenerationEligibleAccount, imageModelsForAccount, isImageModelInFamily, resolveImageAccountModel, selectedImageAccountModels } from "../../utils/codexImageAccounts";
export { isImageGenerationEligibleAccount } from "../../utils/codexImageAccounts";
import "./CodexImageAccountPickerModal.css";

type SortField = "last_used" | "created_at" | "account" | "plan";

const PAGE_SIZE_OPTIONS = [10, 20, 50] as const;

export interface CodexImageAccountPickerModalProps {
  accounts: CodexAccount[];
  selectedIds: string[];
  selectedModels?: Record<string, string>;
  imageGenerationModel?: string | null;
  /** 信息区展示的对话账号 / 供应商名称。 */
  contextLabel?: string;
  /** 生图模型等附加设置；传入后与账号选择合并到同一个弹框里。 */
  imageModelControl?: ReactNode;
  saving?: boolean;
  errorMessage?: string;
  onClearError?: () => void;
  onCancel: () => void;
  onConfirm: (accountIds: string[], models: Record<string, string>) => void;
}

function normalizePlanType(account: CodexAccount): string {
  return (account.plan_type || "").trim().toLowerCase();
}

export function CodexImageAccountPickerModal({
  accounts,
  selectedIds,
  selectedModels,
  imageGenerationModel,
  contextLabel,
  imageModelControl,
  saving = false,
  errorMessage,
  onClearError,
  onCancel,
  onConfirm,
}: CodexImageAccountPickerModalProps) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  useModalFocusTrap(dialog, true);
  const [query, setQuery] = useState("");
  const [planFilters, setPlanFilters] = useState<string[]>([]);
  const [sortBy, setSortBy] = useState<SortField>("last_used");
  const [sortDesc, setSortDesc] = useState(true);
  const [draftIds, setDraftIds] = useState<string[]>(selectedIds);
  const [draftModels, setDraftModels] = useState<Record<string, string>>(selectedModels ?? {});
  const [locateAccountId, setLocateAccountId] = useState<string | null>(null);
  const previousFamily = useRef(imageGenerationModel?.trim() || null);
  const registry = useCodexImageModelCatalog(imageGenerationModel || undefined);

  // 该弹框会叠加在启动预览 / API 服务弹框之上，ESC 只关闭最上层，避免连带关闭底层弹框。
  useEscCloseTopmost(!saving, onCancel, dialog);

  const candidates = useMemo(
    () => accounts.filter((account) => isImageGenerationEligibleAccount(account, imageGenerationModel, registry.deleted)),
    [accounts, imageGenerationModel, registry.deleted],
  );

  useEffect(() => {
    if (!imageGenerationModel || !registry.ready) return;
    setDraftIds((ids) => {
      const next = eligibleImageAccountIds(ids, accounts, imageGenerationModel, registry.deleted);
      return next.length === ids.length ? ids : next;
    });
  }, [accounts, imageGenerationModel, registry.ready, registry.deleted]);

  useEffect(() => {
    if (!imageGenerationModel || !registry.ready) return;
    const family = imageGenerationModel.trim();
    const changed = previousFamily.current !== null && previousFamily.current !== family;
    previousFamily.current = family;
    setDraftModels((models) => {
      const next = Object.fromEntries(Object.entries(models).filter(([id, model]) =>
        draftIds.includes(id) && (!changed || isImageModelInFamily(model, family)),
      ));
      return JSON.stringify(next) === JSON.stringify(models) ? models : next;
    });
  }, [imageGenerationModel, draftIds, registry.ready]);

  const planCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const account of candidates) {
      const plan = normalizePlanType(account) || "other";
      counts.set(plan, (counts.get(plan) ?? 0) + 1);
    }
    return counts;
  }, [candidates]);

  const planOptions = useMemo(
    () =>
      Array.from(planCounts.keys())
        .sort()
        .map((plan) => ({ value: plan, label: plan.toUpperCase() })),
    [planCounts],
  );

  const filtered = useMemo(() => {
    const keyword = query.trim().toLowerCase();
    const list = candidates.filter((account) => {
      if (planFilters.length > 0) {
        const plan = normalizePlanType(account) || "other";
        if (!planFilters.includes(plan)) return false;
      }
      if (!keyword) return true;
      return [buildCodexAccountPresentation(account, t).displayName, account.api_base_url, account.api_provider_name, account.email, account.account_name, account.id, account.plan_type]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(keyword));
    });
    const direction = sortDesc ? -1 : 1;
    return [...list].sort((left, right) => {
      switch (sortBy) {
        case "created_at":
          return ((left.created_at ?? 0) - (right.created_at ?? 0)) * direction;
        case "account":
          return (
            buildCodexAccountPresentation(left, t).displayName.localeCompare(buildCodexAccountPresentation(right, t).displayName) *
            direction
          );
        case "plan":
          return (
            normalizePlanType(left).localeCompare(normalizePlanType(right)) *
            direction
          );
        default:
          return ((left.last_used ?? 0) - (right.last_used ?? 0)) * direction;
      }
    });
  }, [candidates, planFilters, query, sortBy, sortDesc, t]);

  const pagination = usePagination({
    items: filtered,
    storageKey: "codex-image-account-picker-page-size",
    pageSizeOptions: PAGE_SIZE_OPTIONS,
    defaultPageSize: PAGE_SIZE_OPTIONS[0],
  });

  const toggleAccount = (accountId: string) => {
    onClearError?.();
    setDraftIds((prev) =>
      prev.includes(accountId)
        ? prev.filter((value) => value !== accountId)
        : [...prev, accountId],
    );
  };

  const effectiveIds = eligibleImageAccountIds(draftIds, accounts, imageGenerationModel, registry.deleted);
  const waitingForModel = !imageGenerationModel || !registry.ready;
  const missingModelIds = effectiveIds.filter((id) => {
    const account = accounts.find((item) => item.id === id)!;
    return isCodexApiKeyAccount(account) && !resolveImageAccountModel(account, imageGenerationModel, draftModels, registry.deleted);
  });
  const missingSpecificModel = missingModelIds.length > 0;
  const locateMissingModel = () => {
    if (saving || !missingModelIds.length) return;
    if (!filtered.some((account) => account.id === missingModelIds[0])) {
      setQuery("");
      setPlanFilters([]);
    }
    setLocateAccountId(missingModelIds[0]);
  };
  useEffect(() => {
    if (!locateAccountId || saving) return;
    const index = filtered.findIndex((account) => account.id === locateAccountId);
    if (index < 0) { setLocateAccountId(null); return; }
    const page = Math.floor(index / pagination.pageSize) + 1;
    if (pagination.currentPage !== page) {
      pagination.setCurrentPage(page);
      return;
    }
    const row = Array.from(dialog.current?.querySelectorAll<HTMLElement>("[data-image-source-id]") ?? [])
      .find((element) => element.dataset.imageSourceId === locateAccountId);
    const trigger = row?.querySelector<HTMLButtonElement>(".single-select-dropdown-trigger");
    if (!trigger) return;
    row?.scrollIntoView({ block: "nearest" });
    trigger.focus({ preventScroll: true });
    setLocateAccountId(null);
  }, [locateAccountId, filtered, pagination.currentPage, pagination.pageSize, pagination.pageItems, pagination.setCurrentPage, saving]);
  const selectedVisible = filtered.filter((account) =>
    draftIds.includes(account.id),
  );
  const allVisibleSelected =
    filtered.length > 0 && selectedVisible.length === filtered.length;

  return (
    <div className="modal-overlay" role="presentation">
      <div
        ref={dialog} tabIndex={-1}
        className="modal-content codex-add-modal codex-oauth-binding-modal codex-image-account-picker"
        role="dialog"
        aria-modal="true"
        aria-labelledby="codex-image-account-picker-title"
      >
        <div className="modal-header">
          <h2 id="codex-image-account-picker-title">
            {t("codex.deepSeek.start.pickerTitle", "选择生图账号")}
          </h2>
          <button
            type="button"
            className="modal-close"
            onClick={onCancel}
            disabled={saving}
            aria-label={t("common.close", "关闭")}
          >
            <X />
          </button>
        </div>
        <div className="modal-body">
          <ModalErrorMessage message={errorMessage} />
          {!registry.ready && !registry.syncError && <p role="status">{t("common.loading")}</p>}
          {!imageModelControl && registry.syncError && <>
            <ModalErrorMessage message={registry.syncError.startsWith("codex.localAccess.") ? t(registry.syncError) : registry.syncError} />
            <button type="button" className="btn btn-secondary btn-sm" disabled={saving} onClick={registry.retry}>{t("common.retry")}</button>
          </>}
          <div className="add-section">
            <div className="codex-oauth-binding-context">
              <p className="section-desc codex-oauth-binding-desc">
                {t(
                  "codex.launchPreview.imageGenPickerDesc",
                  "当遇到生图需求时会转发生图请求：OAuth 来源使用基础模型和生图工具；API Key 来源使用所选图片模型和图片接口，费用或额度由对应上游计入。",
                )}
              </p>
              {contextLabel?.trim() && (
                <div className="section-desc codex-oauth-binding-current-target">
                  {t("codex.modelProviders.oauthBinding.currentProvider", {
                    defaultValue: "供应商：{{name}}",
                    name: contextLabel.trim(),
                  })}
                </div>
              )}
            </div>

            <div className="codex-oauth-binding-picker">
              {imageModelControl && (
                <div className="codex-image-account-picker-models">
                  {imageModelControl}
                </div>
              )}
              <div className="codex-oauth-binding-picker-header">
                <label>{t("codex.deepSeek.start.pickerTitle", "选择生图账号")}</label>
                <div className="codex-oauth-binding-picker-controls">
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    disabled={saving || filtered.length === 0}
                    onClick={() => {
                      onClearError?.();
                      if (allVisibleSelected) {
                        const visibleIds = new Set(filtered.map((item) => item.id));
                        setDraftIds((prev) =>
                          prev.filter((value) => !visibleIds.has(value)),
                        );
                        return;
                      }
                      const ids = new Set(draftIds);
                      filtered.forEach((account) => ids.add(account.id));
                      setDraftIds(Array.from(ids));
                    }}
                  >
                    {allVisibleSelected
                      ? t("codex.deepSeek.start.pickerClear", "清空")
                      : t("codex.deepSeek.start.pickerSelectAll", "全选")}
                  </button>
                </div>
              </div>

              {candidates.length === 0 ? (
                <div className="add-status error">
                  <span>
                    {t(
                      "codex.deepSeek.start.pickerEmpty",
                      "没有可用的生图来源。API Key 账号须包含上方所选完整型号；OAuth 仅支持 gpt-image-2 / gpt-image-2.5 的非 Free 账号。",
                    )}
                  </span>
                </div>
              ) : (
                <>
                  <div className="codex-oauth-binding-toolbar">
                    <div className="search-box codex-oauth-binding-search">
                      <Search size={16} className="search-icon" />
                      <input
                        type="text"
                        placeholder={t("common.shared.search", "搜索账号...")}
                        value={query}
                        onChange={(event) => setQuery(event.target.value)}
                        disabled={saving}
                      />
                    </div>
                    <MultiSelectFilterDropdown
                      options={planOptions}
                      selectedValues={planFilters}
                      allLabel={t("common.shared.filter.all", {
                        count: candidates.length,
                      })}
                      filterLabel={t("common.shared.filterLabel", "筛选")}
                      clearLabel={t("accounts.clearFilter", "清空筛选")}
                      emptyLabel={t("common.none", "暂无")}
                      ariaLabel={t("common.shared.filterLabel", "筛选")}
                      onToggleValue={(value) =>
                        setPlanFilters((prev) =>
                          prev.includes(value)
                            ? prev.filter((item) => item !== value)
                            : [...prev, value],
                        )
                      }
                      onClear={() => setPlanFilters([])}
                    />
                    <SingleSelectFilterDropdown
                      value={sortBy}
                      options={[
                        {
                          value: "last_used",
                          label: t("accounts.columns.lastUsed", "最后使用"),
                        },
                        {
                          value: "created_at",
                          label: t("common.shared.sort.createdAt", "按创建时间"),
                        },
                        {
                          value: "account",
                          label: t("common.shared.columns.account", "账号"),
                        },
                        {
                          value: "plan",
                          label: t("accounts.sort.plan", "按套餐"),
                        },
                      ]}
                      ariaLabel={t("common.shared.sortLabel", "排序")}
                      icon={<ArrowDownWideNarrow size={14} />}
                      disabled={saving}
                      onChange={(value) => setSortBy(value as SortField)}
                    />
                    <button
                      type="button"
                      className="sort-direction-btn"
                      onClick={() => setSortDesc((prev) => !prev)}
                      disabled={saving}
                      title={
                        sortDesc
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
                      {sortDesc ? <ArrowDown size={15} /> : <ArrowUp size={15} />}
                    </button>
                  </div>

                  {filtered.length === 0 ? (
                    <div className="group-account-empty">
                      <span>
                        {t("common.shared.noMatch.title", "没有匹配的账号")}
                      </span>
                    </div>
                  ) : (
                    <div className="codex-oauth-binding-list">
                      {pagination.pageItems.map((account) => {
                        const presentation = buildCodexAccountPresentation(
                          account,
                          t,
                        );
                        const subscriptionInfo = getCodexSubscriptionPresentation(
                          account.subscription_active_until,
                          t,
                        );
                        const selected = draftIds.includes(account.id);
                        const emailText = maskSensitiveValue(
                          presentation.displayName || account.id,
                          isPrivacyModeEnabledByDefault(),
                        );
                        return (
                          <div
                            key={account.id}
                            data-image-source-id={account.id}
                            className={`codex-oauth-binding-row codex-image-account-row ${
                              selected ? "is-selected" : ""
                            } ${isCodexApiKeyAccount(account) ? "is-api-source" : "is-oauth-source"}`}
                            aria-label={emailText}
                            aria-disabled={saving}
                          >
                            <label className="codex-image-account-choice">
                              <input
                                type="checkbox"
                                checked={selected}
                                onChange={() => toggleAccount(account.id)}
                                disabled={saving}
                              />
                              <div className="codex-oauth-binding-row-main">
                                <span
                                  className="codex-oauth-binding-row-name"
                                  title={emailText}
                                >
                                  {emailText}
                                </span>
                                <span
                                  className={`tier-badge codex-oauth-binding-row-plan ${
                                    presentation.planClass || "unknown"
                                  }`}
                                  title={presentation.planLabel}
                                >
                                  {presentation.planLabel}
                                </span>
                                {!isCodexApiKeyAccount(account) && <span
                                  className={`codex-oauth-binding-row-term ${subscriptionInfo.tone}`}
                                  title={subscriptionInfo.titleText}
                                >
                                  <Clock size={12} />
                                  <span>
                                    {t("codex.subscription.label", "有效期")}
                                  </span>
                                  <strong>{subscriptionInfo.valueText}</strong>
                                  <span>{subscriptionInfo.detailText}</span>
                                </span>}
                              </div>
                            </label>
                            {isCodexApiKeyAccount(account) && (
                              <div className="codex-image-account-model">
                                <SingleSelectDropdown
                                  value={resolveImageAccountModel(account, imageGenerationModel, draftModels, registry.deleted) || ""}
                                  options={imageModelsForAccount(account, imageGenerationModel, registry.deleted).map((model) => ({ value: model, label: model }))}
                                  placeholder={t("codex.localAccess.imageAccounts.chooseModel")}
                                  ariaLabel={`${emailText}: ${t("codex.localAccess.imageAccounts.upstreamModel")}`}
                                  className="codex-image-account-model-select" menuClassName="codex-image-account-model-menu" menuWidth={290} menuMaxHeight={200}
                                  disabled={saving || !selected || !registry.ready}
                                  onChange={(model) => { if (saving || !selected || !registry.ready) return; onClearError?.(); setDraftModels((models) => ({ ...models, [account.id]: model })); }}
                                />
                                {selected && registry.ready && !resolveImageAccountModel(account, imageGenerationModel, draftModels, registry.deleted) && (
                                  <small className="codex-image-account-model-error" role="alert">{t("codex.localAccess.imageAccounts.modelRequired")}</small>
                                )}
                              </div>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  )}
                  <PaginationControls
                    totalItems={pagination.totalItems}
                    currentPage={pagination.currentPage}
                    totalPages={pagination.totalPages}
                    pageSize={pagination.pageSize}
                    pageSizeOptions={pagination.pageSizeOptions}
                    rangeStart={pagination.rangeStart}
                    rangeEnd={pagination.rangeEnd}
                    canGoPrevious={pagination.canGoPrevious}
                    canGoNext={pagination.canGoNext}
                    onPageSizeChange={pagination.setPageSize}
                    onPreviousPage={pagination.goToPreviousPage}
                    onNextPage={pagination.goToNextPage}
                  />
                </>
              )}
            </div>

          </div>
        </div>

        {/* 操作区固定在底部：正文再长也能直接取消 / 保存。 */}
        <div className="api-key-edit-actions codex-image-account-picker-actions">
          {!waitingForModel && missingSpecificModel && <div className="codex-image-account-picker-model-hint">
            <span role="alert">{t("codex.localAccess.imageAccounts.missingModels", { count: missingModelIds.length })}</span>
            <button type="button" className="btn btn-secondary btn-sm" disabled={saving} onClick={locateMissingModel}>
              {t("codex.localAccess.imageAccounts.locateModel")}
            </button>
          </div>}
          <span className="codex-image-account-picker-count">
            {t("codex.deepSeek.start.imageGenSelected", {
              count: effectiveIds.length,
              defaultValue: "已选 {{count}} 个账号",
            })}
          </span>
          <button
            className="btn btn-secondary"
            onClick={onCancel}
            disabled={saving}
          >
            {t("common.cancel", "取消")}
          </button>
          <button
            className="btn btn-primary"
            onClick={() => { if (!saving && !waitingForModel && !missingSpecificModel) onConfirm(effectiveIds, selectedImageAccountModels(effectiveIds, accounts, imageGenerationModel, draftModels, registry.deleted)); }}
            disabled={saving || waitingForModel || missingSpecificModel}
          >
            {saving ? t("common.saving", "保存中...") : t("common.save", "保存")}
          </button>
        </div>
      </div>
    </div>
  );
}
