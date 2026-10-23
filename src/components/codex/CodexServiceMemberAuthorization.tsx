import type { CodexAccount } from '../../types/codex';
import type { ChatGptPlanAccount } from '../../services/chatgptPlanService';
import { CodexAccountOfficialBadge } from './CodexOfficialBadge';
import { useTranslation } from 'react-i18next';
import { codexServiceRelatedEndpointIds } from '../../utils/chatgptPlanOverview';

/** Service rows share the same passive badge as account cards; adding owns the authorization flow. */
export function CodexServiceMemberAuthorization({ account, accounts, grants, memberIds, pausedAccountIds = [] }: {
  account: CodexAccount;
  accounts: readonly CodexAccount[];
  grants: readonly ChatGptPlanAccount[];
  memberIds: readonly string[];
  pausedAccountIds?: readonly string[];
}) {
  const { t } = useTranslation();
  const paused = codexServiceRelatedEndpointIds(account, accounts).some((id) => memberIds.includes(id) && pausedAccountIds.includes(id));
  return <><CodexAccountOfficialBadge account={account} grants={grants} memberIds={memberIds} />
    {paused && <span className="codex-official-badge is-pending">{t('codex.localAccess.accountPoolHealth.paused')}</span>}</>;
}
