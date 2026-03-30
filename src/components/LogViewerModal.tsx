import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Copy, FileText, FolderOpen, RefreshCw, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  getLatestLogSnapshot,
  openLogDirectory,
  type LatestLogSnapshot,
} from '../services/logService';
import './LogViewerModal.css';

interface LogViewerModalProps {
  open: boolean;
  onClose: () => void;
}

const DEFAULT_LINE_LIMIT = 200;
const MIN_LINE_LIMIT = 20;
const MAX_LINE_LIMIT = 5000;
const POLL_INTERVAL_MS = 1000;
const FEEDBACK_DURATION_MS = 1200;

function clampLineLimit(value: number): number {
  if (!Number.isFinite(value)) {
    return DEFAULT_LINE_LIMIT;
  }
  return Math.min(MAX_LINE_LIMIT, Math.max(MIN_LINE_LIMIT, Math.round(value)));
}

export function LogViewerModal({ open, onClose }: LogViewerModalProps) {
  const { t } = useTranslation();
  const logsLabel = t('manual.dataPrivacy.keywords.5', '日志');
  const logDirLabel = t('manual.dataPrivacy.keywords.6', '日志目录');

  const [lineLimit, setLineLimit] = useState<number>(DEFAULT_LINE_LIMIT);
  const [lineLimitDraft, setLineLimitDraft] = useState<string>(String(DEFAULT_LINE_LIMIT));
  const [snapshot, setSnapshot] = useState<LatestLogSnapshot | null>(null);
  const [rawContent, setRawContent] = useState<string>('');
  const [displayedContent, setDisplayedContent] = useState<string>('');
  const [loading, setLoading] = useState<boolean>(false);
  const [error, setError] = useState<string>('');
  const [copied, setCopied] = useState<boolean>(false);
  const [pathCopied, setPathCopied] = useState<boolean>(false);

  const viewRef = useRef<HTMLDivElement>(null);
  const shouldStickToBottomRef = useRef<boolean>(true);
  const clearMarkerRef = useRef<string | null>(null);

  const updatedAtText = useMemo(() => {
    if (!snapshot?.modified_at_ms) {
      return '-';
    }
    const date = new Date(snapshot.modified_at_ms);
    if (Number.isNaN(date.getTime())) {
      return '-';
    }
    return date.toLocaleString();
  }, [snapshot?.modified_at_ms]);

  const applyLineLimit = useCallback(() => {
    const parsed = Number.parseInt(lineLimitDraft.trim(), 10);
    if (!Number.isFinite(parsed)) {
      setLineLimitDraft(String(lineLimit));
      return;
    }
    const next = clampLineLimit(parsed);
    setLineLimit(next);
    setLineLimitDraft(String(next));
  }, [lineLimit, lineLimitDraft]);

  const loadSnapshot = useCallback(
    async (showLoading: boolean) => {
      try {
        if (showLoading) {
          setLoading(true);
        }
        const next = await getLatestLogSnapshot(lineLimit);
        setSnapshot(next);
        setError('');
        setRawContent(next.content);

        const marker = clearMarkerRef.current;
        let nextDisplay = next.content;
        if (marker !== null) {
          if (next.content === marker) {
            nextDisplay = '';
          } else if (next.content.startsWith(marker)) {
            nextDisplay = next.content.slice(marker.length).replace(/^\n+/, '');
          } else {
            nextDisplay = next.content;
          }

          if (nextDisplay.length > 0) {
            clearMarkerRef.current = null;
          }
        }

        setDisplayedContent(nextDisplay);
      } catch (err) {
        setError(String(err));
      } finally {
        if (showLoading) {
          setLoading(false);
        }
      }
    },
    [lineLimit],
  );

  useEffect(() => {
    if (!open) {
      return;
    }

    void loadSnapshot(true);
    const timer = window.setInterval(() => {
      void loadSnapshot(false);
    }, POLL_INTERVAL_MS);

    return () => {
      window.clearInterval(timer);
    };
  }, [loadSnapshot, open]);

  useEffect(() => {
    if (!open) {
      clearMarkerRef.current = null;
      return;
    }

    const view = viewRef.current;
    if (!view || !shouldStickToBottomRef.current) {
      return;
    }
    view.scrollTop = view.scrollHeight;
  }, [displayedContent, open]);

  if (!open) {
    return null;
  }

  const handleClearOutput = () => {
    clearMarkerRef.current = rawContent;
    setDisplayedContent('');
    setError('');
  };

  const handleCopyLogs = async () => {
    try {
      await navigator.clipboard.writeText(displayedContent);
      setCopied(true);
      window.setTimeout(() => setCopied(false), FEEDBACK_DURATION_MS);
    } catch (err) {
      setError(String(err));
    }
  };

  const handleCopyPath = async () => {
    if (!snapshot?.log_file_path) {
      return;
    }
    try {
      await navigator.clipboard.writeText(snapshot.log_file_path);
      setPathCopied(true);
      window.setTimeout(() => setPathCopied(false), FEEDBACK_DURATION_MS);
    } catch (err) {
      setError(String(err));
    }
  };

  const handleOpenDir = async () => {
    try {
      await openLogDirectory();
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <div className="modal-overlay log-viewer-overlay" onClick={onClose}>
      <div className="modal log-viewer-modal" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2>{logsLabel}</h2>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close', '关闭')}>
            <X size={16} />
          </button>
        </div>

        <div className="modal-body log-viewer-body">
          <div className="log-viewer-meta">
            <div className="log-viewer-meta-item">
              <FileText size={14} />
              <span className="log-viewer-path-text">
                {snapshot?.log_file_name || '-'}
              </span>
            </div>
            <div className="log-viewer-meta-item">
              <FolderOpen size={14} />
              <span className="log-viewer-path-text">
                {snapshot?.log_dir_path || '-'}
              </span>
            </div>
            <div className="log-viewer-meta-item">
              <RefreshCw size={14} />
              <span>{updatedAtText}</span>
            </div>
            <div className="log-viewer-line-limit-wrap">
              <span className="log-viewer-line-limit-label">
                {t('pagination.perPage', { count: lineLimit, defaultValue: '{{count}} / page' })}
              </span>
              <input
                className="log-viewer-line-limit-input"
                type="number"
                min={MIN_LINE_LIMIT}
                max={MAX_LINE_LIMIT}
                value={lineLimitDraft}
                onChange={(event) => setLineLimitDraft(event.target.value)}
                onBlur={applyLineLimit}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    applyLineLimit();
                  }
                }}
              />
            </div>
          </div>

          <div
            className="log-viewer-content"
            ref={viewRef}
            onScroll={(event) => {
              const target = event.currentTarget;
              const bottomDistance = target.scrollHeight - target.scrollTop - target.clientHeight;
              shouldStickToBottomRef.current = bottomDistance <= 24;
            }}
          >
            {loading && !displayedContent ? (
              <div className="log-viewer-placeholder">{t('common.loading', '加载中...')}</div>
            ) : displayedContent ? (
              <pre>{displayedContent}</pre>
            ) : (
              <div className="log-viewer-placeholder">{t('common.none', '暂无')}</div>
            )}
          </div>

          {error ? <p className="log-viewer-error">{error}</p> : null}
        </div>

        <div className="modal-footer log-viewer-footer">
          <button className="btn btn-ghost" onClick={onClose}>
            {t('common.close', '关闭')}
          </button>
          <button className="btn btn-secondary" onClick={() => void loadSnapshot(true)}>
            {t('common.refresh', '刷新')}
          </button>
          <button className="btn btn-secondary" onClick={handleClearOutput}>
            {t('breakout.historyClear', '清空')}
          </button>
          <button className="btn btn-secondary" onClick={handleOpenDir}>
            {t('common.open', '打开')} {logDirLabel}
          </button>
          <button className="btn btn-secondary" onClick={() => void handleCopyPath()}>
            {pathCopied
              ? t('common.success', '成功')
              : `${t('common.copy', '复制')} ${t('error.fileCorrupted.filePath', '文件位置')}`}
          </button>
          <button className="btn btn-primary" onClick={() => void handleCopyLogs()}>
            <Copy size={14} />
            {copied ? t('common.success', '成功') : `${t('common.copy', '复制')} ${logsLabel}`}
          </button>
        </div>
      </div>
    </div>
  );
}
