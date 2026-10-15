import React from 'react';
import { createRoot } from 'react-dom/client';
import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import { CodexTeamQuotaHistory } from '../src/components/codex/CodexTeamQuotaHistory';
import type { CodexAccount } from '../src/types/codex';
import '../src/styles/pages/codex-account-cards.css';

await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: {}, interpolation: { escapeValue: false } });
const now = Math.floor(Date.now() / 1000);
const base = {
  id: 'fixture', email: 'fixture@example.invalid', user_id: 'user-a', account_id: 'space-a',
  plan_type: 'self_serve_business_usage_based', created_at: now, last_used: now,
} as CodexAccount;
const history = { user_id: 'user-a', account_id: 'space-a', observed_at: now - 120, hourly_reset_time: now + 90, weekly_reset_time: now + 604800, hourly_percentage: 0, weekly_percentage: 64 };
const cases = [
  ['countdown', { ...base, team_quota_history: history }],
  ['elapsed', { ...base, team_quota_history: { ...history, hourly_reset_time: now - 60 } }],
  ['unknown', base],
  ['wrong-space', { ...base, team_quota_history: { ...history, account_id: 'space-b' } }],
  ['team-hidden', { ...base, plan_type: 'team', team_quota_history: history }],
] as const;
createRoot(document.getElementById('root')!).render(<>
  <style>{`:root { --bg-card: white; --border: #ccc; --radius-lg: 8px; } body { margin: 16px; font-family: system-ui; color: #222; background: #f4f5f6; } main { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(260px, 100%), 1fr)); gap: 16px; } article { min-width: 0; }`}</style>
  <main className="codex-accounts-page">{cases.map(([label, account]) => <article className="codex-account-card" data-case={label} key={label}>
    <h3 style={{fontSize: 14}}>{label}</h3>
    <div className="quota-grid"><CodexTeamQuotaHistory account={account} /></div>
  </article>)}</main>
</>);
