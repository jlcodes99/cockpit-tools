/**
 * WorkBuddy / CodeBuddy CN 模型可用性批量探测弹窗
 *
 * - 顶部入口：选账号范围（已勾选 / 当前 / 全部）× 多选模型 → 一键跑「账号 × 模型」矩阵
 * - 卡片入口：lockedAccountId 锁定该账号（只测它）
 * - 每个格子真实发送一句 "Reply with OK."，可用 / 被限流 / 异常逐格标出
 * - 结果同步到账号卡片标识（可点 × 关闭）；默认测试后销毁会话
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft, ArrowUpRight, CheckCircle2, Crosshair, HelpCircle, Loader2, Search, X, XCircle, AlertTriangle, type LucideIcon } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { CodebuddyAccount } from '../../types/codebuddy';
import type { WorkbuddyAccount } from '../../types/workbuddy';
import { getWorkbuddyAccountDisplayEmail } from '../../types/workbuddy';
import { buildCodebuddyAccountPresentation, buildWorkbuddyAccountPresentation } from '../../presentation/platformAccountPresentation';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import {
  fetchCodebuddyCnCloudModels,
  fetchWorkbuddyCloudModels,
  probeCodebuddyCnModel,
  probeWorkbuddyModel,
  type WorkbuddyCloudModel,
  type WorkbuddyModelProbeResult,
} from '../../services/workbuddyService';
import {
  clearWorkbuddyProbeRecord,
  parseProbeResetAt,
  setWorkbuddyProbeRecord,
  type WorkbuddyProbePlatform,
} from '../../utils/workbuddyProbeStatus';

type Scope = 'all' | 'selected' | 'current';
type CellStatus = 'pending' | 'running' | 'available' | 'rate_limited' | 'error' | 'unknown' | 'failed';

interface CellState {
  status: CellStatus;
  result?: WorkbuddyModelProbeResult;
  error?: string;
}

type Props = ({ platform?: 'workbuddy'; accounts: WorkbuddyAccount[] }
  | { platform: 'codebuddy_cn'; accounts: CodebuddyAccount[] }) & {
  currentAccountId?: string | null;
  /** 账号列表里已勾选的账号 id */
  selectedAccountIds?: string[];
  /** 从卡片入口打开时锁定该账号（隐藏范围切换） */
  lockedAccountId?: string | null;
  /** 打开时默认范围（卡片入口传 current） */
  initialScope?: Scope;
  /** 平台：CodeBuddy CN 与 WorkBuddy 同构（额度共享） */
  platform?: WorkbuddyProbePlatform;
  /** 账号显示名（不同平台取法不同） */
  displayEmail?: (account: WorkbuddyAccount) => string;
  onClose: () => void;
};

const MODELS_STORAGE_KEY = 'agtools.workbuddy.probe_models.v1';
const CONCURRENCY = 3;

function cellKey(accountId: string, model: string) {
  return `${accountId}\u0000${model}`;
}

function loadSavedModels(): string[] {
  try {
    const raw = localStorage.getItem(MODELS_STORAGE_KEY);
    const parsed = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.filter((value) => typeof value === 'string') : [];
  } catch {
    return [];
  }
}

function saveModels(models: string[]) {
  try {
    localStorage.setItem(MODELS_STORAGE_KEY, JSON.stringify(models));
  } catch {
    /* ignore */
  }
}

function statusMeta(status: CellStatus): { icon: LucideIcon; className: string; key: string } {
  switch (status) {
    case 'available':
      return { icon: CheckCircle2, className: 'ok', key: 'status_available' };
    case 'rate_limited':
      return { icon: XCircle, className: 'limited', key: 'status_rate_limited' };
    case 'error':
    case 'failed':
      return { icon: AlertTriangle, className: 'error', key: 'status_error' };
    case 'running':
      return { icon: Loader2, className: 'running', key: 'running' };
    default:
      return { icon: HelpCircle, className: 'pending', key: 'status_unknown' };
  }
}

