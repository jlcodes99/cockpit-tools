import type { Account } from '../types/account';
import type { AccountFilterType } from '../utils/accountFilters';
import { VALID_ACCOUNTS_FILTER_VALUE } from '../utils/accountValidityFilter';
import { normalizeAccountsOverviewScope } from '../utils/accountsOverviewFilterPersistence';
import type { MfaRecord } from '../utils/mfaVault';
import type { MailVerificationCodePreview } from '../utils/mailVerificationCode';
import {
  persistUserMemoryList,
  readUserMemoryList,
  USER_MEMORY_LISTS,
} from '../utils/userMemory';

export type AccountsFilterType = AccountFilterType | typeof VALID_ACCOUNTS_FILTER_VALUE;
export type ViewMode = 'grid' | 'list' | 'compact';

export interface VerificationDetailRecord {
  status: string;
  lastMessage?: string | null;
  lastErrorCode?: number | null;
  validationUrl?: string | null;
  appealUrl?: string | null;
}

export interface VerificationHistoryRecord extends VerificationDetailRecord {
  accountId: string;
}

export interface VerificationHistoryBatch {
  batchId: string;
  verifiedAt: number;
  records?: VerificationHistoryRecord[];
}

/** 将验证历史归并为账号级最新状态，供总览快速渲染和错误详情展示。 */
export function buildVerificationHistoryMaps(batches: VerificationHistoryBatch[] = []) {
  const sorted = [...batches].sort((a, b) => b.verifiedAt - a.verifiedAt);
  const statusMap: Record<string, string> = {};
  const detailMap: Record<string, VerificationDetailRecord> = {};

  for (const batch of sorted) {
    for (const record of batch.records || []) {
      if (!(record.accountId in statusMap)) {
        statusMap[record.accountId] = record.status;
        detailMap[record.accountId] = {
          status: record.status,
          lastMessage: record.lastMessage,
          lastErrorCode: record.lastErrorCode,
          validationUrl: record.validationUrl,
          appealUrl: record.appealUrl,
        };
      }
    }
  }

  return { statusMap, detailMap };
}

export interface ExtensionImportProgressPayload {
  phase?: string;
  current?: number;
  total?: number;
  email?: string;
}

export const ANTIGRAVITY_TOKEN_SINGLE_EXAMPLE = `{"refresh_token":"1//0gAbCdEf..."}`;
export const ANTIGRAVITY_TOKEN_BATCH_EXAMPLE = `[
  {"refresh_token":"1//0gTokenA..."},
  {"refreshToken":"1//0gTokenB..."}
]`;
export const ANTIGRAVITY_FILTER_PERSISTENCE_SCOPE = normalizeAccountsOverviewScope('antigravity');
export const ANTIGRAVITY_FILTER_FIELD_VIEW_MODE = 'view_mode';
export const ANTIGRAVITY_FILTER_FIELD_SORT_BY = 'sort_by';
export const ANTIGRAVITY_FILTER_FIELD_SORT_DIRECTION = 'sort_direction';
export const ANTIGRAVITY_FILTER_FIELD_FILTER_TYPES = 'filter_types';
export const ANTIGRAVITY_FILTER_FIELD_TAG_FILTER = 'tag_filter';
export const ANTIGRAVITY_FILTER_FIELD_GROUP_BY_TAG = 'group_by_tag';
export const ANTIGRAVITY_FILTER_FIELD_ACTIVE_GROUP_ID = 'active_group_id';
export const DEFAULT_FILTER_TYPES: AccountsFilterType[] = [];
export const DEFAULT_TAG_FILTER: string[] = [];
export const ANTIGRAVITY_ACCOUNT_NOTE_MAX_LENGTH = 200;

const ANTIGRAVITY_CUSTOM_SORT_ACTIVE_KEY = 'agtools.antigravity.accounts.custom_sort_active.v1';

export type AntigravityAccountNoteFormState = {
  note: string;
  twoFactorSecret: string;
  accountPassword: string;
  phoneNumber: string;
  mailUrl: string;
};

export type AntigravityAccountNoteMailPreviewState = MailVerificationCodePreview & {
  fetchedAt: number;
  truncated: boolean;
  status: 'initial' | 'changed' | 'unchanged';
};

export const EMPTY_ANTIGRAVITY_ACCOUNT_NOTE_FORM: AntigravityAccountNoteFormState = {
  note: '',
  twoFactorSecret: '',
  accountPassword: '',
  phoneNumber: '',
  mailUrl: '',
};

/** 建立账号备注编辑草稿，避免 UI 直接修改账号缓存对象。 */
export function buildAntigravityAccountNoteForm(
  account?: Account | null,
): AntigravityAccountNoteFormState {
  return {
    note: account?.notes ?? '',
    twoFactorSecret: account?.two_factor_secret ?? '',
    accountPassword: account?.account_password ?? '',
    phoneNumber: account?.phone_number ?? '',
    mailUrl: account?.mail_url ?? '',
  };
}

export function hasAntigravityAccountNoteDetails(account?: Account | null): boolean {
  return Boolean(
    account?.notes?.trim()
      || account?.two_factor_secret?.trim()
      || account?.account_password?.trim()
      || account?.phone_number?.trim()
      || account?.mail_url?.trim(),
  );
}

export function hasAntigravityAccountNoteFormDetails(
  form: AntigravityAccountNoteFormState,
): boolean {
  return Boolean(
    form.note.trim()
      || form.twoFactorSecret.trim()
      || form.accountPassword.trim()
      || form.phoneNumber.trim()
      || form.mailUrl.trim(),
  );
}

/** 把页面草稿转换为后端更新命令使用的字段结构。 */
export function buildAntigravityAccountNoteUpdate(form: AntigravityAccountNoteFormState) {
  return {
    note: form.note,
    twoFactorSecret: form.twoFactorSecret,
    accountPassword: form.accountPassword,
    phoneNumber: form.phoneNumber,
    mailUrl: form.mailUrl,
  };
}

export function isPendingAntigravityAccount(account?: Account | null): boolean {
  return Boolean(account?.pending_oauth);
}

export function formatMfaRecordOption(record: MfaRecord, fallback: string): string {
  return record.accountName?.trim() || fallback;
}

export function getAntigravityAccountNoteTitle(account: Account, fallback: string): string {
  const values = [
    account.account_password?.trim(),
    account.two_factor_secret?.trim(),
    account.mail_url?.trim(),
    account.phone_number?.trim(),
    account.notes?.trim(),
  ].filter(Boolean);
  return values.length > 0 ? values.join(' · ') : fallback;
}

export function formatAntigravityMailPreviewTime(timestamp: number): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return '';
  return new Intl.DateTimeFormat(undefined, {
    hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false,
  }).format(date);
}

export function readAntigravityCustomSortOrder(): string[] {
  return readUserMemoryList(USER_MEMORY_LISTS.antigravityCustomSort);
}

export function writeAntigravityCustomSortOrder(accountIds: string[]): void {
  persistUserMemoryList(USER_MEMORY_LISTS.antigravityCustomSort, accountIds);
}

export function readAntigravityCustomSortActive(): boolean {
  try {
    return localStorage.getItem(ANTIGRAVITY_CUSTOM_SORT_ACTIVE_KEY) === '1';
  } catch {
    return false;
  }
}

export function writeAntigravityCustomSortActive(active: boolean): void {
  try {
    localStorage.setItem(ANTIGRAVITY_CUSTOM_SORT_ACTIVE_KEY, active ? '1' : '0');
  } catch {
    // 自定义排序只影响显示顺序，存储不可用时允许静默降级。
  }
}
