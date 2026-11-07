import { ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { requestChatgptPlanAuthorization } from '../../utils/chatgptPlanNavigation';

export function ChatGptPlanShortcut({ email, className = 'card-action-btn' }: { email: string; className?: string }) {
  const { t } = useTranslation();
  return (
    <button type="button" className={className}
      title={t('chatgptPlan.title')} aria-label={t('chatgptPlan.title')}
      onClick={() => requestChatgptPlanAuthorization(email)}>
      <ShieldCheck size={14} />
    </button>
  );
}
