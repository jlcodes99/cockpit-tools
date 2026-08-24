import { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { X } from 'lucide-react';
import { createPortal } from 'react-dom';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import * as codexService from '../../services/codexService';
import {
  isStandardCodexOAuthAccount,
  type CodexAccount,
  type CodexFingerprintMode,
} from '../../types/codex';
import './CodexOAuthPolicyModal.css';

type BatchToggleValue = 'unchanged' | 'on' | 'off';

interface CodexOAuthPolicyModalProps {
  accounts: CodexAccount[];
  onAccountsChange: (accounts: CodexAccount[]) => void;
  onClose: () => void;
}

const FINGERPRINT_MODES: Array<{ value: CodexFingerprintMode; key: string; fallback: string }> = [
  { value: 'off', key: 'Off', fallback: '关闭' },
  { value: 'device', key: 'Device', fallback: '仅设备' },
  { value: 'session', key: 'Session', fallback: '设备 + 会话' },
  { value: 'full', key: 'Full', fallback: '完整收敛' },
];

export function CodexOAuthPolicyModal({
  accounts,
  onAccountsChange,
  onClose,
}: CodexOAuthPolicyModalProps) {
  const { t } = useTranslation();
  const oauthAccounts = useMemo(
    () => accounts.filter((account) => isStandardCodexOAuthAccount(account)),
    [accounts],
  );
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [batchOfficialOnly, setBatchOfficialOnly] =
    useState<BatchToggleValue>('unchanged');
  const [batchAppServer, setBatchAppServer] =
    useState<BatchToggleValue>('unchanged');
  const [batchFingerprint, setBatchFingerprint] =
    useState<BatchToggleValue>('unchanged');
  const [batchFingerprintMode, setBatchFingerprintMode] =
    useState<CodexFingerprintMode>('off');
  const [savingId, setSavingId] = useState<string | null>(null);
  const [batchSaving, setBatchSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fingerprintLabel = (mode: (typeof FINGERPRINT_MODES)[number]) =>
    t('settings.general.codexFingerprint' + mode.key, mode.fallback);
  const allSelected =
    oauthAccounts.length > 0 && oauthAccounts.every((account) => selectedIds.has(account.id));

  const replaceAccounts = (updated: CodexAccount[]) => {
    const byId = new Map(updated.map((account) => [account.id, account]));
    onAccountsChange(accounts.map((account) => byId.get(account.id) ?? account));
  };

  const updateAccount = async (
    account: CodexAccount,
    patch: {
      officialOnly?: boolean;
      allowAppServer?: boolean;
      fingerprintMode?: CodexFingerprintMode;
    },
  ) => {
    setError(null);
    setSavingId(account.id);
    try {
      if (patch.fingerprintMode !== undefined) {
        const updated = await codexService.updateCodexAccountsFingerprintMode(
          [account.id],
          patch.fingerprintMode,
        );
        replaceAccounts(updated);
      } else {
        const updated = await codexService.updateCodexAccountClientPolicy(
          account.id,
          patch.officialOnly ?? account.codex_cli_only === true,
          patch.allowAppServer ?? account.codex_cli_only_allow_app_server === true,
        );
        replaceAccounts([updated]);
      }
    } catch (value) {
      setError(String(value).replace(/^Error:\s*/, ''));
    } finally {
      setSavingId(null);
    }
  };

  const applyBatch = async () => {
    const selectedAccounts = oauthAccounts.filter((account) => selectedIds.has(account.id));
    if (selectedAccounts.length === 0) {
      setError(t('codex.oauthPolicy.selectAccounts', '请先选择账号。'));
      return;
    }
    if (
      batchOfficialOnly === 'unchanged' &&
      batchAppServer === 'unchanged' &&
      batchFingerprint === 'unchanged'
    ) {
      setError(t('codex.oauthPolicy.selectBatchPolicy', '请选择要批量修改的策略。'));
      return;
    }

    setError(null);
    setBatchSaving(true);
    try {
      let nextAccounts = accounts;
      if (batchFingerprint !== 'unchanged') {
        const fingerprintAccounts = await codexService.updateCodexAccountsFingerprintMode(
          selectedAccounts.map((account) => account.id),
          batchFingerprint === 'on' ? batchFingerprintMode : 'off',
        );
        const byId = new Map(fingerprintAccounts.map((account) => [account.id, account]));
        nextAccounts = nextAccounts.map((account) => byId.get(account.id) ?? account);
      }

      if (batchOfficialOnly !== 'unchanged' || batchAppServer !== 'unchanged') {
        const nextById = new Map(nextAccounts.map((account) => [account.id, account]));
        for (const account of selectedAccounts) {
          const current = nextById.get(account.id) ?? account;
          const officialOnly =
            batchOfficialOnly === 'on'
              ? true
              : batchOfficialOnly === 'off'
                ? false
                : current.codex_cli_only === true;
          const allowAppServer =
            batchAppServer === 'on'
              ? true
              : batchAppServer === 'off'
                ? false
                : current.codex_cli_only_allow_app_server === true;
          const updated = await codexService.updateCodexAccountClientPolicy(
            account.id,
            officialOnly,
            allowAppServer,
          );
          nextById.set(updated.id, updated);
        }
        nextAccounts = nextAccounts.map((account) => nextById.get(account.id) ?? account);
      }

      replaceAccounts(nextAccounts);
      setBatchOfficialOnly('unchanged');
      setBatchAppServer('unchanged');
      setBatchFingerprint('unchanged');
      setSelectedIds(new Set());
    } catch (value) {
      setError(String(value).replace(/^Error:\s*/, ''));
    } finally {
      setBatchSaving(false);
    }
  };

  return createPortal(
    (
    <div className="codex-oauth-policy-modal-overlay">
      <div
        className="codex-oauth-policy-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="codex-oauth-policy-modal-title"
      >
        <div className="codex-oauth-policy-modal__header">
          <div>
            <h2 id="codex-oauth-policy-modal-title">
              {t('codex.oauthPolicy.title', 'Codex OAuth 账号策略')}
            </h2>
            <p>
              {t(
                'codex.oauthPolicy.description',
                '可批量设置，也可以在账号行内单独修改。',
              )}
            </p>
          </div>
          <button
            type="button"
            className="codex-oauth-policy-modal__close"
            onClick={onClose}
            aria-label={t('common.close', '关闭')}
          >
            <X size={18} />
          </button>
        </div>

        {error && (
          <div className="codex-oauth-policy-modal__error" role="alert">
            {error}
          </div>
        )}

        <div className="codex-oauth-policy-modal__body">
          {oauthAccounts.length === 0 ? (
            <div className="codex-oauth-policy-modal__empty">
              {t('codex.oauthPolicy.noAccounts', '暂无可配置的 Codex OAuth 账号')}
            </div>
          ) : (
            <>
              <div className="codex-oauth-policy-modal__batch">
                <div className="codex-oauth-policy-modal__batch-title">
                  <label className="codex-oauth-policy-modal__check-all">
                    <input
                      type="checkbox"
                      checked={allSelected}
                      onChange={(event) => {
                        setSelectedIds(
                          event.target.checked
                            ? new Set(oauthAccounts.map((account) => account.id))
                            : new Set(),
                        );
                      }}
                    />
                    <span>{t('codex.oauthPolicy.selectAll', '全选')}</span>
                  </label>
                  <span>
                    {t('codex.oauthPolicy.selectedCount', {
                      count: selectedIds.size,
                      defaultValue: '已选 {{count}} 个账号',
                    })}
                  </span>
                </div>
                <div className="codex-oauth-policy-modal__batch-fields">
                  <SingleSelectDropdown
                    value={batchOfficialOnly}
                    options={[
                      { value: 'unchanged', label: t('codex.oauthPolicy.officialOnlyUnchanged', '官方客户端：不修改') },
                      { value: 'on', label: t('codex.oauthPolicy.officialOnlyOn', '官方客户端：开启') },
                      { value: 'off', label: t('codex.oauthPolicy.officialOnlyOff', '官方客户端：关闭') },
                    ]}
                    onChange={(value) => setBatchOfficialOnly(value as BatchToggleValue)}
                    ariaLabel={t('codex.oauthPolicy.officialOnly', '仅官方客户端')}
                    disabled={batchSaving}
                    menuClassName="codex-oauth-policy-dropdown-menu"
                    menuWidth={230}
                  />
                  <SingleSelectDropdown
                    value={batchAppServer}
                    options={[
                      { value: 'unchanged', label: t('codex.oauthPolicy.appServerUnchanged', '第三方客户端：不修改') },
                      { value: 'on', label: t('codex.oauthPolicy.appServerOn', '第三方客户端：开启') },
                      { value: 'off', label: t('codex.oauthPolicy.appServerOff', '第三方客户端：关闭') },
                    ]}
                    onChange={(value) => setBatchAppServer(value as BatchToggleValue)}
                    ariaLabel={t('codex.oauthPolicy.appServer', '允许第三方客户端')}
                    disabled={batchSaving}
                    menuClassName="codex-oauth-policy-dropdown-menu"
                    menuWidth={230}
                  />
                  <SingleSelectDropdown
                    value={batchFingerprint}
                    options={[
                      { value: 'unchanged', label: t('codex.oauthPolicy.fingerprintUnchanged', '指纹：不修改') },
                      { value: 'on', label: t('codex.oauthPolicy.fingerprintOn', '指纹：设置模式') },
                      { value: 'off', label: t('codex.oauthPolicy.fingerprintOff', '指纹：关闭') },
                    ]}
                    onChange={(value) => setBatchFingerprint(value as BatchToggleValue)}
                    ariaLabel={t('codex.oauthPolicy.fingerprint', '设备指纹')}
                    disabled={batchSaving}
                    menuClassName="codex-oauth-policy-dropdown-menu"
                    menuWidth={190}
                  />
                  <SingleSelectDropdown
                    value={batchFingerprintMode}
                    options={FINGERPRINT_MODES.map((mode) => ({
                      value: mode.value,
                      label: fingerprintLabel(mode),
                    }))}
                    onChange={(value) => setBatchFingerprintMode(value as CodexFingerprintMode)}
                    ariaLabel={t('codex.oauthPolicy.fingerprintMode', '指纹模式')}
                    disabled={batchSaving || batchFingerprint !== 'on'}
                    menuClassName="codex-oauth-policy-dropdown-menu"
                    menuWidth={190}
                  />
                  <button
                    type="button"
                    className="btn btn-primary"
                    onClick={() => void applyBatch()}
                    disabled={batchSaving || savingId !== null}
                  >
                    {batchSaving
                      ? t('common.saving', '保存中...')
                      : t('codex.oauthPolicy.applyBatch', '应用到所选')}
                  </button>
                </div>
              </div>

              <div className="codex-oauth-policy-modal__list">
                {oauthAccounts.map((account) => {
                  const saving = savingId === account.id;
                  const fingerprintMode = account.codex_fingerprint_mode ?? 'session';
                  return (
                    <div className="codex-oauth-policy-modal__row" key={account.id}>
                      <label className="codex-oauth-policy-modal__account">
                        <input
                          type="checkbox"
                          checked={selectedIds.has(account.id)}
                          onChange={(event) => {
                            setSelectedIds((current) => {
                              const next = new Set(current);
                              if (event.target.checked) next.add(account.id);
                              else next.delete(account.id);
                              return next;
                            });
                          }}
                        />
                        <span title={account.email}>{account.email}</span>
                      </label>
                      <label className="codex-oauth-policy-modal__toggle">
                        <input
                          type="checkbox"
                          checked={account.codex_cli_only === true}
                          disabled={saving || batchSaving}
                          onChange={(event) =>
                            void updateAccount(account, {
                              officialOnly: event.target.checked,
                              allowAppServer:
                                event.target.checked &&
                                account.codex_cli_only_allow_app_server === true,
                            })
                          }
                        />
                        <span>{t('codex.oauthPolicy.officialOnlyShort', '仅官方')}</span>
                      </label>
                      <label className="codex-oauth-policy-modal__toggle">
                        <input
                          type="checkbox"
                          checked={account.codex_cli_only_allow_app_server === true}
                          disabled={saving || batchSaving || account.codex_cli_only !== true}
                          onChange={(event) =>
                            void updateAccount(account, {
                              allowAppServer: event.target.checked,
                            })
                          }
                        />
                        <span>{t('codex.oauthPolicy.appServerShort', '第三方客户端：允许')}</span>
                      </label>
                      <SingleSelectDropdown
                        value={fingerprintMode}
                        options={FINGERPRINT_MODES.map((mode) => ({
                          value: mode.value,
                          label: fingerprintLabel(mode),
                        }))}
                        onChange={(value) =>
                          void updateAccount(account, {
                            fingerprintMode: value as CodexFingerprintMode,
                          })
                        }
                        ariaLabel={t('codex.oauthPolicy.fingerprintMode', '指纹模式')}
                        disabled={saving || batchSaving}
                        menuPlacement="up"
                        menuClassName="codex-oauth-policy-dropdown-menu"
                        menuWidth={190}
                      />
                    </div>
                  );
                })}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
    ),
    document.body,
  );
}
