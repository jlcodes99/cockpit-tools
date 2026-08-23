import { Key } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import './TwoFactorAuthPage.css';
import { MfaVaultManager } from '../components/MfaVaultManager';
import { AccountConversionChallengePanel } from '../components/AccountConversionChallengePanel';

export function TwoFactorAuthPage() {
  const { t } = useTranslation();

  return (
    <main className="main-content ghcp-accounts-page two-factor-query-page">
      <div className="page-heading" style={{ padding: '24px 24px 0', display: 'flex', flexDirection: 'column', gap: '16px' }}>
        <h1 style={{ margin: 0, display: 'flex', alignItems: 'center', gap: '8px' }}>
          <Key size={20} /> {t('twoFactorAuth.pageTitle')}
        </h1>
        <div className="mfa-page-desc">
          {t('twoFactorAuth.pageDescNew')}
        </div>
      </div>

      <AccountConversionChallengePanel />
      <MfaVaultManager />
    </main>
  );
}
