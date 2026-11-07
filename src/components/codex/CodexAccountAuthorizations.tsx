import { ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { CodexAccount } from '../../types/codex';
import type { ChatGptPlanAccount } from '../../services/chatgptPlanService';
import { linkedChatGptPlanAccount, chatGptPlanOverviewId } from '../../utils/chatgptPlanOverview';
import { canChooseCodexServiceAuthorization, requestCodexServiceAuthorization } from '../../utils/codexAccountServiceNavigation';
import './CodexAccountAuthorizations.css';

export function CodexAccountAuthorizations({ account, registrations, memberIds }: {
  account: CodexAccount; registrations: readonly ChatGptPlanAccount[]; memberIds: ReadonlySet<string>;
}) {
  const { t } = useTranslation();
  if (!canChooseCodexServiceAuthorization(account)) return null;
  const grant = linkedChatGptPlanAccount(registrations, account.id);
  const official = Boolean(grant && memberIds.has(chatGptPlanOverviewId(grant.id)));
  const legacy = memberIds.has(account.id);
  const state = !grant ? 'unlinked' : !grant.connected || grant.needs_reauth ? 'reauthorize' : 'connected';
  return <div className="codex-account-authorizations">
    <span className="codex-login-subline">{t('chatgptPlan.serviceMode.codex')} · {t(account.requires_reauth ? 'chatgptPlan.status.reauthorize' : 'chatgptPlan.card.codexReady')}</span>
    <button type="button" className="group-account-badge" onClick={(event) => {
      event.stopPropagation(); requestCodexServiceAuthorization(account.id, 'official');
    }} title={t('chatgptPlan.card.manageBoth')}>
      <ShieldCheck size={12} />{t('chatgptPlan.serviceMode.official')} · {t(`chatgptPlan.card.${state}`)}
    </button>
    {(legacy || official) && <button type="button" className="group-account-badge" onClick={(event) => {
      event.stopPropagation(); requestCodexServiceAuthorization(account.id);
    }}>{t('chatgptPlan.card.serviceRoute', { mode: t(official ? 'chatgptPlan.officialFlow.rowOfficial' : 'chatgptPlan.officialFlow.rowLocal') })}</button>}
    {official && <span className="codex-login-subline">{t('chatgptPlan.imagePolicyLocked')}</span>}
    {grant && <span className="codex-login-subline" title={t('chatgptPlan.card.quotaHint')}>{t('chatgptPlan.card.quotaSource')}</span>}
  </div>;
}
