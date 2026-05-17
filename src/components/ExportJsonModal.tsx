import { Check, Copy, Download, Eye, EyeOff, FolderOpen, X } from 'lucide-react';
import { type ReactNode, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { ModalErrorMessage } from './ModalErrorMessage';
import { useEscClose } from '../hooks/useEscClose';

interface ExportJsonModalProps {
  isOpen: boolean;
  title: string;
  jsonContent: string;
  customContent?: ReactNode;
  errorMessage?: string | null;
  errorScrollKey?: string | number;
  hidden: boolean;
  copied: boolean;
  saving: boolean;
  savedPath: string | null;
  canOpenSavedDirectory: boolean;
  pathCopied: boolean;
  toolbarContent?: ReactNode;
  onClose: () => void;
  onToggleHidden: () => void;
  onCopyJson: () => Promise<void>;
  onSaveJson: () => Promise<void>;
  onOpenSavedDirectory: () => Promise<void>;
  onCopySavedPath: () => Promise<void>;
}

function maskStringValue(value: string): string {
  const length = value.length;
  if (length <= 0) return value;
  if (length <= 2) return '*'.repeat(length);
  if (length === 3) return `${value.slice(0, 1)}*${value.slice(-1)}`;
  if (length === 4) return `${value.slice(0, 1)}**${value.slice(-1)}`;
  return `${value.slice(0, 2)}***${value.slice(-2)}`;
}

function maskJsonValues(value: unknown): unknown {
  if (typeof value === 'string') {
    return maskStringValue(value);
  }

  if (Array.isArray(value)) {
    return value.map((item) => maskJsonValues(item));
  }

  if (value && typeof value === 'object') {
    const result: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(value as Record<string, unknown>)) {
      result[key] = maskJsonValues(item);
    }
    return result;
  }

  return value;
}

export function maskJsonPreviewContent(jsonContent: string): string {
  if (!jsonContent) return '';
  try {
    const parsed = JSON.parse(jsonContent) as unknown;
    const masked = maskJsonValues(parsed);
    return JSON.stringify(masked, null, 2);
  } catch {
    return jsonContent.replace(/[^\s]/g, '*');
  }
}

export function ExportJsonModal(props: ExportJsonModalProps) {
  const {
    isOpen,
    title,
    jsonContent,
    customContent,
    errorMessage,
    errorScrollKey,
    hidden,
    copied,
    saving,
    savedPath,
    canOpenSavedDirectory,
    pathCopied,
    toolbarContent,
    onClose,
    onToggleHidden,
    onCopyJson,
    onSaveJson,
    onOpenSavedDirectory,
    onCopySavedPath,
  } = props;
  const { t } = useTranslation();
  useEscClose(isOpen, onClose);

  const maskedContent = useMemo(() => {
    return maskJsonPreviewContent(jsonContent);
  }, [jsonContent]);

  if (!isOpen) return null;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal export-json-modal" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2>{title}</h2>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close', '关闭')}>
            <X />
          </button>
        </div>

        <div className="modal-body export-json-modal-body">
          {toolbarContent ? (
            <div className="export-json-toolbar">
              {toolbarContent}
            </div>
          ) : null}
          <ModalErrorMessage message={errorMessage} scrollKey={errorScrollKey} />
          {customContent ? (
            customContent
          ) : (
            <>
              <div className="export-json-actions">
                <button className="btn btn-secondary btn-sm" onClick={onToggleHidden}>
                  {hidden ? <Eye size={14} /> : <EyeOff size={14} />}
                  {hidden ? t('common.preview', '预览') : t('common.close', '关闭')}
                </button>
                <button className="btn btn-secondary btn-sm" onClick={onCopyJson}>
                  {copied ? <Check size={14} /> : <Copy size={14} />}
                  {copied ? t('common.success', '成功') : t('common.copy', '复制')}
                </button>
                <button className="btn btn-primary btn-sm" onClick={onSaveJson} disabled={saving}>
                  <Download size={14} />
                  {saving ? t('common.loading', '加载中...') : t('settings.about.download', 'Download')}
                </button>
              </div>

              <textarea
                className="export-json-textarea"
                readOnly
                spellCheck={false}
                value={hidden ? maskedContent : jsonContent}
              />

              {savedPath && (
                <div className="export-json-path-box">
                  <div className="export-json-path-title">{t('instances.labels.path', '目录')}</div>
                  <div className="export-json-path-value">{savedPath}</div>
                  <div className="export-json-path-actions">
                    <button
                      className="btn btn-secondary btn-sm"
                      onClick={onOpenSavedDirectory}
                      disabled={!canOpenSavedDirectory}
                    >
                      <FolderOpen size={14} />
                      {t('instances.actions.openFolder', '打开文件夹')}
                    </button>
                    <button className="btn btn-secondary btn-sm" onClick={onCopySavedPath}>
                      {pathCopied ? <Check size={14} /> : <Copy size={14} />}
                      {pathCopied ? t('common.success', '成功') : t('common.copy', '复制')}
                    </button>
                  </div>
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
