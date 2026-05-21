import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ChevronDown, Copy, FileText, FolderOpen, RefreshCw, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { getLogSnapshot, openLogDirectory, type LogSnapshot } from '../services/logService';
import { useEscClose } from '../hooks/useEscClose';
import './LogViewerModal.css';

interface LogViewerModalProps {
  open: boolean;
  onClose: () => void;
}

type LogLevelFilter = 'ALL' | 'INFO' | 'WARN' | 'ERROR';

const DEFAULT_LINE_LIMIT = 200;
const MIN_LINE_LIMIT = 20;
const MAX_LINE_LIMIT = 5000;
const POLL_INTERVAL_MS = 1000;
const FEEDBACK_DURATION_MS = 1200;
const LOG_ENTRY_LEVEL_PATTERN = /^\S+\s+(INFO|WARN|ERROR)\s/;

function clampLineLimit(value: number): number {
  if (!Number.isFinite(value)) {
    return DEFAULT_LINE_LIMIT;
  }
  return Math.min(MAX_LINE_LIMIT, Math.max(MIN_LINE_LIMIT, Math.round(value)));
}

function filterLogContent(content: string, level: LogLevelFilter): string {
  if (level === 'ALL' || !content) {
    return content;
  }

  const lines = content.split('\n');
  const matchedEntries: string[] = [];
  let currentEntry: string[] = [];
  let currentLevel: LogLevelFilter | null = null;

  const flushEntry = () => {
    if (currentEntry.length > 0 && currentLevel === level) {
      matchedEntries.push(currentEntry.join('\n'));
    }
    currentEntry = [];
    currentLevel = null;
  };

  for (const line of lines) {
    const matchedLevel = line.match(LOG_ENTRY_LEVEL_PATTERN)?.[1] as LogLevelFilter | undefined;
    if (matchedLevel) {
      flushEntry();
      currentEntry = [line];
      currentLevel = matchedLevel;
      continue;
    }

    if (currentEntry.length > 0) {
      currentEntry.push(line);
    }
  }

  flushEntry();
  return matchedEntries.join('\n');
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
  const [snapshot, setSnapshot] = useState<LogSnapshot | null>(null);
  const [rawContent, setRawContent] = useState<string>('');
  const [visibleRawContent, setVisibleRawContent] = useState<string>('');
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

  const displayedContent = useMemo(
    () => filterLogContent(visibleRawContent, levelFilter),
    [levelFilter, visibleRawContent],
  );

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

        const next = await getLogSnapshot(selectedFileName || undefined, lineLimit);
        setSnapshot(next);
        setError('');
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
        setError(String(err));
      } finally {
        if (showLoading) {
          setLoading(false);
        }
      }
    },
    [lineLimit, selectedFileName],
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
    levelFilter !== 'ALL' &&
    visibleRawContent.trim().length > 0 &&
    displayedContent.trim().length === 0;

  const handleClearOutput = () => {
    clearMarkerRef.current = rawContent;
    setVisibleRawContent('');
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
            <div className="log-viewer-meta-item log-viewer-file-item">
              <FileText size={14} />
              {snapshot?.available_files?.length ? (
                <div className="log-viewer-select-wrap">
                  <select
                    className="log-viewer-select"
                    value={activeFileName}
                    onChange={(event) => {
                      setSelectedFileName(event.target.value);
                      setError('');
                    }}
                    aria-label={t('logViewer.fileLabel', '日志文件')}
                  >
                    {snapshot.available_files.map((file) => (
                      <option key={file.log_file_name} value={file.log_file_name}>
                        {file.log_file_name}
                      </option>
                    ))}
                  </select>
                  <ChevronDown size={14} />
                </div>
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
                <div className="log-viewer-select-wrap log-viewer-level-select-wrap">
                  <select
                    className="log-viewer-select"
                    value={levelFilter}
                    onChange={(event) => setLevelFilter(event.target.value as LogLevelFilter)}
                    aria-label={t('logViewer.levelLabel', '级别')}
                  >
                    {levelOptions.map((option) => (
                      <option key={option.value} value={option.value}>
                        {option.label}
                      </option>
                    ))}
                  </select>
                  <ChevronDown size={14} />
                </div>
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
