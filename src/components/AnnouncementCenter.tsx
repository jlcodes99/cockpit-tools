import { ReactNode, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Bell, ChevronLeft, X } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import type { Page } from '../types/navigation';
import type { Announcement, AnnouncementAction } from '../types/announcement';
import { useAnnouncementStore } from '../stores/useAnnouncementStore';
import { useEscCloseTopmost } from '../hooks/useEscClose';
import { useModalFocusTrap } from '../hooks/useModalFocusTrap';
import './AnnouncementCenter.css';

interface AnnouncementCenterProps {
  onNavigate: (page: Page) => void;
  variant?: 'floating' | 'inline';
  trigger?: 'icon' | 'button';
}

interface AnnouncementSurfaceProps extends AnnouncementCenterProps {
  showTrigger: boolean;
  autoPopup: boolean;
  autoRefresh: boolean;
}

const ANNOUNCEMENT_BACKGROUND_REFRESH_INTERVAL_MS = 15 * 60 * 1000;

function hasBlockingOverlay(): boolean {
  if (typeof document === 'undefined') {
    return false;
  }
  return Boolean(
    document.querySelector(
      '.modal-overlay, .close-dialog-overlay, .qs-overlay, .settings-account-scope-dialog-overlay, [aria-modal="true"]',
    ),
  );
}

const TAB_TARGET_PAGE_MAP: Partial<Record<string, Page>> = {
  dashboard: 'dashboard',
  overview: 'overview',
  codex: 'codex',
  'github-copilot': 'github-copilot',
  windsurf: 'windsurf',
  kiro: 'kiro',
  wakeup: 'wakeup',
  instances: 'instances',
  settings: 'settings',
};

function isSafeUrl(url: string): boolean {
  if (!url) return false;
  return /^https?:\/\//i.test(url.trim());
}

function sanitizeTypeClass(type: string): string {
  return type.replace(/[^a-zA-Z0-9_-]/g, '');
}

function formatTimeAgo(
  value: string,
  translate: (key: string, fallback: string, options?: Record<string, unknown>) => string,
): string {
  const time = new Date(value).getTime();
  if (!Number.isFinite(time)) {
    return value;
  }

  const now = Date.now();
  const diffMs = Math.max(0, now - time);
  const diffMins = Math.floor(diffMs / 60_000);
  const diffHours = Math.floor(diffMs / 3_600_000);
  const diffDays = Math.floor(diffMs / 86_400_000);

  if (diffMins < 1) {
    return translate('announcement.timeAgo.justNow', '刚刚');
  }
  if (diffMins < 60) {
    const text = translate('announcement.timeAgo.minutesAgo', '{count}分钟前', { count: diffMins });
    return text.replace('{count}', String(diffMins)).replace('{{count}}', String(diffMins));
  }
  if (diffHours < 24) {
    const text = translate('announcement.timeAgo.hoursAgo', '{count}小时前', { count: diffHours });
    return text.replace('{count}', String(diffHours)).replace('{{count}}', String(diffHours));
  }
  const text = translate('announcement.timeAgo.daysAgo', '{count}天前', { count: diffDays });
  return text.replace('{count}', String(diffDays)).replace('{{count}}', String(diffDays));
}

