import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  Check,
  ChevronLeft,
  CircleCheck,
  CircleX,
  Copy,
  FolderOpen,
  Play,
  RefreshCw,
  Terminal,
  X,
} from 'lucide-react';
import { open as openFileDialog } from '@tauri-apps/plugin-dialog';
import { useTranslation } from 'react-i18next';
import { PlatformInstancesContent } from '../components/platform/PlatformInstancesContent';
import { ModalErrorMessage } from '../components/ModalErrorMessage';
import { SingleSelectDropdown } from '../components/SingleSelectDropdown';
import { useEscClose } from '../hooks/useEscClose';
import { useLaunchTerminalOptions } from '../hooks/useLaunchTerminalOptions';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import * as piInstanceService from '../services/piInstanceService';
import type { PiCliStatus } from '../services/piInstanceService';
import * as piService from '../services/piService';
import { usePiAccountStore } from '../stores/usePiAccountStore';
import { usePiInstanceStore } from '../stores/usePiInstanceStore';
import {
  getPiAccountDisplayEmail,
  getPiPlanBadge,
  getPiProvidersText,
  type PiAccount,
} from '../types/pi';
import type { InstanceProfile } from '../types/instance';

interface PiInstancesContentProps {
  accountsForSelect?: PiAccount[];
}

interface PiLaunchModalState {
  instanceId: string;
  instanceName: string;
  accountId: string | null;
  workingDir: string;
  switchMessage: string;
  launchCommand: string;
  regeneratingCommand: boolean;
  copied: boolean;
  executing: boolean;
  executeMessage: string | null;
  executeError: string | null;
  errorScrollKey: number;
}

function resolvePiLaunchWorkingDir(
  instance: InstanceProfile,
  accountMap: Map<string, PiAccount>,
): { accountId: string | null; workingDir: string } {
  // 无全局当前账号：仅使用实例显式绑定的账号。
  const boundAccountId = instance.bindAccountId?.trim() || null;
  if (boundAccountId) {
    const bound = accountMap.get(boundAccountId);
    const accountDir = bound?.working_dir?.trim() || '';
    if (accountDir) {
      return { accountId: boundAccountId, workingDir: accountDir };
    }
    return {
      accountId: boundAccountId,
      workingDir: instance.workingDir?.trim() || '',
    };
  }

  return {
    accountId: null,
    workingDir: instance.workingDir?.trim() || '',
  };
}

const PI_CLI_INSTALL_COMMAND_UNIX =
  'curl -fsSL https://pi.dev/install.sh | sh';
const PI_CLI_INSTALL_COMMAND_WINDOWS =
  'irm https://pi.dev/install.ps1 | iex';

function getPiCliInstallCommand(): string {
  if (typeof navigator === 'undefined') {
    return PI_CLI_INSTALL_COMMAND_UNIX;
  }
  const platform = `${navigator.platform || ''} ${navigator.userAgent || ''}`;
  return /win/i.test(platform)
    ? PI_CLI_INSTALL_COMMAND_WINDOWS
    : PI_CLI_INSTALL_COMMAND_UNIX;
}

