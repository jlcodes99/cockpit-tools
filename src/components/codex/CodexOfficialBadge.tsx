import { CircleAlert, Clock3, ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { isCodexChatgptPlanAccount, type CodexAccount } from '../../types/codex';
import type { ChatGptPlanAccount } from '../../services/chatgptPlanService';
import { chatGptPlanBindingStatus, chatGptPlanOverviewId, type ChatGptPlanBindingStatus } from '../../utils/chatgptPlanOverview';
import { codexOfficialAuthorizationState, codexOfficialAuthorizationStateKey, type CodexOfficialAuthorizationState } from '../../utils/codexOfficialAuthorization';
import './CodexOfficialBadge.css';

/** Highlight valid linked authorization independently of service membership. */
export function CodexOfficialBadge({ active, className = '', status, state }: { active: boolean; className?: string; status?: ChatGptPlanBindingStatus; state?: CodexOfficialAuthorizationState }) {
  const { t } = useTranslation();
  const resolved = state ?? (status === 'invalid' ? 'invalid' : status === 'pending' ? 'pending' : active ? 'used' : 'unused');
  const invalid = resolved === 'invalid' || resolved === 'reauthorize';
  const pending = resolved === 'pending';
  const authorized = resolved === 'used' || resolved === 'unused';
  const title = t(codexOfficialAuthorizationStateKey(resolved));
  return <span className={`codex-official-badge${invalid ? ' is-invalid' : pending ? ' is-pending' : authorized ? ' is-active' : ''}${className ? ` ${className}` : ''}`}
    title={title} aria-label={title}>
    {invalid ? <CircleAlert size={12} aria-hidden="true" /> : pending ? <Clock3 size={12} aria-hidden="true" /> : <ShieldCheck size={12} aria-hidden="true" />}
    <span>{t('chatgptPlan.officialFlow.badge')}</span>
  </span>;
}

export function CodexAccountOfficialBadge({ account, grants, memberIds }: {
  account: CodexAccount;
  grants: readonly ChatGptPlanAccount[];
  memberIds: ReadonlySet<string> | readonly string[];
}) {
  const linked = grants.filter((item) => item.source_account_id === account.id);
  const grant = isCodexChatgptPlanAccount(account)
    ? grants.find((item) => chatGptPlanOverviewId(item.id) === account.id)
    : linked.length === 1 ? linked[0] : undefined;
  if (!grant || (!grant.connected && !grant.needs_reauth)) return null;
  const ids = Array.isArray(memberIds) ? new Set(memberIds) : memberIds as ReadonlySet<string>;
  const active = (ids.has(chatGptPlanOverviewId(grant.id)) || Boolean(grant.in_service && grant.source_account_id && ids.has(grant.source_account_id))) && grant.connected && !grant.needs_reauth;
  return <CodexOfficialBadge active={active} status={chatGptPlanBindingStatus(grant)} state={codexOfficialAuthorizationState(grant, active)} />;
}
