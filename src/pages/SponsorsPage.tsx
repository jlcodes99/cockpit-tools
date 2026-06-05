import { useCallback, useEffect, useState } from 'react';
import { ExternalLink, HeartHandshake, RefreshCw } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import { useSponsorStore } from '../stores/useSponsorStore';
import type { Sponsor } from '../types/sponsor';
import './SponsorsPage.css';

function isSafeHttpUrl(value?: string | null): value is string {
  return Boolean(value && /^https?:\/\//i.test(value.trim()));
}

function SponsorLogo({ sponsor }: { sponsor: Sponsor }) {
  const [failed, setFailed] = useState(false);
  const initial = sponsor.name.trim().slice(0, 1).toUpperCase() || 'S';

  if (!isSafeHttpUrl(sponsor.logoUrl) || failed) {
    return <div className="sponsor-logo-fallback">{initial}</div>;
  }

  return (
    <img
      className="sponsor-logo-image"
      src={sponsor.logoUrl}
      alt={sponsor.name}
      loading="lazy"
      onError={() => setFailed(true)}
    />
  );
}

export function SponsorsPage() {
  const { t } = useTranslation();
  const sponsorModule = useSponsorStore((state) => state.state.sponsorModule);
  const loading = useSponsorStore((state) => state.loading);
  const initialized = useSponsorStore((state) => state.initialized);
  const fetchState = useSponsorStore((state) => state.fetchState);

  useEffect(() => {
    void fetchState();
  }, [fetchState]);

  useEffect(() => {
    const handleLanguageChanged = () => {
      void fetchState();
    };
    window.addEventListener('general-language-updated', handleLanguageChanged);
    return () => {
      window.removeEventListener('general-language-updated', handleLanguageChanged);
    };
  }, [fetchState]);

  const openSponsor = useCallback(async (url?: string | null) => {
    const target = url?.trim();
    if (!target || !isSafeHttpUrl(target)) {
      return;
    }
    try {
      await openUrl(target);
    } catch {
      window.open(target, '_blank', 'noopener,noreferrer');
    }
  }, []);

  const sponsors = sponsorModule?.sponsors ?? [];
  const title = sponsorModule?.title?.trim() || t('sponsors.pageTitle', '赞助商');
  const subtitle = sponsorModule?.subtitle?.trim() || t('sponsors.pageDesc', '感谢以下赞助商支持 Cockpit Tools。');
  const showEmpty = initialized && (!sponsorModule || sponsors.length === 0);

  return (
    <main className="main-content sponsors-page">
      <div className="sponsors-hero">
        <div className="sponsors-title-block">
          <div className="sponsors-title-icon">
            <HeartHandshake size={22} />
          </div>
          <div>
            <h1>{title}</h1>
            <p>{subtitle}</p>
          </div>
        </div>
      </div>

      {loading && !initialized ? (
        <div className="sponsors-status-card">{t('sponsors.loading', '正在加载赞助商...')}</div>
      ) : null}

      {showEmpty ? (
        <div className="sponsors-status-card">
          <HeartHandshake size={24} />
          <div>
            <strong>{t('sponsors.emptyTitle', '暂无赞助商')}</strong>
            <p>{t('sponsors.emptyDesc', '当前远端未开启赞助商模块。')}</p>
          </div>
        </div>
      ) : null}

      {sponsors.length > 0 ? (
        <section className="sponsors-panel" aria-label={t('sponsors.listAriaLabel', '赞助商列表')}>
          <div className="sponsors-panel-header">
            <div className="sponsors-panel-title">
              <HeartHandshake size={17} />
              <span>{t('sponsors.listAriaLabel', '赞助商列表')}</span>
            </div>
            <div className="sponsors-panel-actions">
              <span className="sponsors-panel-count">{sponsors.length}</span>
              <button
                type="button"
                className="sponsors-refresh-btn"
                onClick={() => void fetchState(true)}
                disabled={loading}
                title={t('common.refresh', '刷新')}
                aria-label={t('common.refresh', '刷新')}
              >
                <RefreshCw size={14} className={loading ? 'spin' : undefined} />
                <span>{t('common.refresh', '刷新')}</span>
              </button>
            </div>
          </div>
          <div className="sponsors-grid">
            {sponsors.map((sponsor) => {
              const clickable = isSafeHttpUrl(sponsor.url);
              const content = (
                <>
                  <div className="sponsor-card-top">
                    <div className="sponsor-logo">
                      <SponsorLogo sponsor={sponsor} />
                    </div>
                    {sponsor.badge ? <span className="sponsor-badge">{sponsor.badge}</span> : null}
                  </div>
                  <div className="sponsor-card-body">
                    <h2>{sponsor.name}</h2>
                    <p>{sponsor.description || t('sponsors.defaultDescription', '感谢支持 Cockpit Tools。')}</p>
                  </div>
                  {clickable ? (
                    <span className="sponsor-card-link">
                      {t('sponsors.openSponsor', '查看')}
                      <ExternalLink size={14} />
                    </span>
                  ) : null}
                </>
              );

              if (!clickable) {
                return (
                  <article className="sponsor-card" key={sponsor.id}>
                    {content}
                  </article>
                );
              }

              return (
                <button
                  type="button"
                  className="sponsor-card sponsor-card-button"
                  key={sponsor.id}
                  onClick={() => void openSponsor(sponsor.url)}
                >
                  {content}
                </button>
              );
            })}
          </div>
        </section>
      ) : null}
    </main>
  );
}
