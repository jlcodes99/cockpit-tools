import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ChevronDown, CircleAlert } from 'lucide-react';

const FLOW_NOTICE_COLLAPSED_KEY = 'agtools.antigravity_cli.flow_notice_collapsed';

function readNoticeCollapsed(): boolean {
  try {
    return localStorage.getItem(FLOW_NOTICE_COLLAPSED_KEY) === 'true';
  } catch {
    return false;
  }
}

/** Antigravity CLI 账号管理说明，结构与其他平台的 flow notice 一致。 */
export function AntigravityCliFlowNotice() {
  const { t } = useTranslation();
  const [collapsed, setCollapsed] = useState(readNoticeCollapsed);

  const toggle = () => {
    setCollapsed((prev) => {
      const next = !prev;
      try {
        localStorage.setItem(FLOW_NOTICE_COLLAPSED_KEY, String(next));
      } catch {
        // ignore persistence failures
      }
      return next;
    });
  };

  return (
    <div className={`ghcp-flow-notice ${collapsed ? 'collapsed' : ''}`} role="note">
      <button type="button" className="ghcp-flow-notice-toggle" onClick={toggle}>
        <div className="ghcp-flow-notice-title">
          <CircleAlert size={16} />
          <span>{t('antigravityCli.flowNotice.title', 'Antigravity CLI 账号管理说明（点击展开/收起）')}</span>
        </div>
        <ChevronDown size={16} className={`ghcp-flow-notice-arrow ${collapsed ? 'collapsed' : ''}`} />
      </button>
      {!collapsed && (
        <div className="ghcp-flow-notice-body">
          <div className="ghcp-flow-notice-desc">{t('antigravityCli.flowNotice.desc')}</div>
          <ul className="ghcp-flow-notice-list">
            <li>{t('antigravityCli.flowNotice.permission')}</li>
            <li>{t('antigravityCli.flowNotice.network')}</li>
          </ul>
        </div>
      )}
    </div>
  );
}
