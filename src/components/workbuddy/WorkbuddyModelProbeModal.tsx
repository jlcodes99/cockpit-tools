/**
 * WorkBuddy 模型可用性探测弹窗
 *
 * 用法：在 WorkBuddy 账号页点击「探测模型可用性」打开。
 * - 选择一个账号 + 输入/选择要测的模型
 * - 点击「测试」会向该账号的对话接口发送一条极短消息（如 "hi"）
 * - 根据返回判定：可用 / 被限流 / 请求异常
 * - 默认测试后销毁测试会话（可在「保留会话」里改）
 */

import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  X,
  Crosshair,
  Loader2,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  HelpCircle,
  Trash2,
  type LucideIcon,
} from 'lucide-react';
import type { WorkbuddyAccount } from '../../types/workbuddy';
import { getWorkbuddyAccountDisplayEmail } from '../../types/workbuddy';
import {
  fetchWorkbuddyCloudModels,
  getWorkbuddyRecentCreditUsage,
  probeWorkbuddyModel,
  type WorkbuddyCloudModel,
  type WorkbuddyModelProbeResult,
} from '../../services/workbuddyService';
import { WORKBUDDY_MODEL_GROUPS, type WorkbuddyModelOption } from '../../utils/workbuddyModels';

interface WorkbuddyModelProbeModalProps {
  accounts: WorkbuddyAccount[];
  currentAccountId?: string | null;
  onClose: () => void;
}

type ProbeState =
  | { kind: 'idle' }
  | { kind: 'running' }
  | { kind: 'done'; result: WorkbuddyModelProbeResult }
  | { kind: 'failed'; error: string };

function statusBadge(status: WorkbuddyModelProbeResult['status']): {
  icon: LucideIcon;
  className: string;
  text: string;
} {
  switch (status) {
    case 'available':
      return { icon: CheckCircle2, className: 'wb-probe-badge wb-probe-ok', text: '可用' };
    case 'rate_limited':
      return { icon: XCircle, className: 'wb-probe-badge wb-probe-limited', text: '被限流' };
    case 'error':
      return { icon: AlertTriangle, className: 'wb-probe-badge wb-probe-error', text: '请求异常' };
    default:
      return { icon: HelpCircle, className: 'wb-probe-badge wb-probe-unknown', text: '未知' };
  }
}

