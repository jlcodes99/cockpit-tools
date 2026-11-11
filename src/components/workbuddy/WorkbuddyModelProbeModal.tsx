/**
 * WorkBuddy 模型可用性批量探测弹窗
 *
 * 用法：WorkBuddy 账号页顶部「探测模型可用性」或卡片操作栏的准心按钮打开。
 * - 选账号范围：已勾选 / 当前账号 / 全部账号
 * - 多选要测的模型（来自云端实时目录，可搜索）
 * - 一键批量测试「账号 × 模型」矩阵，逐格显示 可用 / 被限流 / 异常
 * - 结果同时写入账号卡片标识（可点 × 关闭）
 * - 每个格子真实发送一句 "Reply with OK."，消耗极少额度；默认测试后销毁会话
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  X,
  Crosshair,
  Loader2,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  HelpCircle,
  Search,
  type LucideIcon,
} from 'lucide-react';
import type { WorkbuddyAccount } from '../../types/workbuddy';
import { getWorkbuddyAccountDisplayEmail } from '../../types/workbuddy';
import {
  fetchWorkbuddyCloudModels,
  probeWorkbuddyModel,
  type WorkbuddyCloudModel,
  type WorkbuddyModelProbeResult,
} from '../../services/workbuddyService';
import { WORKBUDDY_MODEL_GROUPS } from '../../utils/workbuddyModels';
import {
  clearWorkbuddyProbeRecord,
  parseProbeResetAt,
  setWorkbuddyProbeRecord,
} from '../../utils/workbuddyProbeStatus';

interface WorkbuddyModelProbeModalProps {
  accounts: WorkbuddyAccount[];
  currentAccountId?: string | null;
  /** 账号列表里已勾选的账号 id */
  selectedAccountIds?: string[];
  /** 打开时默认的账号范围（卡片快捷入口传 current） */
  initialScope?: Scope;
  /** 锁定单个账号（从卡片入口打开时使用，隐藏范围切换） */
  lockedAccountId?: string | null;
  onClose: () => void;
}

type CellStatus = 'pending' | 'running' | 'available' | 'rate_limited' | 'error' | 'unknown' | 'failed';

interface CellState {
  status: CellStatus;
  result?: WorkbuddyModelProbeResult;
  error?: string;
}

type Scope = 'selected' | 'current' | 'all';

const MODELS_STORAGE_KEY = 'agtools.workbuddy.probe_models.v1';
const CONCURRENCY = 3;

function cellKey(accountId: string, model: string) {
  return `${accountId}\u0000${model}`;
}

