import { ExternalLink, Gift } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useApiKeyFunCampaign } from '../../hooks/useApiKeyFunCampaign';
import { APIKEY_FUN_CAMPAIGN, isApiKeyFunCampaignActive } from '../../utils/apiKeyFunCampaign';
import { APIKEY_FUN_REGISTER_URL } from '../../utils/apikeyFunLinks';
import apiKeyFunIcon from '../../assets/icons/apikey-fun.png';
import './ApiKeyFunPromotionHeader.css';

export function ApiKeyFunPromotionHeader({ onOpen }: { onOpen: (url: string) => void }) {
  const { t } = useTranslation();
  const campaignActive = useApiKeyFunCampaign();

  return (
    <header className={`apikey-fun-header-brand${campaignActive ? ' is-campaign' : ''}`}>
      {campaignActive && (
        <span className="apikey-fun-campaign-watermark" aria-hidden="true">{t('apiKeyFun.campaign.badge')}</span>
      )}
      <div className="apikey-fun-brand-main">
        <img src={apiKeyFunIcon} alt="" className="apikey-fun-brand-logo" />
        <div className="apikey-fun-brand-text">
          <div className="apikey-fun-eyebrow-container">
            <span className="apikey-fun-eyebrow">
              {campaignActive && <Gift size={13} aria-hidden="true" />}
              {campaignActive ? t('apiKeyFun.campaign.label') : t('apiKeyFun.eyebrow')}
            </span>
          </div>
          <h1>{campaignActive ? <>APIKEY.FAN · {t('apiKeyFun.campaign.title')}</> : t('apiKeyFun.title')}</h1>
          <p>{campaignActive ? t('apiKeyFun.campaign.promotionDescription') : t('apiKeyFun.description')}</p>
          {campaignActive && (
            <p className="apikey-fun-campaign-details">
              <span>{t('apiKeyFun.campaign.period')}</span>
              <span>{t('apiKeyFun.campaign.terms')}</span>
            </p>
          )}
        </div>
      </div>
      <div className="apikey-fun-brand-actions">
        <button
          type="button"
          className="btn apikey-fun-register-btn"
          onClick={() => onOpen(isApiKeyFunCampaignActive() ? APIKEY_FUN_CAMPAIGN.url : APIKEY_FUN_REGISTER_URL)}
        >
          <ExternalLink size={15} aria-hidden="true" />
          <span>{campaignActive ? t('apiKeyFun.campaign.action') : t('apiKeyFun.viewNow')}</span>
        </button>
      </div>
    </header>
  );
}