export function WorkbuddyModelProbeModal({ accounts, currentAccountId, onClose }: WorkbuddyModelProbeModalProps) {
  const { t } = useTranslation();
  const [accountId, setAccountId] = useState<string>(
    currentAccountId && accounts.some((a) => a.id === currentAccountId)
      ? currentAccountId
      : accounts[0]?.id ?? '',
  );
  const [model, setModel] = useState<string>('');
  const [keepConversation, setKeepConversation] = useState<boolean>(false);
  const [state, setState] = useState<ProbeState>({ kind: 'idle' });
  const [recentModels, setRecentModels] = useState<string[]>([]);
  const [cloudModels, setCloudModels] = useState<WorkbuddyCloudModel[]>([]);
  const [cloudStatus, setCloudStatus] = useState<'loading' | 'loaded' | 'empty' | 'error'>('loading');

  // 云端模型目录（企业账号）：优先云端，失败/无企业则回退内置目录。
  useEffect(() => {
    let cancelled = false;
    setCloudModels([]);
    if (!accountId) return;
    setCloudStatus('loading');
    fetchWorkbuddyCloudModels(accountId)
      .then((rows) => {
        if (cancelled) return;
        setCloudModels(rows);
        setCloudStatus(rows.length > 0 ? 'loaded' : 'empty');
      })
      .catch(() => {
        if (!cancelled) setCloudStatus('error');
      });
    return () => {
      cancelled = true;
    };
  }, [accountId]);

  // 该账号最近实际调用过的模型（credits 用量接口返回的真实模型 ID），作为候选并入目录。
  useEffect(() => {
    let cancelled = false;
    setRecentModels([]);
    if (!accountId) return;
    getWorkbuddyRecentCreditUsage(accountId)
      .then((rows) => {
        if (cancelled) return;
        const ids = Array.from(
          new Set(rows.map((r) => r.model).filter((m): m is string => typeof m === 'string' && m.length > 0)),
        );
        setRecentModels(ids.slice(0, 12));
      })
      .catch(() => {
        /* 拉不到最近用量就只用内置目录，不打扰用户 */
      });
    return () => {
      cancelled = true;
    };
  }, [accountId]);

  const datalistOptions = useMemo<WorkbuddyModelOption[]>(() => {
    // 云端实时列表（客户端对话选择器同源）置顶，带计费倍率
    const cloud = cloudModels.map((m) => ({
      id: m.id,
      label: `${m.name ?? m.id}${m.credits ? ` · ${m.credits}` : ''}`,
    }));
    const cloudIds = new Set(cloud.map((m) => m.id));
    const recent = recentModels
      .filter((id) => !cloudIds.has(id))
      .map((id) => ({ id, label: t('workbuddy.modelProbe.recentSuffix', '{{id}}（最近使用）', { id }) }));
    const recentIds = new Set(recent.map((m) => m.id));
    const builtin = WORKBUDDY_MODEL_GROUPS.flatMap((g) => g.options).filter(
      (m) => !cloudIds.has(m.id) && !recentIds.has(m.id),
    );
    return [...cloud, ...recent, ...builtin];
  }, [cloudModels, recentModels, t]);

  const canRun = useMemo(
    () => accountId.length > 0 && model.trim().length > 0 && state.kind !== 'running',
    [accountId, model, state.kind],
  );

  const runProbe = async () => {
    if (!canRun) return;
    setState({ kind: 'running' });
    try {
      const result = await probeWorkbuddyModel(accountId, model.trim(), keepConversation);
      setState({ kind: 'done', result });
    } catch (err) {
      setState({ kind: 'failed', error: err instanceof Error ? err.message : String(err) });
    }
  };

  const badge = state.kind === 'done' ? statusBadge(state.result.status) : null;
  const BadgeIcon = badge?.icon;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div
        className="modal-content workbuddy-model-probe-modal"
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div className="modal-header">
          <div className="modal-title">
            <Crosshair size={18} aria-hidden="true" />
            {t('workbuddy.modelProbe.title', '探测模型可用性')}
          </div>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close', '关闭')}>
            <X size={18} />
          </button>
        </div>

        <div className="modal-body workbuddy-model-probe-body">
          <p className="wb-probe-hint">
            {t(
              'workbuddy.modelProbe.hint',
              '向所选账号真实发送一条极短消息（"Reply with OK."），读取模型的实际回复内容；创建会话成功但收不到回复或被限流都会如实标出。',
            )}
          </p>

          <label className="wb-probe-field">
            <span className="wb-probe-label">{t('workbuddy.modelProbe.account', '账号')}</span>
            <select
              className="wb-probe-select"
              value={accountId}
              onChange={(e) => setAccountId(e.target.value)}
              disabled={state.kind === 'running'}
            >
              {accounts.map((a) => (
                <option key={a.id} value={a.id}>
                  {getWorkbuddyAccountDisplayEmail(a)}
                </option>
              ))}
            </select>
          </label>

          <label className="wb-probe-field">
            <span className="wb-probe-label">{t('workbuddy.modelProbe.model', '模型')}</span>
            <select
              className="wb-probe-select"
              value=""
              onChange={(e) => {
                if (e.target.value) setModel(e.target.value);
              }}
              disabled={state.kind === 'running'}
            >
              <option value="">{t('workbuddy.modelProbe.pickFromCatalog', '— 从模型目录选择 —')}</option>
              {cloudModels.length > 0 && (
                <optgroup label={t('workbuddy.modelProbe.pickerGroup', '对话选择器（云端实时）')}>
                  {cloudModels.map((m) => (
                    <option key={`cloud-${m.id}`} value={m.id}>
                      {m.name ?? m.id}
                      {m.credits ? ` · ${m.credits}` : ''}
                      {m.isDefault ? t('workbuddy.modelProbe.defaultSuffix', '（默认）') : ''}
                    </option>
                  ))}
                </optgroup>
              )}
              {recentModels.length > 0 && (
                <optgroup label={t('workbuddy.modelProbe.recentGroup', '最近使用')}>
                  {recentModels.map((id) => (
                    <option key={`recent-${id}`} value={id}>
                      {id}
                    </option>
                  ))}
                </optgroup>
              )}
            </select>
            <input
              className="wb-probe-input"
              list="wb-model-suggestions"
              placeholder={t('workbuddy.modelProbe.modelPlaceholder', '输入或选择模型 ID（以 WorkBuddy 客户端为准）')}
              value={model}
              onChange={(e) => setModel(e.target.value)}
              disabled={state.kind === 'running'}
            />
            <datalist id="wb-model-suggestions">
              {datalistOptions.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.label}
                </option>
              ))}
            </datalist>
            <span className="wb-probe-cloud-note">
              {t(
                'workbuddy.modelProbe.pickerNote',
                '「对话选择器」为服务端实时下发的模型与计费倍率；更多模型可在输入框手输 ID。探测会真实发送一条极短消息，可能产生极少额度消耗。',
              )}
            </span>
            {cloudStatus === 'loaded' && (
              <span className="wb-probe-cloud-note">
                {t('workbuddy.modelProbe.cloudLoaded', '已加载对话选择器实时模型（{{count}} 个）。', {
                  count: cloudModels.length,
                })}
              </span>
            )}
            {cloudStatus === 'empty' && (
              <span className="wb-probe-cloud-note">
                {t('workbuddy.modelProbe.cloudEmpty', '云端模型列表为空，可在输入框直接手输模型 ID。')}
              </span>
            )}
            {cloudStatus === 'error' && (
              <span className="wb-probe-cloud-note wb-probe-cloud-error">
                {t('workbuddy.modelProbe.cloudError', '云端模型列表获取失败，可在输入框直接手输模型 ID。')}
              </span>
            )}
          </label>

          <label className="wb-probe-checkbox">
            <input
              type="checkbox"
              checked={keepConversation}
              onChange={(e) => setKeepConversation(e.target.checked)}
              disabled={state.kind === 'running'}
            />
            <span>{t('workbuddy.modelProbe.keep', '保留测试会话（不自动销毁，便于之后复用）')}</span>
          </label>

          <div className="wb-probe-actions">
            <button
              type="button"
              className="btn btn-primary"
              onClick={runProbe}
              disabled={!canRun}
            >
              {state.kind === 'running' && <Loader2 size={14} className="animate-spin" />}
              {state.kind === 'running'
                ? t('workbuddy.modelProbe.running', '测试中…')
                : t('workbuddy.modelProbe.run', '测试可用性')}
            </button>
            {state.kind === 'done' && (
              <button type="button" className="btn btn-secondary" onClick={runProbe} disabled={!canRun}>
                {t('workbuddy.modelProbe.retry', '再次测试')}
              </button>
            )}
          </div>

          {state.kind === 'failed' && (
            <div className="wb-probe-result wb-probe-result-error">
              <AlertTriangle size={16} aria-hidden="true" />
              <span>{t('workbuddy.modelProbe.failed', '探测失败：{{error}}', { error: state.error })}</span>
            </div>
          )}

          {state.kind === 'done' && (
            <div className={`wb-probe-result wb-probe-result-${state.result.status}`}>
              {BadgeIcon && <BadgeIcon size={18} aria-hidden="true" />}
              <div className="wb-probe-result-main">
                {badge && <span className={badge.className}>{badge.text}</span>}
                <span className="wb-probe-result-msg">{state.result.message}</span>
                <div className="wb-probe-result-meta">
                  <span>HTTP {state.result.httpStatus ?? '—'}</span>
                  {state.result.conversationId && <span>会话 {state.result.conversationId}</span>}
                  <span>
                    {state.result.keepConversation
                      ? t('workbuddy.modelProbe.kept', '已保留会话')
                      : t('workbuddy.modelProbe.discarded', '已销毁测试会话')}
                  </span>
                </div>
                {state.result.reply && (
                  <div className="wb-probe-reply">
                    <span className="wb-probe-reply-label">
                      {t('workbuddy.modelProbe.replyLabel', '模型回复')}
                    </span>
                    <pre>{state.result.reply}</pre>
                  </div>
                )}
                {state.result.rawSnippet && (
                  <details className="wb-probe-raw">
                    <summary>{t('workbuddy.modelProbe.raw', '原始响应片段')}</summary>
                    <pre>
                      {state.result.rawSnippet.trim()
                        ? state.result.rawSnippet
                        : state.result.message}
                    </pre>
                  </details>
                )}
              </div>
            </div>
          )}

          {keepConversation && (
            <p className="wb-probe-note">
              <Trash2 size={13} aria-hidden="true" />
              {t(
                'workbuddy.modelProbe.keepNote',
                '已选择保留会话：测试产生的对话会留在账号里，需要时可手动清理。',
              )}
            </p>
          )}
        </div>
      </div>
    </div>
  );
}
