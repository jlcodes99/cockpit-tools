import { useEffect, useMemo, useState } from 'react';
import { X, Tag, Plus } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { globalRenameTag, globalDeleteTag } from '../utils/globalTagOperations';
import { useEscClose } from '../hooks/useEscClose';
import './TagEditModal.css';

interface TagEditModalProps {
  isOpen: boolean;
  initialTags: string[];
  initialNotes?: string;
  availableTags?: string[];
  onClose: () => void;
  onSave: (tags: string[], notes?: string) => void | Promise<void>;
}

const MAX_TAGS = 10;
const MAX_TAG_LENGTH = 20;
const MAX_NOTES_LENGTH = 200;

const normalizeTag = (value: string) => value.trim().toLowerCase();

const normalizeTagList = (tags: string[]) => {
  const seen = new Set<string>();
  const result: string[] = [];
  tags.forEach((tag) => {
    const normalized = normalizeTag(tag);
    if (!normalized) return;
    if (seen.has(normalized)) return;
    seen.add(normalized);
    result.push(normalized);
  });
  return result;
};

export const TagEditModal = ({ isOpen, initialTags, initialNotes, availableTags = [], onClose, onSave }: TagEditModalProps) => {
  const { t } = useTranslation();
  useEscClose(isOpen, onClose);
  const [tags, setTags] = useState<string[]>([]);
  const [notes, setNotes] = useState('');
  const [inputValue, setInputValue] = useState('');
  const [error, setError] = useState<string>('');
  const [saving, setSaving] = useState(false);
  const [globalRenamingTag, setGlobalRenamingTag] = useState<string | null>(null);

  useEffect(() => {
    if (!isOpen) return;
    setTags(normalizeTagList(initialTags));
    setNotes(initialNotes ?? '');
    setInputValue('');
    setError('');
    setGlobalRenamingTag(null);
  }, [initialNotes, initialTags, isOpen]);

  const remaining = useMemo(() => MAX_TAGS - tags.length, [tags.length]);
  const normalizedAvailableTags = useMemo(() => normalizeTagList(availableTags), [availableTags]);
  const suggestedTags = useMemo(() => {
    const base = normalizedAvailableTags.filter((tag) => !tags.includes(tag));
    const query = normalizeTag(inputValue);
    if (!query) return base;
    return base.filter((tag) => tag.includes(query));
  }, [normalizedAvailableTags, tags, inputValue]);

  const addTag = (rawValue: string) => {
    const normalized = normalizeTag(rawValue);
    if (!normalized) {
      setError(t('accounts.tagModal.error.empty', '标签不能为空'));
      return;
    }
    if (normalized.length > MAX_TAG_LENGTH) {
      setError(
        t('accounts.tagModal.error.tooLong', {
          max: MAX_TAG_LENGTH,
          defaultValue: '标签长度不能超过 {{max}} 个字符',
        })
      );
      return;
    }
    if (tags.includes(normalized)) {
      setError(t('accounts.tagModal.error.duplicate', '标签已存在'));
      return;
    }
    if (tags.length >= MAX_TAGS) {
      setError(
        t('accounts.tagModal.error.tooMany', {
          max: MAX_TAGS,
          defaultValue: '标签数量不能超过 {{max}} 个',
        })
      );
      return;
    }
    setTags((prev) => [...prev, normalized]);
    setInputValue('');
    setError('');
  };

  const removeTag = (target: string) => {
    setTags((prev) => prev.filter((tag) => tag !== target));
    setError('');
  };

  const buildTagsForSave = () => {
    const rawInput = normalizeTag(inputValue);
    if (!rawInput) {
      return { nextTags: tags, error: '' };
    }
    if (rawInput.length > MAX_TAG_LENGTH) {
      return {
        nextTags: tags,
        error: t('accounts.tagModal.error.tooLong', {
          max: MAX_TAG_LENGTH,
          defaultValue: '标签长度不能超过 {{max}} 个字符',
        }),
      };
    }
    const exists = tags.includes(rawInput);
    if (!exists && tags.length >= MAX_TAGS) {
      return {
        nextTags: tags,
        error: t('accounts.tagModal.error.tooMany', {
          max: MAX_TAGS,
          defaultValue: '标签数量不能超过 {{max}} 个',
        }),
      };
    }
    const merged = exists ? tags : [...tags, rawInput];
    return { nextTags: merged, error: '' };
  };

  const handleSave = async () => {
    if (saving) return;
    const { nextTags, error: saveError } = buildTagsForSave();
    if (saveError) {
      setError(saveError);
      return;
    }
    setSaving(true);
    try {
      await onSave(nextTags, notes.trim());
      onClose();
    } finally {
      setSaving(false);
    }
  };

  const handleGlobalRename = async () => {
    if (saving || !globalRenamingTag) return;
    const newName = normalizeTag(inputValue);
    if (!newName) {
      setError(t('accounts.tagModal.error.empty', '标签不能为空'));
      return;
    }
    if (newName.length > MAX_TAG_LENGTH) {
      setError(t('accounts.tagModal.error.tooLong', { max: MAX_TAG_LENGTH, defaultValue: `标签长度不能超过 ${MAX_TAG_LENGTH} 个字符` }));
      return;
    }
    setSaving(true);
    try {
      await globalRenameTag(globalRenamingTag, newName);
      // Automatically update the local view to reflect the rename
      setTags(prev => prev.map(t => t === globalRenamingTag ? newName : t));
      setInputValue('');
      setGlobalRenamingTag(null);
      setError('');
    } catch (err: any) {
      setError(err?.message || '全局重命名失败');
    } finally {
      setSaving(false);
    }
  };

  const handleGlobalDelete = async (targetTag: string) => {
    if (saving) return;
    if (!window.confirm(t('accounts.tagModal.confirmGlobalDelete', { targetTag, defaultValue: `确定要从所有账号中全局删除标签 "${targetTag}" 吗？此操作不可逆。` }))) {
      return;
    }
    setSaving(true);
    try {
      await globalDeleteTag(targetTag);
      setTags(prev => prev.filter(t => t !== targetTag));
      setError('');
      if (globalRenamingTag === targetTag) {
        setGlobalRenamingTag(null);
        setInputValue('');
      }
    } catch (err: any) {
      setError(err?.message || '全局删除失败');
    } finally {
      setSaving(false);
    }
  };

  if (!isOpen) return null;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal tag-edit-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2 className="tag-edit-title">
            <Tag size={18} />
            {t('accounts.tagModal.title', '账户标签')}
          </h2>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close', '关闭')}>
            <X size={18} />
          </button>
        </div>
        <div className="modal-body tag-edit-body">
          <div className="tag-edit-hint">
            {t('accounts.tagModal.hint', {
              max: MAX_TAGS,
              maxLength: MAX_TAG_LENGTH,
              defaultValue: '最多 {{max}} 个标签，单个标签长度不超过 {{maxLength}} 个字符。',
            })}
          </div>
          {initialNotes !== undefined && (
            <div className="tag-notes-section">
              <div className="tag-notes-header">
                <span className="tag-notes-label">{t('accounts.tagModal.notesLabel')}</span>
                <span className="tag-notes-count">{notes.length}/{MAX_NOTES_LENGTH}</span>
              </div>
              <textarea
                value={notes}
                onChange={(e) => setNotes(e.target.value)}
                placeholder={t('accounts.tagModal.notesPlaceholder')}
                maxLength={MAX_NOTES_LENGTH}
              />
            </div>
          )}
          <div className="tag-list">
            {tags.length === 0 ? (
              <div className="tag-empty">{t('accounts.tagModal.empty', '暂无标签')}</div>
            ) : (
              tags.map((tag) => (
                <span key={tag} className="tag-chip">
                  <span
                    className="tag-text"
                    onClick={() => {
                      setInputValue(tag);
                      removeTag(tag);
                    }}
                    style={{ cursor: 'pointer' }}
                    title={t('accounts.tagModal.editHint', '点击修改')}
                  >
                    {tag}
                  </span>
                  <button
                    type="button"
                    className="tag-remove"
                    onClick={() => removeTag(tag)}
                    aria-label={t('accounts.deleteTagAria', {
                      tag,
                      defaultValue: '删除标签 {{tag}}',
                    })}
                  >
                    <X size={12} />
                  </button>
                </span>
              ))
            )}
          </div>
          {normalizedAvailableTags.length > 0 && (
            <div className="tag-suggestions">
              <div className="tag-suggestions-title">{t('accounts.tagModal.suggestionsTitle', '已有标签')}</div>
              {suggestedTags.length === 0 ? (
                <div className="tag-suggestions-empty">{t('accounts.tagModal.suggestionsEmpty', '暂无可选标签')}</div>
              ) : (
                <div className="tag-suggestions-list">
                  {suggestedTags.map((tag) => (
                    <div className="tag-suggestion-wrap" key={tag} style={{ display: 'inline-flex', alignItems: 'center', background: 'var(--tag-bg, #f3f4f6)', borderRadius: 12, paddingRight: 4, marginRight: 6, marginBottom: 6 }}>
                      <button
                        type="button"
                        className="tag-suggestion"
                        style={{ border: 'none', background: 'transparent', paddingRight: 4, margin: 0 }}
                        onClick={() => addTag(tag)}
                        title={t('accounts.tagModal.addHint', '点击添加')}
                      >
                        <Tag size={12} style={{ marginRight: 4 }} />
                        {tag}
                      </button>
                      <button
                        type="button"
                        className="tag-suggestion-edit-btn"
                        onClick={(e) => {
                          e.stopPropagation();
                          setGlobalRenamingTag(tag);
                          setInputValue(tag);
                          setError('');
                        }}
                        title={t('accounts.tagModal.globalRenameHint', '全局重命名此标签')}
                        style={{
                          background: 'transparent',
                          border: 'none',
                          cursor: 'pointer',
                          color: 'var(--text-secondary, #6b7280)',
                          padding: '2px 4px',
                          display: 'flex',
                          alignItems: 'center',
                          borderRadius: '50%'
                        }}
                      >
                        <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/></svg>
                      </button>
                      <button
                        type="button"
                        className="tag-suggestion-delete-btn"
                        onClick={(e) => {
                          e.stopPropagation();
                          handleGlobalDelete(tag);
                        }}
                        title={t('accounts.tagModal.globalDeleteHint', '全局删除此标签')}
                        style={{
                          background: 'transparent',
                          border: 'none',
                          cursor: 'pointer',
                          color: 'var(--text-secondary, #6b7280)',
                          padding: '2px 4px',
                          display: 'flex',
                          alignItems: 'center',
                          borderRadius: '50%'
                        }}
                      >
                        <X size={12} />
                      </button>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
          {globalRenamingTag ? (
            <div className="tag-input-row">
              <div className="tag-global-rename-hint" style={{ fontSize: 12, color: 'var(--text-secondary, #6b7280)', marginBottom: 4 }}>
                {t('accounts.tagModal.renamingGlobal', '正在全局重命名已存在标签所有账号记录：')} <b>{globalRenamingTag}</b>
              </div>
              <div className="tag-input-wrap">
                <input
                  type="text"
                  value={inputValue}
                  onChange={(e) => {
                    setInputValue(e.target.value);
                    if (error) setError('');
                  }}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter') {
                      e.preventDefault();
                      handleGlobalRename();
                    } else if (e.key === 'Escape') {
                      setGlobalRenamingTag(null);
                      setInputValue('');
                    }
                  }}
                  placeholder={t('accounts.tagModal.renamePlaceholder', '输入新的标签名称')}
                  autoFocus
                />
                <button
                  type="button"
                  className="btn btn-primary tag-add-btn"
                  onClick={handleGlobalRename}
                  disabled={!inputValue.trim() || saving}
                >
                  {t('common.save', '保存')}
                </button>
                <button
                  type="button"
                  className="btn btn-secondary tag-add-btn"
                  onClick={() => { setGlobalRenamingTag(null); setInputValue(''); setError(''); }}
                  disabled={saving}
                >
                  <X size={14} />
                </button>
              </div>
            </div>
          ) : (
            <div className="tag-input-row">
              <div className="tag-input-wrap">
                <input
                type="text"
                value={inputValue}
                onChange={(e) => {
                  setInputValue(e.target.value);
                  if (error) setError('');
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    e.preventDefault();
                    addTag(inputValue);
                  }
                }}
                placeholder={remaining > 0
                  ? t('accounts.tagModal.inputPlaceholder', {
                    remaining,
                    defaultValue: '输入标签（还能添加 {{remaining}} 个）',
                  })
                  : t('accounts.tagModal.inputDisabledPlaceholder', '已达到标签上限')}
                disabled={remaining <= 0}
              />
              <button
                type="button"
                className="btn btn-secondary tag-add-btn"
                onClick={() => addTag(inputValue)}
                disabled={!inputValue.trim() || remaining <= 0}
              >
                <Plus size={14} />
                {t('accounts.tagModal.add', '添加')}
              </button>
            </div>
          </div>
          )}
          {error && <div className="tag-edit-error">{error}</div>}
        </div>
        <div className="modal-footer tag-edit-footer">
          <button className="btn btn-secondary" onClick={onClose} disabled={saving}>
            {t('common.cancel', '取消')}
          </button>
          <button className="btn btn-primary" onClick={handleSave} disabled={saving}>
            {saving ? t('common.saving', '保存中...') : t('accounts.tagModal.save', '保存标签')}
          </button>
        </div>
      </div>
    </div>
  );
};

export default TagEditModal;
