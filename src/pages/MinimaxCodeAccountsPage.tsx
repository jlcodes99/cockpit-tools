import { useTranslation } from 'react-i18next';

export function MinimaxCodeAccountsPage() {
  const { t } = useTranslation();
  return (
    <div className="ghcp-accounts-page minimax-code-accounts-page">
      <div className="minimax-code-placeholder">
        <h2>{t('minimaxCode.title', 'MiniMax Code')}</h2>
        <p>{t('minimaxCode.placeholder', 'MiniMax Code account management is coming soon.')}</p>
      </div>
    </div>
  );
}
