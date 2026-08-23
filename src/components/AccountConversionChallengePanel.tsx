import { invoke } from '@tauri-apps/api/core';
import {
  AlertTriangle,
  Check,
  Clipboard,
  ExternalLink,
  RefreshCw,
  X,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  getMfaOtpToken,
  getMfaTimeRemaining,
  loadSavedMfaRecords,
  type MfaRecord,
} from '../utils/mfaVault';
import {
  challengeExpiresInMinutes,
  challengeStatusLabel,
  canConfirmConversionChallenge,
  conversionChallengeNeedsMfaSelection,
  isActiveConversionChallenge,
  matchingMfaRecords,
  type AccountConversionBridgeStatus,
  type AccountConversionChallenge,
} from '../utils/accountConversionChallenges';

const CHALLENGE_LABELS: Record<AccountConversionChallenge['type'], string> = {
  password_current: '当前密码验证',
  password_new: '输入并保存新密码',
  totp: 'Authenticator 动态验证码',
  recovery_email_code: '辅助邮箱验证码',
  phone_code: '手机验证码',
  passkey: 'Passkey / Windows Hello',
  authenticator_setup: 'Authenticator 设置',
  backup_codes: '备用验证码离线保存',
  phone_removal: '手机号删除确认',
  session_signout: '其他会话退出确认',
  captcha: 'CAPTCHA 人工验证',
  account_recovery: '账号恢复',
  extension_install: 'Chrome 扩展安装权限',
  generic: '人工操作',
};

