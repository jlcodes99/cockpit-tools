/**
 * WorkBuddy 模型可用性探测弹窗
 *
 * 用法：在 WorkBuddy 账号页点击「探测模型可用性」打开。
 * - 选择一个账号 + 输入/选择要测的模型
 * - 点击「测试」会向该账号的对话接口发送一条极短消息（如 "hi"）
 * - 根据返回判定：可用 / 被限流 / 请求异常
 * - 默认测试后销毁测试会话（可在「保留会话」里改）
 */

import { useMemo, useState } from 'react';
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
import { probeWorkbuddyModel, type WorkbuddyModelProbeResult } from '../../services/workbuddyService';
import { WORKBUDDY_SUGGESTED_MODELS } from '../../utils/workbuddyModels';

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
              '向所选账号的对话接口发送一条极短消息（如 “hi”），据此判断该模型当前是否可用、是否被限流。仅消耗极少 token。',
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
            <input
              className="wb-probe-input"
              list="wb-model-suggestions"
              placeholder={t('workbuddy.modelProbe.modelPlaceholder', '输入或选择模型 ID（以 WorkBuddy 客户端为准）')}
              value={model}
              onChange={(e) => setModel(e.target.value)}
              disabled={state.kind === 'running'}
            />
            <datalist id="wb-model-suggestions">
              {WORKBUDDY_SUGGESTED_MODELS.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.label}
                </option>
              ))}
            </datalist>
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
                {state.result.rawSnippet && (
                  <details className="wb-probe-raw">
                    <summary>{t('workbuddy.modelProbe.raw', '原始响应片段')}</summary>
                    <pre>{state.result.rawSnippet}</pre>
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
