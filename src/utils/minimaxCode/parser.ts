/**
 * MiniMax Code 配额数据归一化
 *
 * 后端把 `GET /minimax-cloud/api/v1/credit/details` 的原始响应体存放在账号的
 * `quota_raw.userResource` 上。这里把它包装成旧版 `Response.Data.Accounts`
 * 信封，从而复用共享的
 * `getOfficialQuotaModel` → `getQuotaCategoryGroups` → `CodeBuddyQuotaCategoryList`
 * 渲染管线，无需改动任何既有组件。
 */

import { getQuotaCategoryGroups } from '../codebuddy-suite';
import type { QuotaCategoryGroup } from '../../types/codebuddy-suite';
import type { MinimaxCodeAccount } from '../../types/minimaxCode';

interface MiniMaxCreditDetail {
  credit_type?: number;
  granted_amount?: string | number;
  consumed_amount?: string | number;
  remaining_amount?: string | number;
  granted_at_ms?: number;
  expire_at_ms?: number;
}

function toNumber(value: string | number | undefined): number {
  if (typeof value === 'number') return value;
  const n = Number((value ?? '').replace(/,/g, '').trim());
  return Number.isFinite(n) ? n : 0;
}

/**
 * The official client drops a detail row unless `credit_type` is one it knows
 * and every amount parses. Rendering those rows would show a bogus `0 / 0`
 * package, so mirror the same rule here.
 */
function isUsableMiniMaxCreditDetail(d: MiniMaxCreditDetail): boolean {
  if (d.credit_type !== 1 && d.credit_type !== 2) return false;
  if (d.remaining_amount === undefined && d.consumed_amount === undefined) return false;
  return [d.granted_amount, d.consumed_amount, d.remaining_amount].every(
    (value) => value === undefined || value === null || Number.isFinite(toNumber(value)),
  );
}

/** Map a MiniMax `credit/details` item onto the shared legacy resource shape. */
export function normalizeMiniMaxCreditDetail(d: MiniMaxCreditDetail): Record<string, unknown> {
  const granted = toNumber(d.granted_amount);
  const consumed = toNumber(d.consumed_amount);
  const remaining = toNumber(d.remaining_amount);
  const expireMs = d.expire_at_ms ?? 0;
  const expireEpoch = expireMs > 0 ? expireMs / 1000 : 0;
  const label = d.credit_type === 1 ? 'purchased' : 'free';
  return {
    Status: 0,
    PackageCode: label,
    PackageName: label,
    CycleCapacitySizePrecise: String(granted),
    CycleCapacityUsedPrecise: String(consumed),
    CycleCapacityRemainPrecise: String(remaining),
    DeductionEndTime: expireEpoch > 0 ? expireEpoch : null,
    ExpiredTime: expireEpoch > 0 ? new Date(expireMs).toISOString() : null,
  };
}

export function wrapMiniMaxCreditsAsResource(body: unknown): {
  code: number;
  msg: string;
  data: {
    Response: {
      Data: {
        Accounts: Record<string, unknown>[];
        TotalCount: number;
        TotalDosage: number;
      };
    };
  };
} {
  // `credit/details` returns `details` / `total_count` at the TOP LEVEL of the
  // response body — the official client reads `response.data.details` straight
  // off the axios response, and its interceptor returns the response ununwraped.
  // Reading `body.data.details` here silently produced zero quota for everyone.
  const root = (body ?? {}) as { details?: MiniMaxCreditDetail[]; data?: { details?: MiniMaxCreditDetail[] } };
  const details = Array.isArray(root.details) ? root.details : (root.data?.details ?? []);
  const usable = details.filter(isUsableMiniMaxCreditDetail);
  const accounts = usable.map(normalizeMiniMaxCreditDetail);
  const totalDosage = usable.reduce((sum, d) => sum + toNumber(d.consumed_amount), 0);
  return {
    code: 0,
    msg: 'OK',
    data: {
      Response: {
        Data: {
          Accounts: accounts,
          TotalCount: accounts.length,
          TotalDosage: totalDosage,
        },
      },
    },
  };
}

/**
 * Group MiniMax credits with the shared pipeline. The backend stores the raw
 * `credit/details` body at `quota_raw.userResource`; we wrap it into the legacy
 * `Response.Data.Accounts` envelope so `getOfficialQuotaModel` →
 * `getQuotaCategoryGroups` → `CodeBuddyQuotaCategoryList` render it unchanged.
 */
export function getMiniMaxCodeQuotaGroups(
  account: MinimaxCodeAccount,
  t: (key: string, defaultValue?: string) => string,
): QuotaCategoryGroup[] {
  const raw = (account.quota_raw as { userResource?: unknown } | null)?.userResource ?? account.quota_raw;
  const wrapped = wrapMiniMaxCreditsAsResource(raw);
  const modelAccount = { ...account, quota_raw: { userResource: wrapped } };
  return getQuotaCategoryGroups(modelAccount, t);
}

export function hasMiniMaxCodeQuotaData(
  account: MinimaxCodeAccount,
  groups: QuotaCategoryGroup[],
): boolean {
  void account;
  return groups.some((g) => g.items.length > 0);
}
