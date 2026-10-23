import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  ArrowDown,
  ArrowUp,
  MoreHorizontal,
  PanelTop,
  RotateCcw,
  GripVertical,
} from 'lucide-react';
import { CodexToolPageHeader } from './CodexToolPageHeader';
import type { CodexRegisteredPage } from './codexPageRegistry';
import type { CodexTab } from '../CodexOverviewTabsHeader';
import {
  CODEX_TOP_TAB_LIMIT,
  createDefaultCodexTopLayout,
  normalizeCodexTopLayout,
  moveCodexTopLayoutTab,
  type CodexTopLayoutPreference,
} from '../../utils/codexTopLayoutPreferences';

interface CodexTopLayoutPageProps {
  pages: CodexRegisteredPage[];
  layout: CodexTopLayoutPreference;
  onChange: (layout: CodexTopLayoutPreference) => void;
  onBack: () => void;
}

export function CodexTopLayoutPage({
  pages,
  layout,
  onChange,
  onBack,
}: CodexTopLayoutPageProps) {
  const { t } = useTranslation();
  const [draggingTab, setDraggingTab] = useState<CodexTab | null>(null);
  const normalizedOrder = useMemo(
    () => normalizeCodexTopLayout(layout, pages.map(page => page.id)).order,
    [layout, pages],
  );

  useEffect(() => {
    if (!draggingTab) return;
    const stopDragging = () => setDraggingTab(null);
    window.addEventListener('mouseup', stopDragging);
    window.addEventListener('blur', stopDragging);
    return () => {
      window.removeEventListener('mouseup', stopDragging);
      window.removeEventListener('blur', stopDragging);
    };
  }, [draggingTab]);

  const moveTab = (index: number, offset: number) => {
    onChange(moveCodexTopLayoutTab(layout, index, index + offset, pages.map(page => page.id)));
  };

  return (
    <main className="codex-tool-page codex-top-layout-page" aria-labelledby="codex-top-layout-title">
      <CodexToolPageHeader title={t('codex.more.topLayoutTitle')} titleId="codex-top-layout-title"
        description={t('codex.more.topLayoutDesc')} icon={<PanelTop size={22} />} onBack={onBack} />
      <div className="codex-top-layout-page-card">
        <div className="codex-top-layout-body">
          <div className={`codex-top-layout-list${draggingTab ? ' is-sorting' : ''}`}
            onMouseLeave={() => setDraggingTab(null)}>
            {normalizedOrder.map((tab, index) => {
              const meta = pages.find(page => page.id === tab)!;
              const isTop = index < CODEX_TOP_TAB_LIMIT;
              return (
                <div className={`codex-top-layout-item${draggingTab === tab ? ' is-dragging' : ''}${index === CODEX_TOP_TAB_LIMIT ? ' is-more-start' : ''}`} key={tab}
                  onMouseEnter={(event) => {
                    if (!draggingTab) return;
                    if (!(event.buttons & 1)) { setDraggingTab(null); return; }
                    if (draggingTab !== tab) {
                      onChange(moveCodexTopLayoutTab(layout, normalizedOrder.indexOf(draggingTab), index, pages.map(page => page.id)));
                    }
                  }}>
                  <div className="codex-top-layout-item-main">
                    <button type="button" className="btn btn-secondary compact icon-only codex-top-layout-drag"
                      aria-label={t('platformLayout.dragHandleLabel')} title={t('platformLayout.dragHandleLabel')}
                      onMouseDown={(event) => {
                        if (event.button !== 0) return;
                        event.preventDefault();
                        setDraggingTab(tab);
                      }}>
                      <GripVertical size={16} />
                    </button>
                    <span className="codex-top-layout-index" aria-hidden="true">{String(index + 1).padStart(2, '0')}</span>
                    {meta.icon}
                    <span>{meta.label}</span>
                  </div>
                  <div className="codex-top-layout-order">
                    <button
                      type="button"
                      className="btn btn-secondary compact icon-only"
                      disabled={index === 0}
                      onClick={() => moveTab(index, -1)}
                      aria-label={t('codex.more.moveUp', '上移')}
                      title={t('codex.more.moveUp', '上移')}
                    >
                      <ArrowUp size={14} />
                    </button>
                    <button
                      type="button"
                      className="btn btn-secondary compact icon-only"
                      disabled={index === normalizedOrder.length - 1}
                      onClick={() => moveTab(index, 1)}
                      aria-label={t('codex.more.moveDown', '下移')}
                      title={t('codex.more.moveDown', '下移')}
                    >
                      <ArrowDown size={14} />
                    </button>
                  </div>
                  <span className={`codex-top-layout-location${isTop ? ' is-top' : ''}`}>
                    {isTop ? <PanelTop size={14} /> : <MoreHorizontal size={14} />}
                    {t(isTop ? 'codex.more.locationTop' : 'codex.more.locationMore')}
                  </span>
                </div>
              );
            })}
          </div>
          <p className="codex-top-layout-hint">
            {t(
              'codex.more.topLayoutHint',
              '“更多”按钮始终显示在右侧，不参与排序或隐藏。',
            )}
          </p>
        </div>
        <div className="codex-top-layout-footer">
          <button
            type="button"
            className="btn btn-secondary"
            onClick={() => onChange(createDefaultCodexTopLayout(pages.map(page => page.id)))}
          >
            <RotateCcw size={14} />
            {t('codex.more.resetLayout', '恢复默认')}
          </button>
          <button type="button" className="btn btn-primary" onClick={onBack}>
            {t('codex.more.done', '完成')}
          </button>
        </div>
      </div>
    </main>
  );
}
