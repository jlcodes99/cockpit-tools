import { useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { Download, ExternalLink } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { installT3codeAntigravityRuntime } from '../../services/t3codeService';
import type { T3CodeProviderInstanceView, T3CodeProviderWorkspaceView } from '../../types/t3code';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { PlatformLaunchPathField } from '../PlatformLaunchPathField';
import { getT3codeRuntimePathTarget } from './t3codePresentation';

const ANTIGRAVITY_RUNTIME_GUIDE = 'https://github.com/pingdotgg/t3code/blob/v0.1.11/docs/user/providers-antigravity.md#runtime-installation';

export function T3codeRuntimeSetup({
  instance, workspace,
}: {
  instance: T3CodeProviderInstanceView;
  workspace: T3CodeProviderWorkspaceView | null;
}) {
  const { t } = useTranslation();
  const target = getT3codeRuntimePathTarget(instance.driver);
  const [installing, setInstalling] = useState(false);
  const [started, setStarted] = useState(false);
  const error = useModalErrorState();
  const antigravity = instance.driver === 'antigravity';

  async function installRuntime() {
    error.clear();
    setInstalling(true);
    try {
      if (workspace?.injectionConnected && !instance.explicitRuntimePath) {
        await installT3codeAntigravityRuntime(instance.instanceId, workspace.t3InstanceId);
        setStarted(true);
      } else {
        await openUrl(ANTIGRAVITY_RUNTIME_GUIDE);
      }
    } catch (cause) {
      error.report(String(cause));
    } finally {
      setInstalling(false);
    }
  }

  return (
    <div className="t3code-runtime-setup">
      {instance.explicitRuntimePath
        ? <p className="client-launch-hint">{t('t3code.providerWorkspace.runtimeExplicitPath')}</p>
        : target && <PlatformLaunchPathField target={target} />}
      {antigravity && (
        <div className="t3code-runtime-setup-actions">
          <button type="button" className="btn btn-secondary btn-sm"
            disabled={installing} onClick={() => void installRuntime()}>
            {workspace?.injectionConnected && !instance.explicitRuntimePath
              ? <Download size={14} aria-hidden="true" />
              : <ExternalLink size={14} aria-hidden="true" />}
            {t('t3code.providerWorkspace.runtimeInstall')}
          </button>
          {started && <span role="status">{t('t3code.providerWorkspace.runtimeInstallStarted')}</span>}
        </div>
      )}
      <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />
    </div>
  );
}