function AnnouncementSurface({
  onNavigate,
  variant = 'floating',
  trigger = 'icon',
  showTrigger,
  autoPopup,
  autoRefresh,
}: AnnouncementSurfaceProps) {
  const { t } = useTranslation();
  const announcementState = useAnnouncementStore((state) => state.state);
  const loading = useAnnouncementStore((state) => state.loading);
  const fetchState = useAnnouncementStore((state) => state.fetchState);
  const markAsRead = useAnnouncementStore((state) => state.markAsRead);
  const markAllAsRead = useAnnouncementStore((state) => state.markAllAsRead);
  const translateText = (key: string, fallback: string, options?: Record<string, unknown>): string => {
    const raw = t(key, fallback, options) as unknown;
    return typeof raw === 'string' ? raw : fallback;
  };

  const [listOpen, setListOpen] = useState(false);
  const [detailAnnouncement, setDetailAnnouncement] = useState<Announcement | null>(null);
  const [detailFromList, setDetailFromList] = useState(false);
  const [imagePreviewUrl, setImagePreviewUrl] = useState<string | null>(null);
  const listDialog = useRef<HTMLDivElement>(null);
  const detailDialog = useRef<HTMLDivElement>(null);
  const imageDialog = useRef<HTMLDivElement>(null);
  useModalFocusTrap(listDialog, listOpen, true, !detailAnnouncement && !imagePreviewUrl);
  useModalFocusTrap(detailDialog, Boolean(detailAnnouncement), true, !imagePreviewUrl);
  useModalFocusTrap(imageDialog, Boolean(imagePreviewUrl));
  const [failedImages, setFailedImages] = useState<Set<string>>(new Set());
  const [handledPopupId, setHandledPopupId] = useState<string | null>(null);
  const [popupRetryRevision, setPopupRetryRevision] = useState(0);
  const refreshInFlightRef = useRef(false);
  const lastBackgroundRefreshAtRef = useRef(0);

  const refreshAnnouncements = useCallback(
    async (forceRefresh: boolean) => {
      if (refreshInFlightRef.current) {
        return;
      }
      refreshInFlightRef.current = true;
      try {
        await fetchState(forceRefresh);
      } finally {
        refreshInFlightRef.current = false;
      }
    },
    [fetchState],
  );

  useEffect(() => {
    if (!autoRefresh) {
      return;
    }

    lastBackgroundRefreshAtRef.current = Date.now();
    void refreshAnnouncements(false);

    const refreshIfDue = () => {
      if (document.visibilityState !== 'visible') {
        return;
      }
      const now = Date.now();
      if (
        now - lastBackgroundRefreshAtRef.current <
        ANNOUNCEMENT_BACKGROUND_REFRESH_INTERVAL_MS
      ) {
        return;
      }
      lastBackgroundRefreshAtRef.current = now;
      void refreshAnnouncements(true);
    };
    const handleLanguageChanged = () => {
      void refreshAnnouncements(false);
    };

    window.addEventListener('focus', refreshIfDue);
    window.addEventListener('general-language-updated', handleLanguageChanged);
    document.addEventListener('visibilitychange', refreshIfDue);
    const refreshTimer = window.setInterval(
      refreshIfDue,
      ANNOUNCEMENT_BACKGROUND_REFRESH_INTERVAL_MS,
    );

    return () => {
      window.removeEventListener('focus', refreshIfDue);
      window.removeEventListener('general-language-updated', handleLanguageChanged);
      document.removeEventListener('visibilitychange', refreshIfDue);
      window.clearInterval(refreshTimer);
    };
  }, [autoRefresh, refreshAnnouncements]);

  useEffect(() => {
    setFailedImages(new Set());
  }, [detailAnnouncement?.id]);

  useEffect(() => {
    if (!autoPopup) {
      return;
    }
    const popupAnnouncement = announcementState.popupAnnouncement;
    if (!popupAnnouncement) {
      return;
    }
    if (detailAnnouncement) {
      return;
    }
    if (handledPopupId === popupAnnouncement.id) {
      return;
    }

    if (hasBlockingOverlay()) {
      if (typeof MutationObserver === 'undefined' || !document.body) {
        return;
      }
      const observer = new MutationObserver(() => {
        if (!hasBlockingOverlay()) {
          observer.disconnect();
          setPopupRetryRevision((revision) => revision + 1);
        }
      });
      observer.observe(document.body, { childList: true, subtree: true });
      return () => observer.disconnect();
    }

    setListOpen(false);
    setDetailFromList(false);
    setDetailAnnouncement(popupAnnouncement);
    setHandledPopupId(popupAnnouncement.id);
  }, [
    announcementState.popupAnnouncement,
    autoPopup,
    detailAnnouncement,
    handledPopupId,
    popupRetryRevision,
  ]);

  const unreadCount = announcementState.unreadIds.length;

  const parseAnnouncementTime = (value: string): number => {
    const timestamp = new Date(value).getTime();
    return Number.isFinite(timestamp) ? timestamp : 0;
  };

  const sortedAnnouncements = useMemo(
    () =>
      [...announcementState.announcements].sort(
        (a, b) => parseAnnouncementTime(b.createdAt) - parseAnnouncementTime(a.createdAt),
      ),
    [announcementState.announcements],
  );

  const closeDetail = async (reopenList = false) => {
    if (detailAnnouncement && announcementState.unreadIds.includes(detailAnnouncement.id)) {
      await markAsRead(detailAnnouncement.id);
    }
    setDetailAnnouncement(null);
    setDetailFromList(false);
    if (reopenList) {
      setListOpen(true);
    }
  };

  useEscCloseTopmost(!!detailAnnouncement && !imagePreviewUrl, () => void closeDetail(false), detailDialog);
  useEscCloseTopmost(listOpen && !detailAnnouncement && !imagePreviewUrl, () => setListOpen(false), listDialog);

  useEscCloseTopmost(Boolean(imagePreviewUrl), () => setImagePreviewUrl(null), imageDialog);

  const handleAnnouncementClick = async (announcement: Announcement) => {
    if (announcementState.unreadIds.includes(announcement.id)) {
      await markAsRead(announcement.id);
    }
    setDetailAnnouncement(announcement);
    setDetailFromList(true);
    setListOpen(false);
  };

  const runAction = async (action: AnnouncementAction) => {
    if (action.type === 'tab') {
      const targetPage = TAB_TARGET_PAGE_MAP[action.target];
      if (targetPage) {
        onNavigate(targetPage);
      }
      await closeDetail(false);
      return;
    }

    if (action.type === 'url') {
      if (isSafeUrl(action.target)) {
        try {
          await openUrl(action.target);
        } catch {
          window.open(action.target, '_blank', 'noopener,noreferrer');
        }
      }
      await closeDetail(false);
      return;
    }

    if (action.type === 'command') {
      switch (action.target) {
        case 'update.check': {
          window.dispatchEvent(new CustomEvent('update-check-requested', { detail: { source: 'manual' } }));
          break;
        }
        case 'announcement.forceRefresh': {
          await fetchState(true);
          break;
        }
        case 'page.navigate': {
          const target = typeof action.arguments?.[0] === 'string' ? action.arguments[0] : '';
          const targetPage = target ? TAB_TARGET_PAGE_MAP[target] : undefined;
          if (targetPage) {
            onNavigate(targetPage);
          }
          break;
        }
        default:
          console.warn('[Announcement] 未支持的命令动作:', action.target);
      }
      await closeDetail(false);
    }
  };

  const currentTypeLabel = (type: string) => {
    if (type === 'feature') return t('announcement.type.feature', '✨ 新功能');
    if (type === 'warning') return t('announcement.type.warning', '⚠️ 警告');
    if (type === 'urgent') return t('announcement.type.urgent', '🚨 紧急');
    return t('announcement.type.info', 'ℹ️ 信息');
  };

  const renderInBody = (node: ReactNode) => {
    if (typeof document === 'undefined') {
      return node;
    }
    return createPortal(node, document.body);
  };

  return (
    <>
      {showTrigger ? (
        <div className={`announcement-center-anchor ${variant === 'inline' ? 'inline' : 'floating'}`}>
          <button
            className={trigger === 'button' ? 'announcement-trigger-btn' : 'announcement-bell-btn'}
            onClick={() => setListOpen(true)}
            title={t('announcement.title', '公告')}
          >
            <Bell size={16} />
            {trigger === 'button' ? (
              <span className="announcement-trigger-label">{t('announcement.title', '公告')}</span>
            ) : null}
            {unreadCount > 0 && (
              <span className={`announcement-bell-badge ${unreadCount === 0 ? 'is-hidden' : ''}`}>
                {unreadCount > 9 ? '9+' : unreadCount}
              </span>
            )}
          </button>
        </div>
      ) : null}

      {listOpen &&
        renderInBody(
          <div className="modal-overlay announcement-modal-overlay">
            <div ref={listDialog} tabIndex={-1} role="dialog" aria-modal="true" aria-label={t('announcement.title')} className="modal announcement-list-modal" onClick={(event) => event.stopPropagation()}>
              <div className="modal-header">
                <h2>{t('announcement.title', '公告')}</h2>
                <button className="modal-close" onClick={() => setListOpen(false)} aria-label={t('common.close', '关闭')}>
                  <X size={16} />
                </button>
              </div>
              <div className="modal-body announcement-list-body">
                <div className="announcement-toolbar">
                  <div className="announcement-toolbar-actions">
                    <button
                      className="announcement-toolbar-text-btn"
                      onClick={() => {
                        void markAllAsRead();
                      }}
                      disabled={unreadCount === 0}
                    >
                      {t('announcement.markAllRead', '全部已读')}
                    </button>
                    <button
                      className="announcement-toolbar-text-btn"
                      onClick={() => {
                        void fetchState(true);
                      }}
                      disabled={loading}
                    >
                      {t('common.refresh', '刷新')}
                    </button>
                  </div>
                </div>

                {sortedAnnouncements.length === 0 && (
                  <div className="announcement-empty">{t('announcement.empty', '暂无公告')}</div>
                )}

                {sortedAnnouncements.map((announcement) => {
                  const unread = announcementState.unreadIds.includes(announcement.id);
                  return (
                    <button
                      key={announcement.id}
                      className={`announcement-list-item ${unread ? 'is-unread' : ''}`}
                      onClick={() => {
                        void handleAnnouncementClick(announcement);
                      }}
                    >
                      <div className="announcement-list-item-top">
                        <div className="announcement-title-meta">
                          <span className={`announcement-type-chip ${sanitizeTypeClass(String(announcement.type))}`}>
                            {currentTypeLabel(String(announcement.type))}
                          </span>
                          <strong className="announcement-item-title">{announcement.title}</strong>
                          {unread && <span className="announcement-unread-dot" />}
                        </div>
                        <span className="announcement-time">{formatTimeAgo(announcement.createdAt, translateText)}</span>
                      </div>
                      <p className="announcement-summary">{announcement.summary}</p>
                    </button>
                  );
                })}
              </div>
            </div>
          </div>,
        )}

      {detailAnnouncement &&
        renderInBody(
          <div className="modal-overlay announcement-modal-overlay">
            <div ref={detailDialog} tabIndex={-1} role="dialog" aria-modal="true" aria-label={detailAnnouncement.title} className="modal announcement-detail-modal" onClick={(event) => event.stopPropagation()}>
              <div className="modal-header">
                <div className="announcement-detail-header-left">
                  {detailFromList ? (
                    <button className="btn btn-secondary icon-only" onClick={() => {
                      void closeDetail(true);
                    }} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
                  ) : null}
                  <span className={`announcement-type-chip ${sanitizeTypeClass(String(detailAnnouncement.type))}`}>
                    {currentTypeLabel(String(detailAnnouncement.type))}
                  </span>
                  <h2 className="announcement-detail-header-title">{detailAnnouncement.title}</h2>
                </div>
                <button className="modal-close" onClick={() => void closeDetail(false)} aria-label={t('common.close', '关闭')}>
                  <X size={16} />
                </button>
              </div>

              <div className="modal-body announcement-detail-body">
                <div className="announcement-detail-time">{formatTimeAgo(detailAnnouncement.createdAt, translateText)}</div>
                <div className="announcement-detail-content">{detailAnnouncement.content}</div>

                {detailAnnouncement.images && detailAnnouncement.images.length > 0 && (
                  <div className="announcement-images-grid">
                    {detailAnnouncement.images.map((image) => {
                      const imageKey = `${detailAnnouncement.id}-${image.url}`;
                      const imageFailed = failedImages.has(imageKey);
                      return (
                        <div key={imageKey} className="announcement-image-card">
                          {!imageFailed ? (
                            <img
                              src={image.url}
                              alt={image.alt || image.label || ''}
                              className="announcement-image"
                              onClick={() => {
                                if (isSafeUrl(image.url)) {
                                  setImagePreviewUrl(image.url);
                                }
                              }}
                              onError={() => {
                                setFailedImages((previous) => {
                                  const next = new Set(previous);
                                  next.add(imageKey);
                                  return next;
                                });
                              }}
                            />
                          ) : (
                            <div className="announcement-image-error">
                              {t('announcement.imageLoadFailed', '图片加载失败')}
                            </div>
                          )}
                          {image.label ? <span>{image.label}</span> : null}
                        </div>
                      );
                    })}
                  </div>
                )}
              </div>

              <div className="modal-footer">
                <button className="btn btn-secondary" onClick={() => void closeDetail(false)}>
                  {detailAnnouncement.action ? t('announcement.later', '稍后再说') : t('announcement.gotIt', '知道了')}
                </button>
                {detailAnnouncement.action ? (
                  <button
                    className="btn btn-primary"
                    onClick={() => {
                      void runAction(detailAnnouncement.action as AnnouncementAction);
                    }}
                  >
                    {(detailAnnouncement.action as AnnouncementAction).label || t('common.open', '打开')}
                  </button>
                ) : null}
              </div>
            </div>
          </div>,
        )}

      {imagePreviewUrl &&
        renderInBody(
          <div className="announcement-image-preview-overlay">
            <div ref={imageDialog} tabIndex={-1} role="dialog" aria-modal="true" aria-label={detailAnnouncement?.title || t('announcement.title')} className="announcement-image-preview-wrapper">
              <button
                type="button"
                className="announcement-image-preview-close"
                onClick={() => setImagePreviewUrl(null)}
                aria-label={t('common.close', '关闭')}
              >
                <X size={18} />
              </button>
              <img src={imagePreviewUrl} alt="preview" className="announcement-image-preview" />
            </div>
          </div>,
        )}
    </>
  );
}

export function AnnouncementCenter(props: AnnouncementCenterProps) {
  return (
    <AnnouncementSurface
      {...props}
      showTrigger
      autoPopup={false}
      autoRefresh={false}
    />
  );
}

export function AnnouncementHost({ onNavigate }: Pick<AnnouncementCenterProps, 'onNavigate'>) {
  return (
    <AnnouncementSurface
      onNavigate={onNavigate}
      showTrigger={false}
      autoPopup
      autoRefresh
    />
  );
}