export function PiInstancesContent({
  accountsForSelect,
}: PiInstancesContentProps = {}) {
  const { t } = useTranslation();
  const accountStore = usePiAccountStore();
  const instanceStore = usePiInstanceStore();
  const accounts = accountsForSelect ?? accountStore.accounts;
  const isSupported = usePlatformRuntimeSupport('desktop');
  const piCliInstallCommand = useMemo(() => getPiCliInstallCommand(), []);
  const [launchModal, setLaunchModal] = useState<PiLaunchModalState | null>(
    null,
  );
  const [cliStatus, setCliStatus] = useState<PiCliStatus | null>(null);
  const [cliPath, setCliPath] = useState('');
  const [cliModalOpen, setCliModalOpen] = useState(false);
  const [cliStatusLoading, setCliStatusLoading] = useState(false);
  const [cliSaving, setCliSaving] = useState(false);
  const [cliError, setCliError] = useState<string | null>(null);
  const [cliErrorScrollKey, setCliErrorScrollKey] = useState(0);
  const [cliActionError, setCliActionError] = useState<string | null>(null);
  const [cliActionErrorScrollKey, setCliActionErrorScrollKey] = useState(0);
  const [installCommandCopied, setInstallCommandCopied] = useState(false);
  const [installExecuting, setInstallExecuting] = useState(false);
  const [installOpened, setInstallOpened] = useState(false);
  const [retryInstanceId, setRetryInstanceId] = useState<string | null>(null);
  const { terminalOptions, selectedTerminal, setSelectedTerminal } =
    useLaunchTerminalOptions();

  useEscClose(!!launchModal, () => setLaunchModal(null));
  useEscClose(cliModalOpen, () => {
    setCliModalOpen(false);
    setCliError(null);
    setCliActionError(null);
    setInstallCommandCopied(false);
    setInstallExecuting(false);
    setInstallOpened(false);
    setRetryInstanceId(null);
  });

  const applyCliStatus = useCallback((status: PiCliStatus) => {
    setCliStatus(status);
    setCliPath(status.configuredPath || '');
  }, []);

  const reportCliError = useCallback((message: string) => {
    setCliError(message);
    setCliErrorScrollKey((current) => current + 1);
  }, []);

  const reportCliActionError = useCallback((message: string) => {
    setCliActionError(message);
    setCliActionErrorScrollKey((current) => current + 1);
  }, []);

  const resolveCliUnavailableMessage = useCallback(
    (status: PiCliStatus) => {
      const configuredPath = status.configuredPath?.trim();
      return configuredPath
        ? t(
            'pi.instances.cliPathInvalid',
            '配置的 pi CLI 路径无效：{{path}}',
            { path: configuredPath },
          )
        : t(
            'quickSettings.pi.cliMissing',
            '未检测到 pi CLI，可填写自定义路径',
          );
    },
    [t],
  );

  const loadCliStatus = useCallback(
    async (showError: boolean) => {
      setCliStatusLoading(true);
      if (showError) {
        setCliError(null);
        setCliActionError(null);
      }
      try {
        const status = await piInstanceService.getPiCliStatus();
        applyCliStatus(status);
        return status;
      } catch (error) {
        if (showError) reportCliError(String(error));
        return null;
      } finally {
        setCliStatusLoading(false);
      }
    },
    [applyCliStatus, reportCliError],
  );

  useEffect(() => {
    void loadCliStatus(false);
  }, [loadCliStatus]);

  const accountMap = useMemo(() => {
    const map = new Map<string, PiAccount>();
    accounts.forEach((account) => map.set(account.id, account));
    return map;
  }, [accounts]);

  const handleInstanceStarted = async (instance: InstanceProfile) => {
    const { accountId, workingDir } = resolvePiLaunchWorkingDir(
      instance,
      accountMap,
    );
    const boundAccount = accountId ? accountMap.get(accountId) : undefined;
    const instanceName = instance.isDefault
      ? t('instances.defaultName', '默认实例')
      : instance.name || t('instances.defaultName', '默认实例');
    try {
      const launchInfo = await piInstanceService.getPiInstanceLaunchCommand(
        instance.id,
        {
          workingDir,
          applyWorkingDirOverride: true,
          accountId,
        },
      );
      setLaunchModal({
        instanceId: instance.id,
        instanceName,
        accountId,
        workingDir,
        switchMessage: boundAccount
          ? t(
              'pi.instances.accountHomeReady',
              '已准备独立目录：{{email}}',
              {
                email: getPiAccountDisplayEmail(boundAccount),
              },
            )
          : t('instances.messages.launchPrepared', '启动命令已准备'),
        launchCommand: launchInfo.launchCommand,
        regeneratingCommand: false,
        copied: false,
        executing: false,
        executeMessage: null,
        // 账号授权类问题只体现在账号列表，启动弹框不展示
        executeError: null,
        errorScrollKey: 0,
      });
      void accountStore.fetchAccounts();
    } catch (error) {
      const message = String(error);
      setLaunchModal({
        instanceId: instance.id,
        instanceName,
        accountId,
        workingDir,
        switchMessage: boundAccount
          ? t('accounts.switched', '已切换至 {{email}}', {
              email: getPiAccountDisplayEmail(boundAccount),
            })
          : t('instances.messages.launchPrepared', '启动命令已准备'),
        launchCommand: '',
        regeneratingCommand: false,
        copied: false,
        executing: false,
        executeMessage: null,
        executeError: message,
        errorScrollKey: 1,
      });
    }
  };

  const handleInstanceStartError = useCallback(
    async (_error: unknown, instance: InstanceProfile) => {
      let status: PiCliStatus;
      try {
        status = await piInstanceService.getPiCliStatus();
      } catch {
        return false;
      }
      applyCliStatus(status);
      if (status.available) return false;

      setRetryInstanceId(instance.id);
      setCliModalOpen(true);
      reportCliError(resolveCliUnavailableMessage(status));
      return true;
    },
    [applyCliStatus, reportCliError, resolveCliUnavailableMessage],
  );

  const closeCliModal = () => {
    setCliModalOpen(false);
    setCliError(null);
    setCliActionError(null);
    setInstallCommandCopied(false);
    setInstallExecuting(false);
    setInstallOpened(false);
    setRetryInstanceId(null);
  };

  const openCliModal = () => {
    setRetryInstanceId(null);
    setCliError(null);
    setCliActionError(null);
    setInstallCommandCopied(false);
    setInstallExecuting(false);
    setInstallOpened(false);
    setCliModalOpen(true);
    void loadCliStatus(true);
  };

  const handleSaveCliPath = async () => {
    if (cliSaving || installExecuting) return;
    setCliSaving(true);
    setCliError(null);
    setCliActionError(null);
    try {
      const status = await piInstanceService.updatePiCliRuntimeConfig(
        cliPath,
      );
      applyCliStatus(status);
      if (!status.available) {
        reportCliError(resolveCliUnavailableMessage(status));
        return;
      }

      if (!retryInstanceId) return;
      try {
        const startedInstance = await instanceStore.startInstance(
          retryInstanceId,
        );
        await handleInstanceStarted(startedInstance);
        setRetryInstanceId(null);
        setCliModalOpen(false);
      } catch (error) {
        reportCliError(String(error));
      }
    } catch (error) {
      reportCliError(String(error));
    } finally {
      setCliSaving(false);
    }
  };

  const cliStatusText = cliStatus?.available
    ? t('quickSettings.pi.cliDetected', '已检测 {{version}} · {{path}}', {
        version: cliStatus.version || '--',
        path: cliStatus.binaryPath || '--',
      })
    : t(
        'quickSettings.pi.cliMissing',
        '未检测到 pi CLI，可填写自定义路径',
      );
  const showLaunchInstallGuide = Boolean(
    launchModal?.executeError &&
      piInstanceService.isPiCliMissingError(launchModal.executeError),
  );

  const handleCopyInstallCommand = async () => {
    setCliActionError(null);
    setInstallCommandCopied(false);
    try {
      await navigator.clipboard.writeText(piCliInstallCommand);
      setInstallCommandCopied(true);
      window.setTimeout(() => setInstallCommandCopied(false), 1200);
    } catch {
      reportCliActionError(
        t('common.shared.export.copyFailed', '复制失败，请手动复制'),
      );
    }
  };

  const handleExecuteInstallCommand = async () => {
    if (installExecuting) return;
    setCliActionError(null);
    setInstallOpened(false);
    setInstallExecuting(true);
    try {
      await piInstanceService.executePiCliInstallCommand(selectedTerminal);
      setInstallOpened(true);
    } catch (error) {
      reportCliActionError(String(error));
    } finally {
      setInstallExecuting(false);
    }
  };

  const handleInstallTerminalChange = (terminal: string) => {
    setSelectedTerminal(terminal);
    setCliActionError(null);
    setInstallOpened(false);
  };

  const persistLaunchWorkingDir = useCallback(
    async (accountId: string | null, workingDir: string) => {
      if (!accountId) return;
      await piService.updatePiAccountWorkingDir(
        accountId,
        workingDir.trim() || null,
      );
      await accountStore.fetchAccounts();
    },
    [accountStore],
  );

  const regenerateLaunchCommand = useCallback(
    async (modal: PiLaunchModalState, workingDir: string) => {
      setLaunchModal((current) =>
        current && current.instanceId === modal.instanceId
          ? {
              ...current,
              workingDir,
              regeneratingCommand: true,
              executeError: null,
              executeMessage: null,
            }
          : current,
      );
      try {
        const launchInfo =
          await piInstanceService.getPiInstanceLaunchCommand(
            modal.instanceId,
            {
              workingDir,
              applyWorkingDirOverride: true,
              accountId: modal.accountId,
            },
          );
        setLaunchModal((current) =>
          current && current.instanceId === modal.instanceId
            ? {
                ...current,
                workingDir,
                launchCommand: launchInfo.launchCommand,
                regeneratingCommand: false,
                executeError: null,
              }
            : current,
        );
        void accountStore.fetchAccounts();
      } catch (error) {
        const message = String(error);
        setLaunchModal((current) =>
          current && current.instanceId === modal.instanceId
            ? {
                ...current,
                workingDir,
                regeneratingCommand: false,
                executeError: message,
                errorScrollKey: current.errorScrollKey + 1,
              }
            : current,
        );
      }
    },
    [accountStore],
  );

  const updateLaunchWorkingDir = (value: string) => {
    setLaunchModal((current) =>
      current
        ? {
            ...current,
            workingDir: value,
            launchCommand: '',
            executeError: null,
            executeMessage: null,
          }
        : current,
    );
  };

  const handleChooseLaunchWorkingDir = async () => {
    if (!launchModal || launchModal.executing || launchModal.regeneratingCommand)
      return;
    const selected = await openFileDialog({
      directory: true,
      multiple: false,
      title: t('pi.instances.selectWorkingDir', '选择 pi CLI 工作目录'),
    });
    if (!selected || typeof selected !== 'string') return;
    try {
      await persistLaunchWorkingDir(launchModal.accountId, selected);
      await regenerateLaunchCommand(launchModal, selected);
    } catch (error) {
      setLaunchModal((current) =>
        current
          ? {
              ...current,
              executeError: String(error),
              errorScrollKey: current.errorScrollKey + 1,
            }
          : current,
      );
    }
  };

  const handleLaunchWorkingDirBlur = async () => {
    if (!launchModal || launchModal.executing || launchModal.regeneratingCommand)
      return;
    const nextWorkingDir = launchModal.workingDir.trim();
    try {
      await regenerateLaunchCommand(launchModal, nextWorkingDir);
    } catch (error) {
      setLaunchModal((current) =>
        current
          ? {
              ...current,
              executeError: String(error),
              errorScrollKey: current.errorScrollKey + 1,
            }
          : current,
      );
    }
  };

  const ensureLaunchCommandReady = async (
    modal: PiLaunchModalState,
  ): Promise<PiLaunchModalState | null> => {
    if (modal.launchCommand.trim() && !modal.regeneratingCommand) {
      return modal;
    }
    try {
      await persistLaunchWorkingDir(modal.accountId, modal.workingDir);
      const launchInfo = await piInstanceService.getPiInstanceLaunchCommand(
        modal.instanceId,
        {
          workingDir: modal.workingDir,
          applyWorkingDirOverride: true,
          accountId: modal.accountId,
        },
      );
      const next: PiLaunchModalState = {
        ...modal,
        launchCommand: launchInfo.launchCommand,
        regeneratingCommand: false,
        executeError: null,
      };
      setLaunchModal((current) =>
        current && current.instanceId === modal.instanceId ? next : current,
      );
      return next;
    } catch (error) {
      setLaunchModal((current) =>
        current && current.instanceId === modal.instanceId
          ? {
              ...current,
              regeneratingCommand: false,
              executeError: String(error),
              errorScrollKey: current.errorScrollKey + 1,
            }
          : current,
      );
      return null;
    }
  };

  const handleCopyLaunchCommand = async () => {
    if (!launchModal) return;
    const prepared = await ensureLaunchCommandReady(launchModal);
    if (!prepared?.launchCommand) return;
    try {
      await navigator.clipboard.writeText(prepared.launchCommand);
      setLaunchModal((current) =>
        current ? { ...current, copied: true, executeError: null } : current,
      );
      window.setTimeout(() => {
        setLaunchModal((current) =>
          current ? { ...current, copied: false } : current,
        );
      }, 1200);
    } catch {
      setLaunchModal((current) =>
        current
          ? {
              ...current,
              executeError: t(
                'common.shared.export.copyFailed',
                '复制失败，请手动复制',
              ),
              errorScrollKey: current.errorScrollKey + 1,
            }
          : current,
      );
    }
  };

  const handleExecuteInTerminal = async () => {
    if (!launchModal || launchModal.executing || launchModal.regeneratingCommand)
      return;
    setLaunchModal((current) =>
      current
        ? {
            ...current,
            executing: true,
            executeError: null,
            executeMessage: null,
          }
        : current,
    );
    try {
      const prepared = await ensureLaunchCommandReady(launchModal);
      if (!prepared) {
        setLaunchModal((current) =>
          current ? { ...current, executing: false } : current,
        );
        return;
      }
      const result = await piInstanceService.executePiInstanceLaunchCommand(
        prepared.instanceId,
        selectedTerminal,
        {
          workingDir: prepared.workingDir,
          applyWorkingDirOverride: true,
          accountId: prepared.accountId,
        },
      );
      setLaunchModal((current) =>
        current
          ? {
              ...current,
              executing: false,
              launchCommand: prepared.launchCommand,
              executeMessage: result,
            }
          : current,
      );
    } catch (error) {
      const message = String(error);
      void loadCliStatus(false);
      setLaunchModal((current) =>
        current
          ? {
              ...current,
              executing: false,
              executeError: message,
              errorScrollKey: current.errorScrollKey + 1,
            }
          : current,
      );
    }
  };

  const handleTerminalChange = (terminal: string) => {
    setSelectedTerminal(terminal);
    setCliActionError(null);
    setInstallOpened(false);
    setLaunchModal((current) =>
      current
        ? { ...current, executeError: null, executeMessage: null }
        : current,
    );
  };

  const renderPiCliInstallGuide = (
    controlsDisabled: boolean,
    hintText = t(
      'pi.instances.installHint',
      '可在终端运行以下官方命令，安装完成后点击刷新。',
    ),
  ) => (
    <div className="grok-cli-install-guide">
      <strong>
        {t('pi.instances.installCommand', '官方安装命令')}
      </strong>
      <p>{hintText}</p>
      <div className="grok-cli-install-command">
        <code>{piCliInstallCommand}</code>
        <button
          type="button"
          className="btn btn-secondary icon-only"
          onClick={() => void handleCopyInstallCommand()}
          title={
            installCommandCopied
              ? t('common.success', '成功')
              : t('common.copy', '复制')
          }
          aria-label={t('common.copy', '复制')}
        >
          {installCommandCopied ? <Check size={14} /> : <Copy size={14} />}
        </button>
      </div>
      <div className="grok-cli-install-actions">
        <div className="grok-cli-install-terminal">
          <label>{t('instances.launchDialog.terminal', '终端')}</label>
          <SingleSelectDropdown
            value={selectedTerminal}
            onChange={handleInstallTerminalChange}
            options={terminalOptions}
            disabled={controlsDisabled || installExecuting}
          />
        </div>
        <button
          type="button"
          className="btn btn-primary"
          onClick={() => void handleExecuteInstallCommand()}
          disabled={controlsDisabled || installExecuting}
        >
          <Play size={14} />
          {installExecuting
            ? t('common.loading', '加载中...')
            : t('pi.instances.runInTerminal', '终端执行')}
        </button>
      </div>
      {installOpened && (
        <div className="add-status success">
          <Check size={14} />
          <span>{t('common.success', '成功')}</span>
        </div>
      )}
      <ModalErrorMessage
        message={cliActionError}
        scrollKey={cliActionErrorScrollKey}
      />
    </div>
  );

  return (
    <>
      <PlatformInstancesContent<PiAccount>
        instanceStore={instanceStore}
        accounts={accounts}
        fetchAccounts={accountStore.fetchAccounts}
        renderAccountQuotaPreview={(account) => (
          <span className="instance-plan-badge" title={getPiProvidersText(account)}>
            {account.default_model || t('pi.noDefaultModel', '未设置默认模型')}
          </span>
        )}
        renderAccountBadge={(account) => (
          <span className="instance-plan-badge">
            {getPiPlanBadge(account) || t('common.none', '暂无')}
          </span>
        )}
        getAccountDisplayText={getPiAccountDisplayEmail}
        getAccountSearchText={(account) =>
          [
            getPiAccountDisplayEmail(account),
            account.default_model,
            ...(account.providers ?? []).map((item) => item.provider),
            getPiPlanBadge(account) || t('common.none', '暂无'),
          ]
            .filter(Boolean)
            .join(' ')
        }
        appType="pi"
        isSupported={isSupported}
        unsupportedTitleKey="common.shared.instances.unsupported.title"
        unsupportedTitleDefault="暂不支持当前系统"
        unsupportedDescKey="pi.instances.unsupported"
        unsupportedDescDefault="pi CLI 多开仅支持 macOS、Windows 和 Linux。"
        onInstanceStarted={handleInstanceStarted}
        onInstanceStartError={handleInstanceStartError}
        resolveStartSuccessMessage={() =>
          t('instances.messages.launchPrepared', '启动命令已准备')
        }
        toolbarExtraActions={
          <button
            type="button"
            className={`btn btn-secondary grok-cli-status-button${
              cliStatus?.available ? ' is-ready' : ' is-missing'
            }`}
            onClick={openCliModal}
            title={cliStatusText}
            aria-label={t('quickSettings.pi.title', 'pi CLI 设置')}
          >
            <Terminal size={14} />
            <span>
              {cliStatusLoading
                ? t('common.loading', '加载中...')
                : cliStatus?.available
                  ? cliStatus.version ||
                    t('pi.instances.cliReady', '已检测')
                  : t('pi.instances.cliMissingShort', '未检测')}
            </span>
          </button>
        }
      />

      {cliModalOpen && (
        <div className="modal-overlay">
          <div
            className="modal grok-cli-settings-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <button
                className="btn btn-secondary icon-only"
                onClick={closeCliModal}
                title={t('common.back', '返回')}
                aria-label={t('common.back', '返回')}
              >
                <ChevronLeft size={14} />
              </button>
              <h2>{t('quickSettings.pi.title', 'pi CLI 设置')}</h2>
              <button
                className="modal-close"
                onClick={closeCliModal}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div
                className={`grok-cli-runtime-status${
                  cliStatus?.available ? ' is-ready' : ' is-missing'
                }`}
              >
                {cliStatus?.available ? (
                  <CircleCheck size={18} />
                ) : (
                  <CircleX size={18} />
                )}
                <span>{cliStatusText}</span>
                <button
                  type="button"
                  className="btn btn-secondary icon-only"
                  onClick={() => void loadCliStatus(true)}
                  disabled={cliStatusLoading || cliSaving || installExecuting}
                  title={t('common.refresh', '刷新')}
                  aria-label={t('common.refresh', '刷新')}
                >
                  <RefreshCw
                    size={14}
                    className={cliStatusLoading ? 'icon-spin' : ''}
                  />
                </button>
              </div>
              {cliStatus && !cliStatus.available &&
                renderPiCliInstallGuide(cliSaving)}
              <div className="form-group">
                <label>{t('quickSettings.pi.cliPath', 'CLI 路径')}</label>
                <input
                  className="form-input"
                  value={cliPath}
                  placeholder={cliStatus?.binaryPath || 'pi'}
                  onChange={(event) => {
                    setCliPath(event.target.value);
                    setCliError(null);
                  }}
                  disabled={cliSaving || installExecuting}
                  autoFocus
                />
                <ModalErrorMessage
                  message={cliError}
                  scrollKey={cliErrorScrollKey}
                />
              </div>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={closeCliModal}
                disabled={cliSaving || installExecuting}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleSaveCliPath()}
                disabled={cliSaving || installExecuting}
              >
                {cliSaving
                  ? t('common.loading', '加载中...')
                  : retryInstanceId
                    ? t('pi.instances.saveAndRetry', '保存并重试')
                    : t('common.save', '保存')}
              </button>
            </div>
          </div>
        </div>
      )}

      {launchModal && (
        <div className="modal-overlay">
          <div
            className="modal modal-lg grok-launch-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <button
                className="btn btn-secondary icon-only"
                onClick={() => setLaunchModal(null)}
                title={t('common.back', '返回')}
                aria-label={t('common.back', '返回')}
              >
                <ChevronLeft size={14} />
              </button>
              <h2>{t('pi.instances.launchDialogTitle', '启动实例')}</h2>
              <button
                className="modal-close"
                onClick={() => setLaunchModal(null)}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="add-status success">
                <Check size={16} />
                <span>{launchModal.switchMessage}</span>
              </div>
              <ModalErrorMessage
                message={launchModal.executeError}
                scrollKey={launchModal.errorScrollKey}
              />
              {showLaunchInstallGuide &&
                renderPiCliInstallGuide(
                  launchModal.executing,
                  t(
                    'pi.instances.installLaunchHint',
                    '可在终端运行以下官方命令，安装完成后重新点击终端执行。',
                  ),
                )}
              <div className="form-group">
                <label>{t('instances.columns.instance', '实例')}</label>
                <input
                  className="form-input"
                  value={launchModal.instanceName}
                  readOnly
                />
              </div>
              <div className="form-group">
                <label>{t('instances.form.workingDir', '工作目录')}</label>
                <div className="grok-launch-working-dir-row">
                  <input
                    className="form-input"
                    value={launchModal.workingDir}
                    placeholder={t(
                      'instances.form.workingDirPlaceholder',
                      '默认当前路径',
                    )}
                    onChange={(event) =>
                      updateLaunchWorkingDir(event.target.value)
                    }
                    onBlur={() => void handleLaunchWorkingDirBlur()}
                    disabled={
                      launchModal.executing || launchModal.regeneratingCommand
                    }
                  />
                  <button
                    className="btn btn-secondary"
                    type="button"
                    onClick={() => void handleChooseLaunchWorkingDir()}
                    disabled={
                      launchModal.executing || launchModal.regeneratingCommand
                    }
                    title={t(
                      'pi.instances.selectWorkingDir',
                      '选择 pi CLI 工作目录',
                    )}
                    aria-label={t(
                      'pi.instances.selectWorkingDir',
                      '选择 pi CLI 工作目录',
                    )}
                  >
                    <FolderOpen size={16} />
                  </button>
                </div>
                <p className="form-hint">
                  {launchModal.accountId
                    ? t(
                        'pi.instances.workingDirAccountHint',
                        '工作目录与当前账号绑定，下次切号启动会自动回填。',
                      )
                    : t(
                        'instances.form.workingDirDesc',
                        '启动时将首先切换到此目录',
                      )}
                </p>
              </div>
              <div className="form-group">
                <label>{t('instances.launchDialog.command', '启动命令')}</label>
                <textarea
                  className="form-input instance-args-input"
                  value={launchModal.launchCommand}
                  placeholder={
                    launchModal.regeneratingCommand
                      ? t('common.loading', '加载中...')
                      : t(
                          'pi.instances.launchCommandPlaceholder',
                          '选择或确认工作目录后生成启动命令',
                        )
                  }
                  readOnly
                />
                <p className="form-hint">
                  {t(
                    'pi.instances.launchHint',
                    '可复制命令手动执行，或点击下方按钮直接在终端执行。',
                  )}
                </p>
              </div>
              <div className="form-group">
                <label>{t('instances.launchDialog.terminal', '终端')}</label>
                <SingleSelectDropdown
                  value={selectedTerminal}
                  onChange={handleTerminalChange}
                  options={terminalOptions}
                  disabled={
                    launchModal.executing || launchModal.regeneratingCommand
                  }
                  ariaLabel={t('instances.launchDialog.terminal', '终端')}
                />
              </div>
              {launchModal.executeMessage && (
                <div className="add-status success">
                  <Check size={16} />
                  <span>{launchModal.executeMessage}</span>
                </div>
              )}
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => void handleCopyLaunchCommand()}
                disabled={
                  launchModal.executing || launchModal.regeneratingCommand
                }
              >
                <Copy size={16} />
                {launchModal.copied
                  ? t('common.success', '成功')
                  : t('common.copy', '复制')}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleExecuteInTerminal()}
                disabled={
                  launchModal.executing || launchModal.regeneratingCommand
                }
              >
                <Play size={16} />
                {launchModal.executing
                  ? t('common.loading', '加载中...')
                  : t('pi.instances.runInTerminal', '终端执行')}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
