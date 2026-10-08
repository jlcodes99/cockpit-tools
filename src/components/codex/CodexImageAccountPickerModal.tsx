import {
  ArrowDown,
  ArrowDownWideNarrow,
  ArrowUp,
  Clock,
  Search,
  X,
} from "lucide-react";
import { useMemo, useState } from "react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { useEscCloseTopmost } from "../../hooks/useEscClose";
import { usePagination } from "../../hooks/usePagination";
import { MultiSelectFilterDropdown } from "../MultiSelectFilterDropdown";
import { PaginationControls } from "../PaginationControls";
import { SingleSelectFilterDropdown } from "../SingleSelectFilterDropdown";
import {
  getCodexSubscriptionPresentation,
  isCodexApiKeyAccount,
  isCodexAgentIdentityAccount,
  isCodexWebSessionAccount,
  type CodexAccount,
} from "../../types/codex";
import { buildCodexAccountPresentation } from "../../presentation/platformAccountPresentation";
import {
  isPrivacyModeEnabledByDefault,
  maskSensitiveValue,
} from "../../utils/privacy";
import "./CodexImageAccountPickerModal.css";

type SortField = "last_used" | "created_at" | "account" | "plan";

const PAGE_SIZE_OPTIONS = [10, 20, 50] as const;

export interface CodexImageAccountPickerModalProps {
  accounts: CodexAccount[];
  selectedIds: string[];
  /** 信息区展示的对话账号 / 供应商名称。 */
  contextLabel?: string;
  /** 生图模型等附加设置；传入后与账号选择合并到同一个弹框里。 */
  imageModelControl?: ReactNode;
  saving?: boolean;
  onCancel: () => void;
  onConfirm: (accountIds: string[]) => void;
}

function normalizePlanType(account: CodexAccount): string {
  return (account.plan_type || "").trim().toLowerCase();
}

export function isImageGenerationEligibleAccount(account: CodexAccount): boolean {
  if (!account) return false;
  if (isCodexApiKeyAccount(account)) return false;
  if (isCodexAgentIdentityAccount(account)) return false;
  if (isCodexWebSessionAccount(account)) return false;
  return !normalizePlanType(account).includes("free");
}

export function CodexImageAccountPickerModal({
  accounts,
  selectedIds,
  contextLabel,
  imageModelControl,
  saving = false,
  onCancel,
  onConfirm,
}: CodexImageAccountPickerModalProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [planFilters, setPlanFilters] = useState<string[]>([]);
  const [sortBy, setSortBy] = useState<SortField>("last_used");
  const [sortDesc, setSortDesc] = useState(true);
  const [draftIds, setDraftIds] = useState<string[]>(selectedIds);

  // 该弹框会叠加在启动预览 / API 服务弹框之上，ESC 只关闭最上层，避免连带关闭底层弹框。
  useEscCloseTopmost(!saving, onCancel);

  const candidates = useMemo(
    () => accounts.filter((account) => isImageGenerationEligibleAccount(account)),
    [accounts],
  );

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
      return [account.email, account.account_name, account.id, account.plan_type]
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
            String(left.email || "").localeCompare(String(right.email || "")) *
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
  }, [candidates, planFilters, query, sortBy, sortDesc]);

  const pagination = usePagination({
    items: filtered,
    storageKey: "codex-image-account-picker-page-size",
    pageSizeOptions: PAGE_SIZE_OPTIONS,
    defaultPageSize: PAGE_SIZE_OPTIONS[0],
  });

  const toggleAccount = (accountId: string) => {
    setDraftIds((prev) =>
      prev.includes(accountId)
        ? prev.filter((value) => value !== accountId)
        : [...prev, accountId],
    );
  };

  const selectedVisible = filtered.filter((account) =>
    draftIds.includes(account.id),
  );
  const allVisibleSelected =
    filtered.length > 0 && selectedVisible.length === filtered.length;

  return (
    <div className="modal-overlay" role="presentation">
      <div
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
          <div className="add-section">
            <div className="codex-oauth-binding-context">
              <p className="section-desc codex-oauth-binding-desc">
                {t(
                  "codex.launchPreview.imageGenPickerDesc",
                  "当遇到生图需求时会转发生图请求：先由基础模型处理，再由生图模型出图，会消耗所选账号的部分额度。",
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
              <div className="codex-oauth-binding-picker-header">
                <label>{t("codex.deepSeek.start.pickerTitle", "选择生图账号")}</label>
                <div className="codex-oauth-binding-picker-controls">
                  {imageModelControl}
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    disabled={saving || filtered.length === 0}
                    onClick={() => {
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
                      "没有可用于生图的 OAuth 账号（Free 账号与 API Key 账号不支持）。",
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
                          account.email ||
                            account.account_name ||
                            presentation.displayName ||
                            account.id,
                          isPrivacyModeEnabledByDefault(),
                        );
                        return (
                          <label
                            key={account.id}
                            className={`codex-oauth-binding-row codex-image-account-row ${
                              selected ? "is-selected" : ""
                            }`}
                            aria-label={emailText}
                            aria-disabled={saving}
                          >
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
                              <span
                                className={`codex-oauth-binding-row-term ${subscriptionInfo.tone}`}
                                title={subscriptionInfo.titleText}
                              >
                                <Clock size={12} />
                                <span>
                                  {t("codex.subscription.label", "有效期")}
                                </span>
                                <strong>{subscriptionInfo.valueText}</strong>
                                <span>{subscriptionInfo.detailText}</span>
                              </span>
                            </div>
                          </label>
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
          <span className="codex-image-account-picker-count">
            {t("codex.deepSeek.start.imageGenSelected", {
              count: draftIds.length,
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
            onClick={() => onConfirm(draftIds)}
            disabled={saving}
          >
            {saving ? t("common.saving", "保存中...") : t("common.save", "保存")}
          </button>
        </div>
      </div>
    </div>
  );
}
