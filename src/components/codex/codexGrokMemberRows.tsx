import {
  formatGrokQuotaUsedTotal,
  getGrokQuotaClass,
  type GrokAccount,
} from "../../types/grok";
import { getCodexPlanFilterKey, type CodexAccount } from "../../types/codex";
import {
  getCodexLocalAccessAccountIneligibleReason,
  isCodexLocalAccessEligibleAccount,
} from "../../utils/codexLocalAccessAccounts";

const GROK_MEMBER_ROW_ID_PREFIX = "grok:";

export function isGrokMemberRowId(accountId: string): boolean {
  return accountId.startsWith(GROK_MEMBER_ROW_ID_PREFIX);
}

/** 每次渲染合并最新的跨平台行，源账号已删除的代理行保留用于移除成员。 */
export function selectCodexLocalAccessMemberRows(input: {
  codexAccounts: CodexAccount[];
  grokRows: CodexAccount[];
  grokAccounts: GrokAccount[];
  manageGrok: boolean;
  showCodexPlatform: boolean;
  showGrokPlatform: boolean;
  restrictFreeAccounts: boolean;
  selected: ReadonlySet<string>;
}): CodexAccount[] {
  const sourceIds = new Set(input.grokAccounts.map((account) => account.id));
  return [...input.codexAccounts, ...input.grokRows].filter((account) => {
    const isGrokRow = isGrokMemberRowId(account.id);
    if (isGrokRow ? !input.showGrokPlatform : !input.showCodexPlatform) {
      return false;
    }
    const upstreamId = account.upstream_grok_account_id?.trim();
    if (input.manageGrok && !isGrokRow && upstreamId && sourceIds.has(upstreamId)) {
      return false;
    }
    const reason = getCodexLocalAccessAccountIneligibleReason(account, input.restrictFreeAccounts);
    return reason === "pending_oauth" || reason === "web_session_quota_only"
      || isCodexLocalAccessEligibleAccount(account, input.restrictFreeAccounts)
      || input.selected.has(account.id);
  });
}

/**
 * 把 Grok 平台账号映射成成员列表行。
 *
 * 行 ID 使用 `grok:` 前缀与真实 Codex 账号区分；真正的选中值始终是宿主创建的
 * 供应商账号 ID，因此这里只负责展示与筛选，不参与保存。
 */
export function buildGrokMemberRowAccounts(input: {
  accounts: GrokAccount[];
  searchQuery: string;
  tagFilterActive: boolean;
  groupFilterActive: boolean;
  requireValidAccounts: boolean;
  selectedTypes: Set<string>;
}): CodexAccount[] {
  // Grok 账号没有 Codex 标签/分组：这两类筛选生效时不参与匹配。
  if (input.tagFilterActive || input.groupFilterActive) {
    return [];
  }
  const queryText = input.searchQuery.trim().toLowerCase();
  return input.accounts
    .filter((account) => {
      if (queryText) {
        const haystack = `${account.email} ${account.plan_type ?? ""}`.toLowerCase();
        if (!haystack.includes(queryText)) {
          return false;
        }
      }
      if (
        input.requireValidAccounts &&
        account.status &&
        account.status !== "normal"
      ) {
        return false;
      }
      if (input.selectedTypes.size > 0) {
        const planKey = getCodexPlanFilterKey({
          plan_type: account.plan_type ?? account.quota?.subscriptionTier,
        } as CodexAccount);
        if (!input.selectedTypes.has(planKey)) {
          return false;
        }
      }
      return true;
    })
    .map(
      (account) =>
        ({
          id: `${GROK_MEMBER_ROW_ID_PREFIX}${account.id}`,
          email: account.email,
          auth_mode: "apikey",
          api_provider_id: "grok",
          api_provider_name: "Grok",
          plan_type: account.plan_type ?? account.quota?.subscriptionTier,
          api_model_catalog: [],
          upstream_grok_account_id: account.id,
          tokens: {
            id_token: "",
            access_token: "",
            refresh_token: null,
          },
          created_at: account.created_at,
          last_used: account.last_used,
        }) as unknown as CodexAccount,
    );
}

/** Grok 账号额度：只展示 GrokBuild 这一项。 */
export function CodexGrokBuildQuotaChip({
  t,
  account,
}: {
  t: (key: string, defaultValue?: string | any) => string;
  account: GrokAccount;
}) {
  const product = (account.quota?.products ?? []).find((item) =>
    /grok\s*build/i.test(item.product ?? ""),
  );
  if (!product) {
    return null;
  }
  const used = product.used ?? null;
  const total = product.total ?? null;
  const remaining = product.remaining ?? null;
  const usedPercent =
    product.usagePercent ??
    (used != null && total != null && total > 0
      ? (used / total) * 100
      : remaining != null && total != null && total > 0
        ? ((total - remaining) / total) * 100
        : null);
  if (usedPercent == null) {
    return null;
  }
  const clampedUsed = Math.max(0, Math.min(100, Math.round(usedPercent)));
  const remainingPercent = 100 - clampedUsed;
  const amountText = formatGrokQuotaUsedTotal(used, total);
  const title = [
    product.product,
    amountText || null,
    t("common.shared.quota.leftPercent", {
      defaultValue: "{{value}}% left",
      value: remainingPercent,
    }),
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <div className="codex-local-access-quota-line">
      <span
        className={`codex-local-access-quota-chip ${getGrokQuotaClass(clampedUsed)}`}
        title={title}
      >
        <span className="codex-local-access-quota-dot" />
        <span>
          {product.product} {remainingPercent}%
        </span>
      </span>
    </div>
  );
}