function loadSavedModels(): string[] {
  try {
    const raw = localStorage.getItem(MODELS_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
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

function statusMeta(status: CellStatus): { icon: LucideIcon; className: string; text: string } {
  switch (status) {
    case 'available':
      return { icon: CheckCircle2, className: 'ok', text: '可用' };
    case 'rate_limited':
      return { icon: XCircle, className: 'limited', text: '被限流' };
    case 'error':
    case 'failed':
      return { icon: AlertTriangle, className: 'error', text: '异常' };
    case 'running':
      return { icon: Loader2, className: 'running', text: '测试中' };
    default:
      return { icon: HelpCircle, className: 'pending', text: '待测' };
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
  initialScope,
  lockedAccountId,
  onClose,
}: WorkbuddyModelProbeModalProps) {
  const { t } = useTranslation();
  const [scope, setScope] = useState<Scope>(() => {
    if (initialScope) return initialScope;
    if (selectedAccountIds && selectedAccountIds.length > 0) return 'selected';
    if (currentAccountId && accounts.some((account) => account.id === currentAccountId)) {
      return 'current';
    }
    return 'all';
  });
  const [models, setModels] = useState<string[]>(loadSavedModels);
  const [modelQuery, setModelQuery] = useState('');
  const [cloudModels, setCloudModels] = useState<WorkbuddyCloudModel[]>([]);
  const [catalogError, setCatalogError] = useState<string | null>(null);
  const [keepConversation, setKeepConversation] = useState(false);
  const [cells, setCells] = useState<Record<string, CellState>>({});
  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState({ done: 0, total: 0 });
  const [activeCell, setActiveCell] = useState<string | null>(null);
  const runTokenRef = useRef(0);

  // 云端实时模型目录（失败回退内置目录）
  useEffect(() => {
    const anchorAccountId =
      (currentAccountId && accounts.some((account) => account.id === currentAccountId)
        ? currentAccountId
        : accounts[0]?.id) ?? '';
    if (!anchorAccountId) return;
    let cancelled = false;
    fetchWorkbuddyCloudModels(anchorAccountId)
      .then((list) => {
        if (!cancelled) setCloudModels(list);
      })
      .catch((error) => {
        if (!cancelled) setCatalogError(error instanceof Error ? error.message : String(error));
      });
    return () => {
      cancelled = true;
    };
    // 只需要首次打开时加载一次
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const catalog = useMemo<WorkbuddyCloudModel[]>(() => {
    if (cloudModels.length > 0) return cloudModels;
    return WORKBUDDY_MODEL_GROUPS.flatMap((group) =>
      group.options.map((option) => ({
        id: option.id,
        name: option.label,
        credits: null,
        description: null,
        supportsImages: false,
        isDefault: false,
      })),
    );
  }, [cloudModels]);

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

  const runBatch = async () => {
    if (!canRun) return;
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
          const result = await probeWorkbuddyModel(account.id, model, keepConversation);
          if (result.status === 'rate_limited' || result.status === 'error') {
            setWorkbuddyProbeRecord({
              accountId: account.id,
              model,
              status: result.status,
              message: result.message,
              resetAt: parseProbeResetAt(result.message),
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

    await Promise.all(
      Array.from({ length: Math.min(CONCURRENCY, pairs.length) }, () => worker()),
    );
    if (runTokenRef.current === token) setRunning(false);
  };

  const activeDetail = activeCell ? cells[activeCell] : undefined;
  const activePair = useMemo(() => {
    if (!activeCell) return null;
    const [accountId, model] = activeCell.split('\u0000');
    const account = accounts.find((item) => item.id === accountId);
    return { accountId, model, account };
  }, [activeCell, accounts]);

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div
        className="modal-content workbuddy-model-probe-modal"
        onClick={(event) => event.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div className="modal-header">
          <div className="modal-title">
            <Crosshair size={18} aria-hidden="true" />
            {lockedAccountId
              ? t('workbuddy.modelProbe.singleTitle', '探测该账号模型可用性')
              : t('workbuddy.modelProbe.batchTitle', '批量探测模型可用性')}
          </div>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close', '关闭')}>
            <X size={18} />
          </button>
        </div>

        <div className="modal-body workbuddy-model-probe-body">
          <p className="wb-probe-hint">
            {t(
              'workbuddy.modelProbe.batchHint',
              '对所选账号逐个模型真实发送一句 "Reply with OK."，读取模型实际回复；可用 / 被限流 / 异常会逐格标出，并同步到账号卡片。每次探测只消耗极少额度。',
            )}
          </p>

          {lockedAccountId ? (
            <div className="wb-probe-scope">
              <span className="wb-probe-label">{t('workbuddy.modelProbe.scope', '账号范围')}</span>
              <span className="wb-probe-scope-locked">
                {(() => {
                  const account = accounts.find((item) => item.id === lockedAccountId);
                  return account ? getWorkbuddyAccountDisplayEmail(account) : lockedAccountId;
                })()}
                <em>{t('workbuddy.modelProbe.singleAccount', '仅测该账号')}</em>
              </span>
            </div>
          ) : (
            <div className="wb-probe-scope">
              <span className="wb-probe-label">{t('workbuddy.modelProbe.scope', '账号范围')}</span>
              <div className="wb-probe-scope-options">
                <button
                  type="button"
                  className={`wb-probe-scope-btn${scope === 'selected' ? ' active' : ''}`}
                  onClick={() => setScope('selected')}
                  disabled={!selectedAccountIds || selectedAccountIds.length === 0}
                >
                  {t('workbuddy.modelProbe.scopeSelected', '已勾选（{{count}}）', {
                    count: selectedAccountIds?.length ?? 0,
                  })}
                </button>
                <button
                  type="button"
                  className={`wb-probe-scope-btn${scope === 'current' ? ' active' : ''}`}
                  onClick={() => setScope('current')}
                  disabled={!currentAccountId || !accounts.some((account) => account.id === currentAccountId)}
                >
                  {t('workbuddy.modelProbe.scopeCurrent', '当前账号')}
                </button>
                <button
                  type="button"
                  className={`wb-probe-scope-btn${scope === 'all' ? ' active' : ''}`}
                  onClick={() => setScope('all')}
                >
                  {t('workbuddy.modelProbe.scopeAll', '全部（{{count}}）', { count: accounts.length })}
                </button>
              </div>
              <span className="wb-probe-scope-summary">
                {t('workbuddy.modelProbe.scopeSummary', '将测试 {{accounts}} 个账号', {
                  accounts: scopedAccounts.length,
                })}
              </span>
            </div>
          )}

          <div className="wb-probe-models">
            <div className="wb-probe-models-head">
              <span className="wb-probe-label">
                {t('workbuddy.modelProbe.modelsLabel', '选择模型（{{count}}）', {
                  count: models.length,
                })}
              </span>
              <div className="wb-probe-models-actions">
                <span className="wb-probe-search">
                  <Search size={12} aria-hidden="true" />
                  <input
                    value={modelQuery}
                    onChange={(event) => setModelQuery(event.target.value)}
                    placeholder={t('workbuddy.modelProbe.searchModel', '搜索模型')}
                  />
                </span>
                <button
                  type="button"
                  className="btn btn-sm btn-secondary"
                  onClick={() => {
                    const next = Array.from(
                      new Set([...models, ...filteredCatalog.map((model) => model.id)]),
                    );
                    setModels(next);
                    saveModels(next);
                  }}
                >
                  {t('workbuddy.modelProbe.selectAll', '全选')}
                </button>
                <button
                  type="button"
                  className="btn btn-sm btn-secondary"
                  onClick={() => {
                    setModels([]);
                    saveModels([]);
                  }}
                >
                  {t('workbuddy.modelProbe.clearModels', '清空')}
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
                  {catalogError
                    ? t('workbuddy.modelProbe.catalogError', '模型目录获取失败，请稍后重试')
                    : t('workbuddy.modelProbe.noModels', '没有匹配的模型')}
                </span>
              )}
            </div>
          </div>

          <div className="wb-probe-run-row">
            <label className="wb-probe-checkbox">
              <input
                type="checkbox"
                checked={keepConversation}
                onChange={(event) => setKeepConversation(event.target.checked)}
                disabled={running}
              />
              <span>{t('workbuddy.modelProbe.keep', '保留测试会话（不自动销毁，便于之后复用）')}</span>
            </label>
            <button type="button" className="btn btn-primary" onClick={runBatch} disabled={!canRun}>
              {running && <Loader2 size={14} className="animate-spin" />}
              {running
                ? t('workbuddy.modelProbe.batchRunning', '测试中 {{done}}/{{total}}', progress)
                : t('workbuddy.modelProbe.batchRun', '开始测试（{{pairs}} 组）', { pairs: totalPairs })}
            </button>
          </div>

          {(Object.keys(cells).length > 0 || running) && (
            <div className="wb-probe-matrix-wrap">
              <div className="wb-probe-summary">
                <span className="wb-probe-summary-item ok">可用 {summary.available}</span>
                <span className="wb-probe-summary-item limited">限流 {summary.rate_limited}</span>
                <span className="wb-probe-summary-item error">异常 {summary.error}</span>
                <span className="wb-probe-summary-item unknown">未知 {summary.unknown}</span>
              </div>
              <div className="wb-probe-matrix-scroll">
                <table className="wb-probe-matrix">
                  <thead>
                    <tr>
                      <th>{t('workbuddy.modelProbe.matrixAccount', '账号')}</th>
                      {models.map((model) => (
                        <th key={model} title={model}>
                          {model}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {scopedAccounts.map((account) => (
                      <tr key={account.id}>
                        <td className="wb-probe-matrix-account">
                          <span title={getWorkbuddyAccountDisplayEmail(account)}>
                            {getWorkbuddyAccountDisplayEmail(account)}
                          </span>
                          {account.id === currentAccountId && (
                            <em className="wb-probe-matrix-current">
                              {t('workbuddy.modelProbe.currentTag', '当前')}
                            </em>
                          )}
                        </td>
                        {models.map((model) => {
                          const key = cellKey(account.id, model);
                          const cell = cells[key] ?? { status: 'pending' as CellStatus };
                          const meta = statusMeta(cell.status);
                          const Icon = meta.icon;
                          const resetAt = cell.result ? parseProbeResetAt(cell.result.message) : null;
                          return (
                            <td key={model}>
                              <button
                                type="button"
                                className={`wb-probe-cell ${meta.className}`}
                                onClick={() => setActiveCell(key)}
                                title={
                                  cell.result?.message ||
                                  cell.error ||
                                  t('workbuddy.modelProbe.cellPending', '点击查看详情')
                                }
                              >
                                <Icon
                                  size={13}
                                  className={cell.status === 'running' ? 'animate-spin' : ''}
                                  aria-hidden="true"
                                />
                                {resetAt && (
                                  <span className="wb-probe-cell-sub">
                                    {formatShortTime(resetAt)}
                                  </span>
                                )}
                              </button>
                            </td>
                          );
                        })}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {activePair && activeDetail && (
                <div className="wb-probe-detail">
                  <div className="wb-probe-detail-head">
                    <strong>{activePair.model}</strong>
                    <span>
                      {activePair.account
                        ? getWorkbuddyAccountDisplayEmail(activePair.account)
                        : activePair.accountId}
                    </span>
                    <button type="button" className="modal-close" onClick={() => setActiveCell(null)}>
                      <X size={14} />
                    </button>
                  </div>
                  <div className="wb-probe-detail-body">
                    {activeDetail.result ? (
                      <>
                        <p>{activeDetail.result.message}</p>
                        {activeDetail.result.reply && (
                          <div className="wb-probe-reply">
                            <span className="wb-probe-reply-label">
                              {t('workbuddy.modelProbe.replyLabel', '模型回复')}
                            </span>
                            <pre>{activeDetail.result.reply}</pre>
                          </div>
                        )}
                        {activeDetail.result.rawSnippet && (
                          <details>
                            <summary>{t('workbuddy.modelProbe.raw', '原始响应片段')}</summary>
                            <pre>
                              {activeDetail.result.rawSnippet.trim()
                                ? activeDetail.result.rawSnippet
                                : activeDetail.result.message}
                            </pre>
                          </details>
                        )}
                      </>
                    ) : (
                      <p>{activeDetail.error ?? t('workbuddy.modelProbe.cellPending', '点击查看详情')}</p>
                    )}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
