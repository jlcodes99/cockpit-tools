import { useEffect, useState } from 'react';
import { getCodexPlanBadgePresentation, type CodexAccount } from '../../types/codex';
import { CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT, getCodexPlanBadgeStyle, withCodexPlanBadgeStyle } from '../../utils/codexPreferences';

export function CodexAccountPlanBadge({ account }: { account: CodexAccount }) {
  const [style, setStyle] = useState(getCodexPlanBadgeStyle);
  useEffect(() => {
    const update = () => setStyle(getCodexPlanBadgeStyle());
    window.addEventListener(CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT, update);
    return () => window.removeEventListener(CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT, update);
  }, []);
  const badge = getCodexPlanBadgePresentation(account);
  return <span className={`tier-badge codex-account-plan-badge ${withCodexPlanBadgeStyle(badge.className, style)}`} title={badge.label}>{badge.label}</span>;
}
