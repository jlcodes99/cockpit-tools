import { useCallback, useEffect, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import { useTopRightAdStore } from '../stores/useTopRightAdStore';

interface TopCenterPromoBannerProps {
  reserveWhenEmpty?: boolean;
}

const PROMO_ROTATION_INTERVAL_MS = 6000;

export function TopCenterPromoBanner({ reserveWhenEmpty = true }: TopCenterPromoBannerProps) {
  const { t } = useTranslation();
  const ads = useTopRightAdStore((state) => state.state.ads);
  const [activeIndex, setActiveIndex] = useState(0);
  const [paused, setPaused] = useState(false);

  const ad = ads[activeIndex] ?? ads[0] ?? null;
  const hasCarousel = ads.length > 1;

  useEffect(() => {
    setActiveIndex(0);
  }, [ads]);

  useEffect(() => {
    if (!hasCarousel || paused) {
      return;
    }

    const timer = window.setInterval(() => {
      setActiveIndex((current) => (current + 1) % ads.length);
    }, PROMO_ROTATION_INTERVAL_MS);

    return () => window.clearInterval(timer);
  }, [ads.length, hasCarousel, paused]);

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
      aria-label={t('common.topRightAd.ariaLabel', '全局右上角广告位')}
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
    >
      <div className="global-promo-slot">
        <span className="global-ad-slot-badge">
          {ad.badge || t('common.topRightAd.badge', '广告')}
        </span>
        <div className="global-promo-main">
          <p className="global-promo-text">{ad.text}</p>
        </div>
        {ad.ctaUrl ? (
          <button className="global-ad-slot-action" onClick={handleClick}>
            {ad.ctaLabel || t('common.topRightAd.action', '查看详情')}
          </button>
        ) : null}
      </div>
    </div>
  );
}
