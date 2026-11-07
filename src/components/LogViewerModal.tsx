import { SingleSelectDropdown } from './SingleSelectDropdown';
import { useLogContentFilter } from '../hooks/useLogContentFilter';
import type { LogLevelFilter } from '../utils/logContentFilter';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Copy, FileText, FolderOpen, RefreshCw, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { getLogSnapshot, openLogDirectory, type LogSnapshot } from '../services/logService';
import { useEscClose } from '../hooks/useEscClose';
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
  useEscClose(open, onClose);
  const logsLabel = t('manual.dataPrivacy.keywords.5', '日志');
  const logDirLabel = t('manual.dataPrivacy.keywords.6', '日志目录');
  const levelOptions: Array<{ value: LogLevelFilter; label: string }> = useMemo(
    () => [
      { value: 'ALL', label: t('logViewer.levels.all', '全部') },
      { value: 'INFO', label: t('logViewer.levels.info', 'INFO') },
      { value: 'WARN', label: t('logViewer.levels.warn', 'WARN') },
      { value: 'ERROR', label: t('logViewer.levels.error', 'ERROR') },
    ],
    [t],
  );

  const [lineLimit, setLineLimit] = useState<number>(DEFAULT_LINE_LIMIT);
  const [lineLimitDraft, setLineLimitDraft] = useState<string>(String(DEFAULT_LINE_LIMIT));
  const [selectedFileName, setSelectedFileName] = useState<string>('');
  const [levelFilter, setLevelFilter] = useState<LogLevelFilter>('ALL');
  const [contentFilter, setContentFilter] = useState('');
  const [useRegex, setUseRegex] = useState(false);
  const filterInputRef = useRef<HTMLInputElement>(null);
  const errorRef = useRef<HTMLParagraphElement>(null);
  const [snapshot, setSnapshot] = useState<LogSnapshot | null>(null);
  const [rawContent, setRawContent] = useState<string>('');
  const [visibleRawContent, setVisibleRawContent] = useState<string>('');
  const [loading, setLoading] = useState<boolean>(false);
  const [error, setError] = useState<string>('');
  const [snapshotError, setSnapshotError] = useState('');
  const displayedError = error || snapshotError;
  const snapshotInFlightRef = useRef(false);
  const snapshotVersionRef = useRef(0);
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

  const filtered = useLogContentFilter(open, {
    content: visibleRawContent, level: levelFilter, pattern: contentFilter, regex: useRegex,
  });
  const displayedContent = filtered.content;
  const filterError = filtered.error ? t(`logViewer.search.errors.${filtered.error}`) : '';
  useEffect(() => {
    if (filterError) { filterInputRef.current?.focus(); filterInputRef.current?.scrollIntoView({ block: 'nearest' }); }
  }, [filterError]);
  useEffect(() => {
    if (displayedError) errorRef.current?.scrollIntoView({ block: 'nearest' });
  }, [displayedError]);

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
      if (showLoading) { setError(''); setSnapshotError(''); setLoading(true); }
      if (snapshotInFlightRef.current) return;
      snapshotInFlightRef.current = true;
      const version = snapshotVersionRef.current;
      try {
        const next = await getLogSnapshot(selectedFileName || undefined, lineLimit);
        if (version !== snapshotVersionRef.current) return;
        setSnapshot(next);
        setSnapshotError('');
        setRawContent(next.content);

        const marker = clearMarkerRef.current;
        let nextVisible = next.content;
        if (marker !== null) {
          if (next.content === marker) {
            nextVisible = '';
          } else if (next.content.startsWith(marker)) {
            nextVisible = next.content.slice(marker.length).replace(/^\n+/, '');
          }

          if (nextVisible.length > 0) {
            clearMarkerRef.current = null;
          }
        }

        setVisibleRawContent(nextVisible);
      } catch (err) {
        if (version === snapshotVersionRef.current) setSnapshotError(String(err));
      } finally {
        snapshotInFlightRef.current = false;
        if (version === snapshotVersionRef.current) setLoading(false);
      }
    },
    [lineLimit, selectedFileName],
  );

  useEffect(() => {
    snapshotVersionRef.current++;
    if (!open) {
      return;
    }

    void loadSnapshot(true);
    const timer = window.setInterval(() => {
      void loadSnapshot(false);
    }, POLL_INTERVAL_MS);

    return () => {
      window.clearInterval(timer);
      snapshotVersionRef.current++;
    };
  }, [loadSnapshot, open]);

  useEffect(() => {
    clearMarkerRef.current = null;
  }, [selectedFileName]);

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

  const activeFileName = selectedFileName || snapshot?.log_file_name || '';
  const hasFilteredOutContent =
    (levelFilter !== 'ALL' || Boolean(contentFilter.trim())) &&
    visibleRawContent.trim().length > 0 &&
    displayedContent.trim().length === 0;

  const handleClearOutput = () => {
    clearMarkerRef.current = rawContent;
    setVisibleRawContent('');
    setError('');
  };

  const handleCopyLogs = async () => {
    setError('');
    try {
      await navigator.clipboard.writeText(displayedContent);
      setCopied(true);
      window.setTimeout(() => setCopied(false), FEEDBACK_DURATION_MS);
    } catch (err) {
      setError(String(err));
    }
  };

  const handleCopyPath = async () => {
    setError('');
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
    setError('');
    try {
      await openLogDirectory();
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <div className="modal-overlay log-viewer-overlay">
      <div className="modal log-viewer-modal" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2>{logsLabel}</h2>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close', '关闭')}>
            <X size={16} />
          </button>
        </div>

        <div className="modal-body log-viewer-body">
          <div className="log-viewer-meta">
            <div className="log-viewer-meta-item log-viewer-file-item">
              <FileText size={14} />
              {snapshot?.available_files?.length ? (
                <SingleSelectDropdown className="log-viewer-dropdown" value={activeFileName}
                  options={snapshot.available_files.map((file) => ({ value: file.log_file_name, label: file.log_file_name }))}
                  onChange={(value) => { setSelectedFileName(value); setError(''); }}
                  ariaLabel={t('logViewer.fileLabel')} />
              ) : (
                <span className="log-viewer-path-text">-</span>
              )}
            </div>
            <div className="log-viewer-meta-item">
              <FolderOpen size={14} />
              <span className="log-viewer-path-text">{snapshot?.log_dir_path || '-'}</span>
            </div>
            <div className="log-viewer-meta-item">
              <RefreshCw size={14} />
              <span>{updatedAtText}</span>
            </div>
            <div className="log-viewer-toolbar">
              <div className="log-viewer-filter-wrap">
                <span className="log-viewer-line-limit-label">
                  {t('logViewer.levelLabel', '级别')}
                </span>
                <SingleSelectDropdown className="log-viewer-dropdown" value={levelFilter} options={levelOptions}
                  onChange={(value) => setLevelFilter(value as LogLevelFilter)}
                  ariaLabel={t('logViewer.levelLabel')} />
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
          </div>

          <div className="log-viewer-search">
            <input ref={filterInputRef} className={`log-viewer-search-input${filterError ? ' has-error' : ''}`}
              value={contentFilter} maxLength={256} placeholder={t('logViewer.search.placeholder')}
              aria-label={t('logViewer.search.placeholder')} aria-invalid={Boolean(filterError)}
              aria-describedby={filterError ? 'log-filter-error' : undefined}
              onChange={(event) => setContentFilter(event.target.value)} />
            <button type="button" className="btn btn-secondary" aria-pressed={useRegex}
              onClick={() => setUseRegex((value) => !value)}>{t('logViewer.search.regex')}</button>
            <button type="button" className="btn btn-ghost" disabled={!contentFilter}
              onClick={() => setContentFilter('')}>{t('common.clear')}</button>
            {filtered.loading ? <span className="log-viewer-search-status">{t('common.loading')}</span> : null}
            {filterError ? <p id="log-filter-error" className="log-viewer-error" role="alert">{filterError}</p> : null}
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
              <div className="log-viewer-placeholder">
                {hasFilteredOutContent
                  ? t('logViewer.noMatches', '当前筛选下无匹配日志')
                  : t('common.none', '暂无')}
              </div>
            )}
          </div>

          {displayedError ? <p ref={errorRef} role="alert" className="log-viewer-error">{displayedError}</p> : null}
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
          <button className="btn btn-primary" disabled={filtered.loading || Boolean(filterError)} onClick={() => void handleCopyLogs()}>
            <Copy size={14} />
            {copied ? t('common.success', '成功') : `${t('common.copy', '复制')} ${logsLabel}`}
          </button>
        </div>
      </div>
    </div>
  );
}