function formatShortTime(timestamp: number): string {
  const date = new Date(timestamp);
  const pad = (value: number) => String(value).padStart(2, '0');
  return `${pad(date.getMonth() + 1)}/${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export function WorkbuddyModelProbeModal({
  accounts,
  currentAccountId,
  selectedAccountIds,
  lockedAccountId,
  initialScope,
  platform = 'workbuddy',
  displayEmail = getWorkbuddyAccountDisplayEmail,
  onClose,
}: Props) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);

  const [scope, setScope] = useState<Scope>(() => {
    if (initialScope) return initialScope;
    if (selectedAccountIds && selectedAccountIds.length > 0) return 'selected';
    return 'all';
  });
  const [models, setModels] = useState<string[]>(loadSavedModels);
  const [modelQuery, setModelQuery] = useState('');
  const [cloudModels, setCloudModels] = useState<WorkbuddyCloudModel[]>([]);
  const [catalogState, setCatalogState] = useState<'loading' | 'loaded' | 'error'>('loading');
  const [keepConversation, setKeepConversation] = useState(false);
  const [cells, setCells] = useState<Record<string, CellState>>({});
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState({ done: 0, total: 0 });
  const [activeCell, setActiveCell] = useState<string | null>(null);
  const [showResults, setShowResults] = useState(false);
  const [batch, setBatch] = useState<{ accounts: WorkbuddyAccount[]; models: string[] } | null>(null);
  const returnToSetup = () => {
    setActiveCell(null);
    setShowResults(false);
  };
  useEscCloseTopmost(true, () => {
    if (!showResults || running) onClose();
    else if (activeCell) setActiveCell(null);
    else returnToSetup();
  }, dialog);
  const runTokenRef = useRef(0);
  const inFlightRef = useRef(false);

  const probeModelFn = platform === 'codebuddy_cn' ? probeCodebuddyCnModel : probeWorkbuddyModel;
  const fetchModelsFn = platform === 'codebuddy_cn' ? fetchCodebuddyCnCloudModels : fetchWorkbuddyCloudModels;

  // 账号套餐标识（与账号总览一致；探测矩阵账号列与锁定账号展示用）
  const planBadges = useMemo(() => {
    const map = new Map<string, { label: string; className: string }>();
    for (const account of accounts) {
      const { planLabel, planClass } = platform === 'codebuddy_cn'
        ? buildCodebuddyAccountPresentation(account, t)
        : buildWorkbuddyAccountPresentation(account, t);
      if (planLabel) map.set(account.id, { label: planLabel, className: planClass });
    }
    return map;
  }, [accounts, platform, t]);

  // 模型目录：用范围内第一个账号拉取（额度共享，同一套目录）
  useEffect(() => {
    const anchorId =
      lockedAccountId ??
      (currentAccountId && accounts.some((account) => account.id === currentAccountId)
        ? currentAccountId
        : accounts[0]?.id);
    if (!anchorId) {
      setCatalogState('error');
      return;
    }
    let cancelled = false;
    setCatalogState('loading');
    void fetchModelsFn(anchorId)
      .then((list) => {
        if (cancelled) return;
        setCloudModels(list);
        setCatalogState('loaded');
      })
      .catch(() => {
        if (!cancelled) setCatalogState('error');
      });
    return () => {
      cancelled = true;
    };
    // 只在首次打开时加载
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const catalog = useMemo<WorkbuddyCloudModel[]>(
    () =>
      cloudModels.length > 0
        ? cloudModels
        : [],
    [cloudModels],
  );

  const filteredCatalog = useMemo(() => {
    const query = modelQuery.trim().toLowerCase();
    if (!query) return catalog;
    return catalog.filter(
      (model) =>
        model.id.toLowerCase().includes(query) ||
        (model.name ?? '').toLowerCase().includes(query),
    );
  }, [catalog, modelQuery]);

  const scopedAccounts = useMemo<WorkbuddyAccount[]>(() => {
    if (lockedAccountId) {
      return accounts.filter((account) => account.id === lockedAccountId);
    }
    if (scope === 'selected') {
      const ids = new Set(selectedAccountIds ?? []);
      return accounts.filter((account) => ids.has(account.id));
    }
    if (scope === 'current') {
      return accounts.filter((account) => account.id === currentAccountId);
    }
    return accounts;
  }, [scope, accounts, selectedAccountIds, currentAccountId, lockedAccountId]);

  const selectedSet = useMemo(() => new Set(models), [models]);

  const toggleModel = (modelId: string) => {
    setModels((previous) => {
      const next = previous.includes(modelId)
        ? previous.filter((id) => id !== modelId)
        : [...previous, modelId];
      saveModels(next);
      return next;
    });
  };

  const totalPairs = scopedAccounts.length * models.length;
  const canRun = !running && scopedAccounts.length > 0 && models.length > 0;

  const summary = useMemo(() => {
    const counts = { available: 0, rate_limited: 0, error: 0, unknown: 0 };
    for (const cell of Object.values(cells)) {
      if (cell.status === 'available') counts.available += 1;
      else if (cell.status === 'rate_limited') counts.rate_limited += 1;
      else if (cell.status === 'error' || cell.status === 'failed') counts.error += 1;
      else if (cell.status === 'unknown') counts.unknown += 1;
    }
    return counts;
  }, [cells]);

  const diagnostic = (message: string) => {
    if (message.startsWith('WORKBUDDY_PROBE_')) {
      return t('workbuddy.modelProbe.protocolFailure') + ' (' + message + ')';
    }
    return message;
  };

  const runBatch = async () => {
    // setRunning 是异步的，同一 tick 内连点两次会重复提交，这里用同步标志防重
    if (!canRun || inFlightRef.current) return;
    inFlightRef.current = true;
    const token = runTokenRef.current + 1;
    runTokenRef.current = token;
    const pairs = scopedAccounts.flatMap((account) =>
      models.map((model) => ({ account, model })),
    );
    const initial: Record<string, CellState> = {};
    for (const pair of pairs) initial[cellKey(pair.account.id, pair.model)] = { status: 'pending' };
    setBatch({ accounts: scopedAccounts, models });
    setActiveCell(null);
    setShowResults(true);
    setCells(initial);
    setProgress({ done: 0, total: pairs.length });
    setRunning(true);

    let cursor = 0;
    let done = 0;
    const worker = async () => {
      while (cursor < pairs.length && runTokenRef.current === token) {
        const index = cursor;
        cursor += 1;
        const { account, model } = pairs[index];
        const key = cellKey(account.id, model);
        setCells((previous) => ({ ...previous, [key]: { status: 'running' } }));
        try {
          const result = await probeModelFn(account.id, model, keepConversation);
          if (result.status === 'rate_limited' || result.status === 'error') {
            setWorkbuddyProbeRecord({
              accountId: account.id,
              model,
              status: result.status,
              message: result.message,
              resetAt: result.status === 'rate_limited' ? parseProbeResetAt(result.message) : null,
              checkedAt: Date.now(),
            });
          } else if (result.status === 'available') {
            clearWorkbuddyProbeRecord(account.id, model);
          }
          if (runTokenRef.current === token) {
            setCells((previous) => ({
              ...previous,
              [key]: { status: result.status as CellStatus, result },
            }));
          }
        } catch (error) {
          if (runTokenRef.current === token) {
            setCells((previous) => ({
              ...previous,
              [key]: {
                status: 'failed',
                error: error instanceof Error ? error.message : String(error),
              },
            }));
          }
        }
        done += 1;
        if (runTokenRef.current === token) setProgress({ done, total: pairs.length });
      }
    };

    try {
      await Promise.all(
        Array.from({ length: Math.min(CONCURRENCY, pairs.length) }, () => worker()),
      );
    } finally {
      inFlightRef.current = false;
      if (runTokenRef.current === token) {
        setRunning(false);
        const [pair] = pairs;
        if (pairs.length === 1 && pair) setActiveCell(cellKey(pair.account.id, pair.model));
      }
    }
  };

  const activePair = useMemo(() => {
    if (!activeCell) return null;
    const [accountId, model] = activeCell.split('\u0000');
    const account = batch?.accounts.find((item) => item.id === accountId);
    return { accountId, model, account, cell: cells[activeCell] };
  }, [activeCell, batch, cells]);
  const activeStatus = activePair ? statusMeta(activePair.cell?.status ?? 'pending') : null;
  const ActiveIcon = activeStatus?.icon;

  return (
    <div className="modal-overlay wb-probe-overlay">
      <div
        ref={dialog}
        className="modal-content workbuddy-model-probe-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="workbuddy-probe-title"
      >
        <div className="modal-header">
          <div className="wb-probe-heading">
            <span className="wb-probe-title-icon"><Crosshair size={20} aria-hidden="true" /></span>
            <div className="wb-probe-heading-copy">
              <h2 id="workbuddy-probe-title">
                {showResults ? t('workbuddy.modelProbe.results') : t('workbuddy.modelProbe.title')}
              </h2>
              {!showResults && <p className="wb-probe-hint">{t('workbuddy.modelProbe.hint')}</p>}
            </div>
          </div>
          <button type="button" className="modal-close" onClick={onClose} aria-label={t('common.close')}>
            <X size={18} />
          </button>
        </div>

        <div className="modal-body workbuddy-model-probe-body">
          {showResults && batch ? (
            <div className="wb-probe-results">
              {(activePair || running) && <div className="wb-probe-results-head">
                {activePair && (
                  <button type="button" className="wb-probe-quiet" onClick={() => setActiveCell(null)}>
                    <ArrowLeft size={15} aria-hidden="true" />
                    {t('common.back')}
                  </button>
                )}
                {running && <span role="status">{t('workbuddy.modelProbe.running')} {progress.done}/{progress.total}</span>}
              </div>}
              {activePair && activeStatus && ActiveIcon ? (
                <section className="wb-probe-detail" aria-labelledby="workbuddy-probe-result-model">
                  <div className="wb-probe-detail-head">
                    <div className="wb-probe-detail-identity">
                      <h3 id="workbuddy-probe-result-model">{activePair.model}</h3>
                      <span>{activePair.account ? displayEmail(activePair.account) : activePair.accountId}</span>
                    </div>
                    <span className={`wb-probe-status ${activeStatus.className}`}>
                      <ActiveIcon size={15} aria-hidden="true" />
                      {t(`workbuddy.modelProbe.${activeStatus.key}`)}
                    </span>
                  </div>
                  {activePair.cell?.result?.message && <p>{diagnostic(activePair.cell.result.message)}</p>}
                  {activePair.cell?.error && <p>{diagnostic(activePair.cell.error)}</p>}
                  {activePair.cell?.result?.reply && (
                    <div className="wb-probe-response">
                      <h4>{t('workbuddy.modelProbe.reply', { defaultValue: 'Model reply' })}</h4>
                      <pre className="wb-probe-reply">{activePair.cell.result.reply}</pre>
                    </div>
                  )}
                  {(activePair.cell?.result?.conversationId || activePair.cell?.result?.cleanupError) && (
                    <div className="wb-probe-session-meta">
                      {activePair.cell.result.conversationId && (
                        <span>
                          {t(activePair.cell.result.conversationDeleted
                            ? 'workbuddy.modelProbe.deleted'
                            : 'workbuddy.modelProbe.retained')}
                        </span>
                      )}
                      {activePair.cell.result.cleanupError && (
                        <span>
                          {t('workbuddy.modelProbe.cleanupFailed')} {diagnostic(activePair.cell.result.cleanupError)}
                        </span>
                      )}
                    </div>
                  )}
                </section>
              ) : (
                <div className="wb-probe-matrix-wrap">
                  <div className="wb-probe-summary" role="status">
                    <span className="wb-probe-summary-item ok">
                      {t('workbuddy.modelProbe.status_available')} {summary.available}
                    </span>
                    <span className="wb-probe-summary-item limited">
                      {t('workbuddy.modelProbe.status_rate_limited')} {summary.rate_limited}
                    </span>
                    <span className="wb-probe-summary-item error">
                      {t('workbuddy.modelProbe.status_error')} {summary.error}
                    </span>
                    <span className="wb-probe-summary-item unknown">
                      {t('workbuddy.modelProbe.status_unknown')} {summary.unknown}
                    </span>
                  </div>
                  <div className="wb-probe-matrix-scroll">
                    <table className="wb-probe-matrix" aria-label={t('workbuddy.modelProbe.results', { defaultValue: 'Test results' })}>
                      <thead>
                        <tr>
                          <th scope="col">{t('workbuddy.modelProbe.account')}</th>
                          {batch.models.map((model) => (
                            <th scope="col" key={model} title={model}>{model}</th>
                          ))}
                        </tr>
                      </thead>
                      <tbody>
                        {batch.accounts.map((account) => (
                          <tr key={account.id}>
                            <th scope="row" className="wb-probe-matrix-account">
                              <div className="wb-probe-matrix-account-line">
                                <span className="wb-probe-account-name" title={displayEmail(account)}>{displayEmail(account)}</span>
                                {planBadges.get(account.id) && (
                                  <span className={`tier-badge account-picker-plan ${planBadges.get(account.id)!.className}`}>
                                    {planBadges.get(account.id)!.label}
                                  </span>
                                )}
                                {account.id === currentAccountId && (
                                  <em className="wb-probe-matrix-current">{t('accounts.status.current', '当前')}</em>
                                )}
                              </div>
                            </th>
                            {batch.models.map((model) => {
                              const key = cellKey(account.id, model);
                              const cell = cells[key] ?? { status: 'pending' as CellStatus };
                              const meta = statusMeta(cell.status);
                              const Icon = meta.icon;
                              const resetAt = cell.result?.status === 'rate_limited'
                                ? parseProbeResetAt(cell.result.message)
                                : null;
                              return (
                                <td key={model}>
                                  <button
                                    type="button"
                                    className={`wb-probe-cell ${meta.className}`}
                                    disabled={cell.status === 'pending' || cell.status === 'running'}
                                    onClick={() => setActiveCell(key)}
                                    title={cell.result?.message || cell.error || t(`workbuddy.modelProbe.${meta.key}`)}
                                  >
                                    <Icon
                                      size={14}
                                      className={cell.status === 'running' ? 'animate-spin' : ''}
                                      aria-hidden="true"
                                    />
                                    <span>{t(`workbuddy.modelProbe.${meta.key}`)}</span>
                                    {resetAt && <span className="wb-probe-cell-sub">{formatShortTime(resetAt)}</span>}
                                  </button>
                                </td>
                              );
                            })}
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              )}
            </div>
          ) : (
            <div className="wb-probe-setup">
              <div className="wb-probe-account-pane">
                <h3 className="wb-probe-pane-heading">{t('workbuddy.modelProbe.account')}</h3>
                <div className="wb-probe-account-controls">
                  {lockedAccountId ? (
                    <strong className="wb-probe-locked">
                      {(() => {
                        const account = accounts.find((item) => item.id === lockedAccountId);
                        const badge = account ? planBadges.get(account.id) : undefined;
                        const label = account ? displayEmail(account) : lockedAccountId;
                        return (
                          <>
                            <span className="wb-probe-account-name" title={label}>{label}</span>
                            {badge && (
                              <span className={`tier-badge account-picker-plan ${badge.className}`}>{badge.label}</span>
                            )}
                          </>
                        );
                      })()}
                    </strong>
                  ) : (
                    <div className="wb-probe-scope-options">
                      <button
                        type="button"
                        className={`wb-probe-scope-btn${scope === 'all' ? ' active' : ''}`}
                        aria-pressed={scope === 'all'}
                        onClick={() => setScope('all')}
                      >
                        {t('workbuddy.modelProbe.scopeAll', { count: accounts.length })}
                      </button>
                      <button
                        type="button"
                        className={`wb-probe-scope-btn${scope === 'selected' ? ' active' : ''}`}
                        aria-pressed={scope === 'selected'}
                        onClick={() => setScope('selected')}
                        disabled={!selectedAccountIds || selectedAccountIds.length === 0}
                      >
                        {t('workbuddy.modelProbe.scopeSelected', { count: selectedAccountIds?.length ?? 0 })}
                      </button>
                      <button
                        type="button"
                        className={`wb-probe-scope-btn${scope === 'current' ? ' active' : ''}`}
                        aria-pressed={scope === 'current'}
                        onClick={() => setScope('current')}
                        disabled={!currentAccountId || !accounts.some((account) => account.id === currentAccountId)}
                      >
                        {t('workbuddy.modelProbe.scopeCurrent')}
                      </button>
                    </div>
                  )}

                  {batch && (
                    <button type="button" className="wb-probe-quiet wb-probe-open-results" onClick={() => setShowResults(true)}>
                      {t('workbuddy.modelProbe.results', { defaultValue: 'Test results' })}
                      <ArrowUpRight size={15} aria-hidden="true" />
                    </button>
                  )}
                </div>
              </div>

              <div className="wb-probe-model-pane">
                <div className="wb-probe-models-head">
                  <h3 className="wb-probe-pane-heading wb-probe-model-heading">
                    {t('workbuddy.modelProbe.model')}
                    <span className="wb-probe-selection-count">{models.length}</span>
                  </h3>
                  <div className="wb-probe-models-actions">
                    <span className="wb-probe-search">
                      <Search size={12} aria-hidden="true" />
                      <input
                        value={modelQuery}
                        onChange={(event) => setModelQuery(event.target.value)}
                        aria-label={t('common.search')}
                        placeholder={t('common.search')}
                      />
                    </span>
                    <button
                      type="button"
                      className="btn btn-sm btn-secondary"
                      onClick={() => {
                        const next = Array.from(new Set([...models, ...filteredCatalog.map((model) => model.id)]));
                        setModels(next);
                        saveModels(next);
                      }}
                    >
                      {t('common.selectAll')}
                    </button>
                    <button
                      type="button"
                      className="btn btn-sm btn-secondary"
                      onClick={() => {
                        setModels([]);
                        saveModels([]);
                      }}
                    >
                      {t('common.clear')}
                    </button>
                  </div>
                </div>
                <div className="wb-probe-model-list">
                  {filteredCatalog.map((model) => (
                    <label key={model.id} className="wb-probe-model-item">
                      <input
                        type="checkbox"
                        checked={selectedSet.has(model.id)}
                        onChange={() => toggleModel(model.id)}
                      />
                      <span className="wb-probe-model-label">
                        <span className="wb-probe-model-name" title={model.name || model.id}>
                          {model.name || model.id}
                        </span>
                        {model.name && model.name !== model.id && (
                          <span className="wb-probe-model-id" title={model.id}>{model.id}</span>
                        )}
                      </span>
                      <span className="wb-probe-model-credits">{model.credits}</span>
                    </label>
                  ))}
                  {filteredCatalog.length === 0 && (
                    <span className="wb-probe-model-empty">
                      {catalogState === 'loading'
                        ? t('workbuddy.modelProbe.catalog_loading')
                        : catalogState === 'error'
                          ? t('workbuddy.modelProbe.catalogUnavailable', { defaultValue: 'Could not load models.' })
                          : t('common.none')}
                    </span>
                  )}
                </div>
              </div>

              <div className="wb-probe-options">
                <label className="wb-probe-checkbox">
                  <input type="checkbox" checked={keepConversation} disabled={running}
                    onChange={(event) => setKeepConversation(event.target.checked)} />
                  <span>{t('workbuddy.modelProbe.keep')}</span>
                </label>
              </div>
            </div>
          )}
        </div>

        <div className="modal-footer">
          {showResults ? (
            <>
              <button type="button" className="btn btn-secondary" disabled={running} onClick={returnToSetup}>
                <ArrowLeft size={15} aria-hidden="true" />
                {t('workbuddy.modelProbe.backToSetup', { defaultValue: 'Back to setup' })}
              </button>
              <button type="button" className="btn btn-primary" onClick={onClose}>{t('common.close')}</button>
            </>
          ) : (
            <>
              <button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.close')}</button>
              <button type="button" className="btn btn-primary" disabled={!canRun} onClick={() => { void runBatch(); }}>
                {t('workbuddy.modelProbe.run')}（{totalPairs}）
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
