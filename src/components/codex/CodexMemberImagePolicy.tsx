import { useTranslation } from 'react-i18next';
import { isCodexChatgptPlanAccount, type CodexAccount } from '../../types/codex';
import type { CodexLocalAccessImageGenerationPolicy } from '../../types/codexLocalAccess';
import { resolveCodexMemberImagePolicy } from '../../utils/codexLocalAccessAccounts';
import { SingleSelectDropdown } from '../SingleSelectDropdown';

export function CodexMemberImagePolicy({ account, value, disabled, official = false, onChange }: {
  account: CodexAccount;
  value?: CodexLocalAccessImageGenerationPolicy;
  disabled: boolean;
  official?: boolean;
  onChange: (value: CodexLocalAccessImageGenerationPolicy) => void;
}) {
  const { t } = useTranslation();
  const locked = official || isCodexChatgptPlanAccount(account);
  const options = locked
    ? [{ value: 'disabled', label: t('chatgptPlan.imagePolicyLocked') }]
    : [
        { value: 'inherit', label: t('codex.localAccess.imagePolicy.inherit') },
        { value: 'enabled', label: t('codex.localAccess.imagePolicy.enabled') },
        { value: 'disabled', label: t('codex.localAccess.imagePolicy.disabled') },
      ];
  return (
    <span title={locked ? t('chatgptPlan.imagePolicyHint') : undefined}>
      <SingleSelectDropdown
        value={resolveCodexMemberImagePolicy(account, value, official)}
        options={options}
        disabled={disabled || locked}
        className="codex-local-access-member-image-policy-dropdown"
        menuClassName="codex-local-access-member-image-policy-menu"
        menuWidth={120}
        ariaLabel={locked ? t('chatgptPlan.imagePolicyHint') : t('codex.localAccess.imagePolicy.label')}
        onChange={(next) => {
          if (disabled || locked) return;
          if (next === 'inherit' || next === 'enabled' || next === 'disabled') onChange(next);
        }}
      />
    </span>
  );
}
