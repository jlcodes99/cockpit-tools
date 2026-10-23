import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Plus, RefreshCw, Settings2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { ChatGptPlanAccountCard } from './ChatGptPlanAccountCard';
import type { CodexLocalAccessAccountStats } from '../../types/codexLocalAccess';
import { ChatGptPlanDialog } from './ChatGptPlanDialog';
import {
  chatGptPlanErrorKey, chatGptPlanStatusKey, listChatGptPlanAccounts,
  withChatGptPlanMembership,
  type ChatGptPlanAccount,
} from '../../services/chatgptPlanService';
import './ChatGptPlan.css';
import { CHATGPT_PLAN_OPEN_REQUEST, takePendingChatgptPlanAuthorization } from '../../utils/chatgptPlanNavigation';
import { subscribeTauriEvent } from '../../utils/tauriEventListener';

interface Props {
  onChanged: () => void;
  memberAccountIds?: readonly string[];
  variant?: 'list' | 'cards' | 'controller';
  maskAccountText?: (value: string) => string;
  accountStats?: readonly CodexLocalAccessAccountStats[];
  searchQuery?: string;
  onCountChange?: (count: number) => void;
}

export function ChatGptPlanAccounts({ onChanged, memberAccountIds, variant = 'list', maskAccountText, accountStats, searchQuery = '', onCountChange }: Props) {
  const { t } = useTranslation();
  const [accounts, setAccounts] = useState<ChatGptPlanAccount[]>([]);
  const visibleAccounts = useMemo(() => accounts.map((account) => withChatGptPlanMembership(account, memberAccountIds)), [accounts, memberAccountIds]);
  const filteredAccounts = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    return !query ? visibleAccounts : visibleAccounts.filter((account) =>
      [account.label, account.email, account.id].some((value) => value?.toLowerCase().includes(query)));
  }, [visibleAccounts, searchQuery]);
  const countRef = useRef(onCountChange);
  countRef.current = onCountChange;
  useEffect(() => { countRef.current?.(accounts.length); }, [accounts.length]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [dialog, setDialog] = useState<{ account?: ChatGptPlanAccount; expectedEmail?: string; discoveryError?: string } | null>(null);
  const generation = useRef(0);
  const loadingRef = useRef(false);
  const active = useRef(true);
  const queuedReload = useRef(false);
  const accountsRef = useRef(visibleAccounts);
  const requestRevision = useRef(0);
  accountsRef.current = visibleAccounts;
  const changedRef = useRef(onChanged);
  changedRef.current = onChanged;

  const reload = useCallback(async () => {
    if (loadingRef.current) { queuedReload.current = true; return; }
    loadingRef.current = true;
    const current = ++generation.current;
    setLoading(true);
    setError('');
    try {
      const next = await listChatGptPlanAccounts();
      if (active.current && current === generation.current) setAccounts(next);
    } catch (failure) {
      if (active.current && current === generation.current) setError(chatGptPlanErrorKey(failure));
    } finally {
      loadingRef.current = false;
      if (active.current && current === generation.current) setLoading(false);
      if (active.current && queuedReload.current) { queuedReload.current = false; void reload(); }
    }
  }, []);

  useEffect(() => {
    active.current = true;
    void reload();
    const open = (event: Event) => {
      const id = (event as CustomEvent<{ registrationId?: string }>).detail?.registrationId;
      requestRevision.current++;
      setDialog({ account: id ? accountsRef.current.find((item) => item.id === id) : undefined });
    };
    const openRequested = () => {
      const context = takePendingChatgptPlanAuthorization();
      if (!context) return;
      const request = ++requestRevision.current;
      // Show a usable dialog immediately; account discovery is not a prerequisite.
      setDialog({ expectedEmail: context.email });
      if (!context.email) return;
      void (async () => {
        try {
          const saved = accountsRef.current.length ? accountsRef.current : await listChatGptPlanAccounts();
          if (!active.current || request !== requestRevision.current) return;
          const matches = context.email ? saved.filter((item) => item.email?.toLowerCase() === context.email?.toLowerCase()) : [];
          setDialog({ account: matches.length === 1 ? matches[0] : undefined, expectedEmail: context.email });
        } catch (failure) {
          if (active.current && request === requestRevision.current) {
            setDialog((current) => current ? { ...current, discoveryError: chatGptPlanErrorKey(failure) } : current);
          }
        }
      })();
    };
    // Overview cards must not consume navigation requests intended for API Service.
    if (variant !== 'cards') {
      openRequested();
      window.addEventListener('chatgpt-plan-open', open);
      window.addEventListener(CHATGPT_PLAN_OPEN_REQUEST, openRequested);
    }
    const updated = () => { void reload(); };
    window.addEventListener('codex-local-access-state-updated', updated);
    window.addEventListener('chatgpt-plan-accounts-changed', updated);
    const unlisten = subscribeTauriEvent('codex-local-access-state-updated', updated);
    return () => {
      active.current = false;
      window.removeEventListener('chatgpt-plan-open', open);
      window.removeEventListener(CHATGPT_PLAN_OPEN_REQUEST, openRequested);
      window.removeEventListener('codex-local-access-state-updated', updated);
      window.removeEventListener('chatgpt-plan-accounts-changed', updated);
      unlisten();
    };
  }, [reload, variant]);

  const changed = () => {
    void reload();
    changedRef.current();
  };

  if (variant === 'controller') return dialog ? <ChatGptPlanDialog key={dialog.account?.id ?? 'new'} account={dialog.account} memberAccountIds={memberAccountIds} expectedEmail={dialog.expectedEmail} discoveryError={dialog.discoveryError}
    onLoginStart={() => { requestRevision.current++; setDialog((current) => current ? { ...current, discoveryError: undefined } : current); }}
    onClose={() => { requestRevision.current++; setDialog(null); }} onChanged={changed} /> : null;

  return (
    <section className={`codex-api-service-panel chatgpt-plan-section${variant === 'cards' ? ' chatgpt-plan-section--cards' : ''}`}
      hidden={variant === 'cards' && accounts.length === 0 && !loading && !error && !dialog}>
      <div className="codex-api-service-panel-head">
        <div>
          <h2>{t('chatgptPlan.title')}</h2>
          <p className="chatgpt-plan-description">{t('chatgptPlan.description')}</p>
        </div>
        <div className="chatgpt-plan-actions">
          <button type="button" className="btn btn-secondary btn-sm" disabled={loading} onClick={() => { changedRef.current(); void reload(); }}>
            <RefreshCw size={14} className={loading ? 'chatgpt-plan-spin' : ''} />{t('common.refresh')}
          </button>
          <button type="button" className="btn btn-primary btn-sm" onClick={() => { requestRevision.current++; setDialog({}); }}>
            <Plus size={14} />{t('chatgptPlan.addTab')}
          </button>
        </div>
      </div>
      <ModalErrorMessage message={!dialog && error ? t(error) : ''} />
      {accounts.length === 0 ? (
        <p className="chatgpt-plan-empty" aria-live="polite">{t(loading ? 'common.loading' : 'chatgptPlan.empty')}</p>
      ) : variant === 'cards' ? (
        <div className="chatgpt-plan-card-grid" aria-busy={loading}>
          {filteredAccounts.map((account) => <ChatGptPlanAccountCard key={account.id} account={account}
            maskAccountText={maskAccountText}
            stats={accountStats?.find((item) => item.accountId === `chatgpt_plan_${account.id}`)?.usage}
            onManage={() => { requestRevision.current++; setDialog({ account }); }} />)}
          {filteredAccounts.length === 0 && <p className="chatgpt-plan-empty">{t('chatgptPlan.card.searchEmpty')}</p>}
        </div>
      ) : (
        <ul className="chatgpt-plan-account-list" aria-busy={loading}>
          {visibleAccounts.map((account) => (
            <li key={account.id}>
              <div className="chatgpt-plan-account-name">
                <strong>{account.label || account.email}</strong>
                {account.email && account.email !== account.label ? <small>{account.email}</small> : null}
              </div>
              <span className={`chatgpt-plan-badge${account.in_service ? ' active' : ''}`}>
                {t(chatGptPlanStatusKey(account))}
              </span>
              <button type="button" className="btn btn-secondary btn-sm" onClick={() => { requestRevision.current++; setDialog({ account }); }}>
                <Settings2 size={14} />{t('chatgptPlan.manage')}
              </button>
            </li>
          ))}
        </ul>
      )}
      <p className="chatgpt-plan-description">{t('chatgptPlan.localStatsHint')}</p>
      {dialog ? <ChatGptPlanDialog key={dialog.account?.id ?? 'new'} account={dialog.account} memberAccountIds={memberAccountIds} expectedEmail={dialog.expectedEmail} discoveryError={dialog.discoveryError}
        onLoginStart={() => { requestRevision.current++; setDialog((current) => current ? { ...current, discoveryError: undefined } : current); }}
        onClose={() => { requestRevision.current++; setDialog(null); }} onChanged={changed} /> : null}
    </section>
  );
}
