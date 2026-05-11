import { useCallback } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import { useTopRightAdStore } from '../stores/useTopRightAdStore';

interface TopCenterPromoBannerProps {
  reserveWhenEmpty?: boolean;
}

export function TopCenterPromoBanner({ reserveWhenEmpty = true }: TopCenterPromoBannerProps) {
  const { t } = useTranslation();
  const ad = useTopRightAdStore((state) => state.state.ad);

  const handleClick = useCallback(async () => {
    const target = ad?.ctaUrl?.trim();
    if (!target || !/^https?:\/\//i.test(target)) {
      return;
    }
    try {
      await openUrl(target);
    } catch {
      window.open(target, '_blank', 'noopener,noreferrer');
    }
  }, [ad?.ctaUrl]);

  if (!ad) {
    return reserveWhenEmpty ? <div className="global-promo-center global-promo-center-placeholder" aria-hidden="true" /> : null;
  }

  return (
    <div
      className="global-promo-center"
      role="complementary"
      aria-label={t('common.topRightAd.ariaLabel', "Global top-right ad slot")}
    >
      <div className="global-promo-slot">
        <span className="global-ad-slot-badge">
          {ad.badge || t('common.topRightAd.badge', "AD")}
        </span>
        <div className="global-promo-main">
          <p className="global-promo-text">{ad.text}</p>
        </div>
        {ad.ctaUrl ? (
          <button className="global-ad-slot-action" onClick={handleClick}>
            {ad.ctaLabel || t('common.topRightAd.action', "View details")}
          </button>
        ) : null}
      </div>
    </div>
  );
}
