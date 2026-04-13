export const VALID_ACCOUNTS_FILTER_VALUE = '__valid_accounts__' as const;

type Translate = (key: string, options?: Record<string, unknown>) => string;

export function buildValidAccountsFilterOption(
  t: Translate,
  count: number,
): { value: typeof VALID_ACCOUNTS_FILTER_VALUE; label: string } {
  return {
    value: VALID_ACCOUNTS_FILTER_VALUE,
    label: t('common.shared.filter.validAccounts', {
      count,
      defaultValue: '有效账号 ({{count}})',
    }),
  };
}

export function splitValidityFilterValues(values: Iterable<string>): {
  requireValidAccounts: boolean;
  selectedTypes: Set<string>;
} {
  const selectedTypes = new Set<string>();
  let requireValidAccounts = false;
  for (const value of values) {
    if (value === VALID_ACCOUNTS_FILTER_VALUE) {
      requireValidAccounts = true;
      continue;
    }
    selectedTypes.add(value);
  }
  return { requireValidAccounts, selectedTypes };
}
