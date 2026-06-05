import { useCallback } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';

interface TopCenterPromoBannerProps {
  reserveWhenEmpty?: boolean;
}

const FORK_AUTHOR_URL = 'https://github.com/malikdoksoz';

export function TopCenterPromoBanner(_props: TopCenterPromoBannerProps) {
  const openForkAuthor = useCallback(async () => {
    try {
      await openUrl(FORK_AUTHOR_URL);
    } catch {
      window.open(FORK_AUTHOR_URL, '_blank', 'noopener,noreferrer');
    }
  }, []);

  return (
    <div
      className="global-promo-center"
      role="complementary"
      aria-label="Fork notice"
    >
      <div className="global-promo-slot">
        <div className="global-promo-main">
          <p className="global-promo-text">
            This repository is a fork and continues to be developed by{' '}
            <button
              type="button"
              className="global-promo-author-link"
              onClick={openForkAuthor}
            >
              Malik
            </button>
            .
          </p>
        </div>
      </div>
    </div>
  );
}
