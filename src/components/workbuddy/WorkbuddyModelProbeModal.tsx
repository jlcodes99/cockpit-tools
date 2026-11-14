/**
 * WorkBuddy / CodeBuddy CN 模型可用性批量探测弹窗
 *
 * - 顶部入口：选账号范围（已勾选 / 当前 / 全部）× 多选模型 → 一键跑「账号 × 模型」矩阵
 * - 卡片入口：lockedAccountId 锁定该账号（只测它）
 * - 每个格子真实发送一句 "Reply with OK."，可用 / 被限流 / 异常逐格标出
 * - 结果同步到账号卡片标识（可点 × 关闭）；默认测试后销毁会话
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft, CheckCircle2, Crosshair, HelpCircle, Loader2, Search, X, XCircle, AlertTriangle, type LucideIcon } from 'lucide-react';
import { useTranslation } from 'react-i18next';
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

interface Props {
  accounts: WorkbuddyAccount[];
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
}

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
  useEscCloseTopmost(true, onClose, dialog);

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
  const [detailOpen, setDetailOpen] = useState(false);
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
      if (runTokenRef.current === token) setRunning(false);
    }
  };

  const activePair = useMemo(() => {
    if (!activeCell) return null;
    const [accountId, model] = activeCell.split('\u0000');
    const account = accounts.find((item) => item.id === accountId);
    return { accountId, model, account, cell: cells[activeCell] };
  }, [activeCell, accounts, cells]);

  return (
    <div className="modal-overlay">
      <div
        ref={dialog}
        className="modal-content workbuddy-model-probe-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="workbuddy-probe-title"
      >
        <div className="modal-header">
          <h3 id="workbuddy-probe-title">
            <Crosshair size={18} />
            {t('workbuddy.modelProbe.title')}
          </h3>
          <button type="button" className="modal-close" onClick={onClose} aria-label={t('common.close')}>
            <X size={18} />
          </button>
        </div>

        <div className="modal-body workbuddy-model-probe-body">
          <p className="wb-probe-hint">{t('workbuddy.modelProbe.hint')}</p>

          {lockedAccountId ? (
            <div className="wb-probe-field">
              <span>{t('workbuddy.modelProbe.account')}</span>
              <strong className="wb-probe-locked">
                {(() => {
                  const account = accounts.find((item) => item.id === lockedAccountId);
                  const badge = account ? planBadges.get(account.id) : undefined;
                  return (
                    <>
                      {account ? displayEmail(account) : lockedAccountId}
                      {badge && (
                        <span className={`tier-badge account-picker-plan ${badge.className}`}>{badge.label}</span>
                      )}
                    </>
                  );
                })()}
              </strong>
            </div>
          ) : (
            <div className="wb-probe-field">
              <span>{t('workbuddy.modelProbe.account')}</span>
              <div className="wb-probe-scope-options">
                <button
                  type="button"
                  className={`wb-probe-scope-btn${scope === 'all' ? ' active' : ''}`}
                  onClick={() => setScope('all')}
                >
                  {t('workbuddy.modelProbe.scopeAll', { count: accounts.length })}
                </button>
                <button
                  type="button"
                  className={`wb-probe-scope-btn${scope === 'selected' ? ' active' : ''}`}
                  onClick={() => setScope('selected')}
                  disabled={!selectedAccountIds || selectedAccountIds.length === 0}
                >
                  {t('workbuddy.modelProbe.scopeSelected', { count: selectedAccountIds?.length ?? 0 })}
                </button>
                <button
                  type="button"
                  className={`wb-probe-scope-btn${scope === 'current' ? ' active' : ''}`}
                  onClick={() => setScope('current')}
                  disabled={!currentAccountId || !accounts.some((account) => account.id === currentAccountId)}
                >
                  {t('workbuddy.modelProbe.scopeCurrent')}
                </button>
              </div>
            </div>
          )}

          <div className="wb-probe-field">
            <div className="wb-probe-models-head">
              <span>{t('workbuddy.modelProbe.model')}（{models.length}）</span>
              <div className="wb-probe-models-actions">
                <span className="wb-probe-search">
                  <Search size={12} aria-hidden="true" />
                  <input
                    value={modelQuery}
                    onChange={(event) => setModelQuery(event.target.value)}
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
                  <span className="wb-probe-model-name">
                    {model.name && model.name !== model.id ? model.name : model.id}
                    {model.name && model.name !== model.id && (
                      <em className="wb-probe-model-id">{model.id}</em>
                    )}
                  </span>
                  {model.credits && <span className="wb-probe-model-credits">{model.credits}</span>}
                </label>
              ))}
              {filteredCatalog.length === 0 && (
                <span className="wb-probe-model-empty">
                  {catalogState === 'loading'
                    ? t('workbuddy.modelProbe.catalog_loading')
                    : catalogState === 'error'
                      ? t('workbuddy.modelProbe.catalog_error')
                      : t('common.none')}
                </span>
              )}
            </div>
            <span className="wb-probe-cloud-note">{t(`workbuddy.modelProbe.catalog_${catalogState}`)}</span>
          </div>

          <label className="wb-probe-checkbox">
            <input
              type="checkbox"
              checked={keepConversation}
              disabled={running}
              onChange={(event) => setKeepConversation(event.target.checked)}
            />
            <span>{t('workbuddy.modelProbe.keep')}</span>
          </label>

          {(Object.keys(cells).length > 0 || running) && (
            <div className="wb-probe-matrix-wrap">
              <div className="wb-probe-summary">
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
                <table className="wb-probe-matrix">
                  <thead>
                    <tr>
                      <th>{t('workbuddy.modelProbe.account')}</th>
                      {models.map((model) => (
                        <th key={model} title={model}>{model}</th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {scopedAccounts.map((account) => (
                      <tr key={account.id}>
                        <td className="wb-probe-matrix-account">
                          <span title={displayEmail(account)}>{displayEmail(account)}</span>
                          {planBadges.get(account.id) && (
                            <span className={`tier-badge account-picker-plan ${planBadges.get(account.id)!.className}`}>
                              {planBadges.get(account.id)!.label}
                            </span>
                          )}
                          {account.id === currentAccountId && (
                            <em className="wb-probe-matrix-current">{t('accounts.status.current', '当前')}</em>
                          )}
                        </td>
                        {models.map((model) => {
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
                                onClick={() => { setActiveCell(key); setDetailOpen(true); }}
                                title={cell.result?.message || cell.error || t(`workbuddy.modelProbe.${meta.key}`)}
                              >
                                <Icon
                                  size={13}
                                  className={cell.status === 'running' ? 'animate-spin' : ''}
                                  aria-hidden="true"
                                />
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

          {detailOpen && activePair && (
            <div className="wb-probe-detail">
              <div className="wb-probe-detail-head">
                <button type="button" className="wb-probe-back" onClick={() => setDetailOpen(false)}>
                  <ArrowLeft size={14} />
                </button>
                <strong>{activePair.model}</strong>
                <span>{activePair.account ? displayEmail(activePair.account) : activePair.accountId}</span>
                <button type="button" className="modal-close" onClick={() => { setDetailOpen(false); setActiveCell(null); }}>
                  <X size={14} />
                </button>
              </div>
              <div className="wb-probe-detail-body">
                {activePair.cell?.result ? (
                  <>
                    <strong>{t(`workbuddy.modelProbe.status_${activePair.cell.result.status}`)}</strong>
                    {activePair.cell.result.message && <p>{diagnostic(activePair.cell.result.message)}</p>}
                    {activePair.cell.result.reply && (
                      <pre className="wb-probe-reply">{activePair.cell.result.reply}</pre>
                    )}
                    {activePair.cell.result.conversationId && (
                      <span>
                        {t(
                          activePair.cell.result.conversationDeleted
                            ? 'workbuddy.modelProbe.deleted'
                            : 'workbuddy.modelProbe.retained',
                        )}
                      </span>
                    )}
                    {activePair.cell.result.cleanupError && (
                      <span>
                        {t('workbuddy.modelProbe.cleanupFailed')} {diagnostic(activePair.cell.result.cleanupError)}
                      </span>
                    )}
                  </>
                ) : (
                  <p>{activePair.cell?.error ? diagnostic(activePair.cell.error) : t('workbuddy.modelProbe.status_unknown')}</p>
                )}
              </div>
            </div>
          )}
        </div>

        <div className="modal-footer">
          <button type="button" className="btn btn-secondary" onClick={onClose}>
            {t('common.close')}
          </button>
          <button type="button" className="btn btn-primary" disabled={!canRun} onClick={() => { void runBatch(); }}>
            {running && <Loader2 size={14} className="animate-spin" />}
            {running
              ? `${t('workbuddy.modelProbe.running')} ${progress.done}/${progress.total}`
              : `${t('workbuddy.modelProbe.run')}（${totalPairs}）`}
          </button>
        </div>
      </div>
    </div>
  );
}
