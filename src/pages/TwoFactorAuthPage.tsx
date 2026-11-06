import { useTranslation } from 'react-i18next';
import './TwoFactorAuthPage.css';
import { MfaVaultManager } from '../components/MfaVaultManager';

export function TwoFactorAuthPage() {
  const { t } = useTranslation();

  return (
    <main className="main-content two-factor-query-page" aria-label={t('twoFactorAuth.pageTitle')}>
      <MfaVaultManager />
    </main>
  );
}