function safeError(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function displayMfaRecord(record: MfaRecord): string {
  return record.accountName.trim() || record.remark?.trim() || '未命名 MFA 记录';
}

function mfaChoiceKey(record: MfaRecord, index: number): string {
  // mfaVault normalizes legacy records on each load, so its generated id is not
  // stable across polling. Build a non-secret UI-only key without using the TOTP
  // seed. Full email matching is still performed separately.
  return [record.time, record.accountName, record.remark ?? '', index].join('|');
}

async function listChallenges(): Promise<AccountConversionChallenge[]> {
  return invoke<AccountConversionChallenge[]>('account_conversion_list_challenges');
}

export function AccountConversionChallengePanel() {
  const [bridge, setBridge] = useState<AccountConversionBridgeStatus | null>(null);
  const [challenges, setChallenges] = useState<AccountConversionChallenge[]>([]);
  const [mfaRecords, setMfaRecords] = useState<MfaRecord[]>(() => loadSavedMfaRecords());
  const [selectedMfaByChallenge, setSelectedMfaByChallenge] = useState<Record<string, string>>({});
  const [workingId, setWorkingId] = useState<string | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [error, setError] = useState('');
  const [timeRemaining, setTimeRemaining] = useState(() => getMfaTimeRemaining());
  const [clockMs, setClockMs] = useState(() => Date.now());

  const refresh = useCallback(async () => {
    try {
      const [nextBridge, nextChallenges] = await Promise.all([
        invoke<AccountConversionBridgeStatus>('account_conversion_bridge_status'),
        listChallenges(),
      ]);
      setBridge(nextBridge);
      setChallenges(nextChallenges);
      setMfaRecords(loadSavedMfaRecords());
      setError('');
    } catch (refreshError) {
      setError(safeError(refreshError));
    }
  }, []);

  useEffect(() => {
    void refresh();
    const interval = window.setInterval(() => void refresh(), 2_000);
    return () => window.clearInterval(interval);
  }, [refresh]);

  useEffect(() => {
    const interval = window.setInterval(
      () => {
        setTimeRemaining(getMfaTimeRemaining());
        setClockMs(Date.now());
      },
      1_000,
    );
    return () => window.clearInterval(interval);
  }, []);

  const activeChallenges = useMemo(
    () => challenges.filter(isActiveConversionChallenge),
    [challenges],
  );
  const recentTerminal = useMemo(
    () => challenges.filter((item) => !isActiveConversionChallenge(item)).slice(0, 5),
    [challenges],
  );

  const focusChrome = async (challenge: AccountConversionChallenge) => {
    await invoke<number>('account_conversion_focus_chrome', {
      chromePid: challenge.chromePid,
    });
  };

  const present = async (challenge: AccountConversionChallenge) => {
    setWorkingId(challenge.id);
    try {
      await invoke('account_conversion_present_challenge', { id: challenge.id });
      await focusChrome(challenge);
      await refresh();
    } catch (actionError) {
      setError(safeError(actionError));
    } finally {
      setWorkingId(null);
    }
  };

  const confirm = async (
    challenge: AccountConversionChallenge,
    selectedMfaId: string,
    matches: MfaRecord[],
  ) => {
    setWorkingId(challenge.id);
    try {
      const needsMfaMatch = conversionChallengeNeedsMfaSelection(challenge.type);
      const selectedMfa = matches.find(
        (record, index) => mfaChoiceKey(record, index) === selectedMfaId,
      );
      if (needsMfaMatch && !selectedMfa) {
        throw new Error('请先明确选择与完整邮箱匹配的 MFA 记录。');
      }
      await invoke('account_conversion_confirm_challenge', {
        id: challenge.id,
        mfaMatchEmail: needsMfaMatch ? challenge.expectedEmail : null,
      });
      await refresh();
    } catch (actionError) {
      setError(safeError(actionError));
    } finally {
      setWorkingId(null);
    }
  };

  const cancel = async (challenge: AccountConversionChallenge) => {
    setWorkingId(challenge.id);
    try {
      await invoke('account_conversion_cancel_challenge', { id: challenge.id });
      await refresh();
    } catch (actionError) {
      setError(safeError(actionError));
    } finally {
      setWorkingId(null);
    }
  };

  const copyTotpAndFocus = async (
    challenge: AccountConversionChallenge,
    matches: MfaRecord[],
  ) => {
    const selectedId = selectedMfaByChallenge[challenge.id]
      || (matches.length === 1 ? mfaChoiceKey(matches[0]!, 0) : '');
    const selected = matches.find(
      (record, index) => mfaChoiceKey(record, index) === selectedId,
    );
    if (!selected) {
      setError('请先明确选择与完整邮箱匹配的 MFA 记录。');
      return;
    }
    if (timeRemaining <= 5) {
      setError('当前验证码即将过期，请等待下一个周期后再复制。');
      return;
    }
    const code = getMfaOtpToken(selected.secret);
    if (!code) {
      setError('所选 MFA 记录当前无法生成验证码，请在 MFA 管理区检查记录。');
      return;
    }
    setWorkingId(challenge.id);
    try {
      // The code exists only inside this WebView and the user clipboard. It is
      // deliberately not passed to Rust, HTTP, logs, or the CDP orchestrator.
      await navigator.clipboard.writeText(code);
      await invoke('account_conversion_present_challenge', { id: challenge.id });
      await focusChrome(challenge);
      setCopiedId(challenge.id);
      window.setTimeout(() => setCopiedId(null), 1_500);
      setError('');
      await refresh();
    } catch (actionError) {
      setError(safeError(actionError));
    } finally {
      setWorkingId(null);
    }
  };

  return (
    <section className="account-conversion-panel" aria-labelledby="account-conversion-title">
      <div className="account-conversion-panel__heading">
        <div>
          <h2 id="account-conversion-title">账号转换挑战</h2>
          <p>
            Cockpit 只展示本机人工挑战。密码、动态码、二维码和备用码不会通过本机 HTTP 桥返回给编排器。
          </p>
        </div>
        <button type="button" className="btn btn-secondary" onClick={() => void refresh()}>
          <RefreshCw size={14} /> 刷新
        </button>
      </div>

      <div className="account-conversion-panel__bridge">
        <span className={bridge?.running ? 'is-running' : 'is-stopped'}>
          {bridge?.running ? '桥已运行' : '桥未运行'}
        </span>
        <span>队列 {bridge?.queuedCount ?? 0}</span>
        <span>处理中 {bridge?.presentedCount ?? 0}</span>
        <span>PID {bridge?.pid ?? '--'}</span>
      </div>

      {error ? (
        <div className="account-conversion-panel__error" role="alert">
          <AlertTriangle size={16} /> {error}
        </div>
      ) : null}

      {activeChallenges.length === 0 ? (
        <div className="account-conversion-panel__empty">当前没有待处理的账号转换挑战。</div>
      ) : (
        <div className="account-conversion-panel__list">
          {activeChallenges.map((challenge) => {
            const matches = matchingMfaRecords(mfaRecords, challenge.expectedEmail);
            const choices = matches.map((record, index) => ({
              record,
              key: mfaChoiceKey(record, index),
            }));
            const selectedId = selectedMfaByChallenge[challenge.id]
              || (choices.length === 1 ? choices[0]?.key : '');
            const isTotp = challenge.type === 'totp';
            const needsMfaMatch = conversionChallengeNeedsMfaSelection(
              challenge.type,
            );
            const busy = workingId === challenge.id;
            return (
              <article className="account-conversion-challenge" key={challenge.id}>
                <div className="account-conversion-challenge__title">
                  <div>
                    <strong>{CHALLENGE_LABELS[challenge.type]}</strong>
                    <span className={`challenge-status challenge-status--${challenge.status}`}>
                      {challengeStatusLabel(challenge.status)}
                    </span>
                  </div>
                  <code>{challenge.slot} · CDP {challenge.port}</code>
                </div>
                <dl>
                  <div><dt>账号</dt><dd>{challenge.expectedEmail}</dd></div>
                  <div><dt>说明</dt><dd>{challenge.instructions}</dd></div>
                  <div>
                    <dt>有效期</dt>
                    <dd>约 {challengeExpiresInMinutes(challenge, clockMs)} 分钟后到期</dd>
                  </div>
                </dl>

                {needsMfaMatch ? (
                  <div className="account-conversion-challenge__mfa">
                    {matches.length === 0 ? (
                      <p className="is-warning">
                        未找到包含该完整邮箱的 MFA 记录。请先在下方现有 MFA 管理区添加或导入。
                      </p>
                    ) : (
                      <>
                        {matches.length > 1 ? (
                          <p className="is-warning">匹配到多条记录，请明确选择，系统不会猜测。</p>
                        ) : null}
                        <label>
                          MFA 记录
                          <select
                            value={selectedId}
                            onChange={(event) => setSelectedMfaByChallenge((current) => ({
                              ...current,
                              [challenge.id]: event.target.value,
                            }))}
                          >
                            {matches.length > 1 ? <option value="">请选择</option> : null}
                            {choices.map((choice) => (
                              <option value={choice.key} key={choice.key}>
                                {displayMfaRecord(choice.record)}
                              </option>
                            ))}
                          </select>
                        </label>
                        {isTotp ? (
                          <span className={timeRemaining <= 5 ? 'is-warning' : ''}>
                            当前验证码周期剩余 {timeRemaining} 秒
                          </span>
                        ) : (
                          <span>完整邮箱已匹配 Cockpit MFA 记录。</span>
                        )}
                      </>
                    )}
                  </div>
                ) : null}

                <div className="account-conversion-challenge__actions">
                  {isTotp ? (
                    <button
                      type="button"
                      className="btn btn-primary"
                      disabled={busy || !selectedId || timeRemaining <= 5}
                      onClick={() => void copyTotpAndFocus(challenge, matches)}
                    >
                      {copiedId === challenge.id ? <Check size={14} /> : <Clipboard size={14} />}
                      {copiedId === challenge.id
                        ? '已复制并定位'
                        : timeRemaining <= 5
                          ? '等待下一个验证码周期'
                          : '复制验证码并定位 Chrome'}
                    </button>
                  ) : (
                    <button
                      type="button"
                      className="btn btn-primary"
                      disabled={busy}
                      onClick={() => void present(challenge)}
                    >
                      <ExternalLink size={14} /> 定位 Chrome
                    </button>
                  )}
                  <button
                    type="button"
                    className="btn btn-secondary"
                    disabled={
                      busy ||
                      !canConfirmConversionChallenge(
                        challenge.type,
                        selectedId ?? '',
                      )
                    }
                    onClick={() => void confirm(challenge, selectedId ?? '', matches)}
                  >
                    <Check size={14} /> 我已完成
                  </button>
                  <button
                    type="button"
                    className="btn btn-secondary"
                    disabled={busy}
                    onClick={() => void cancel(challenge)}
                  >
                    <X size={14} /> 取消
                  </button>
                </div>
                <p className="account-conversion-challenge__note">
                  “我已完成”仅记录 user_confirmed；CDP 编排器恢复后仍会重新读取 Google/Chrome 状态。
                </p>
              </article>
            );
          })}
        </div>
      )}

      {recentTerminal.length > 0 ? (
        <details className="account-conversion-panel__recent">
          <summary>最近已结束挑战（{recentTerminal.length}）</summary>
          <ul>
            {recentTerminal.map((challenge) => (
              <li key={challenge.id}>
                {challenge.slot} · {CHALLENGE_LABELS[challenge.type]} · {challengeStatusLabel(challenge.status)}
              </li>
            ))}
          </ul>
        </details>
      ) : null}
    </section>
  );
}
