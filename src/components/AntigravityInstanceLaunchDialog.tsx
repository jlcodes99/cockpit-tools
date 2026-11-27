import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useAccountStore } from '../stores/useAccountStore';
import { useAntigravityLegacyInstanceStore } from '../stores/useAntigravityLegacyInstanceStore';
import { useInstanceStore } from '../stores/useInstanceStore';
import { launchAntigravityCli, switchAntigravityCli, type AntigravityCliLaunchOptions } from '../services/accountService';
import type { Account } from '../types/account';
import type { AntigravityLaunchClient, AntigravityRuntimeTarget } from '../utils/antigravityRuntimeTarget';
import { getAntigravityTierBadge } from '../utils/account';
import { isPrivacyModeEnabledByDefault, maskSensitiveValue } from '../utils/privacy';
import { AntigravityLaunchDialog } from './AntigravityLaunchDialog';
import { SingleSelectDropdown } from './SingleSelectDropdown';

interface Props {
  source: { target: AntigravityRuntimeTarget; instanceId: string };
  onClose: () => void;
}

export function AntigravityInstanceLaunchDialog({ source, onClose }: Props) {
  const { t } = useTranslation();
  const [privacyMode] = useState(isPrivacyModeEnabledByDefault);
  const [selectedIds, setSelectedIds] = useState<Partial<Record<AntigravityRuntimeTarget, string>>>({
    [source.target]: source.instanceId,
  });
  const accounts = useAccountStore((state) => state.accounts);
  const legacyStore = useAntigravityLegacyInstanceStore();
  const ideStore = useInstanceStore();

  useEffect(() => {
    const otherStore = source.target === 'antigravity' ? useInstanceStore : useAntigravityLegacyInstanceStore;
    void otherStore.getState().fetchInstances();
  }, [source.target]);

  function getStore(target: AntigravityRuntimeTarget) {
    return target === 'antigravity' ? legacyStore : ideStore;
  }

  function resolveInstance(target: AntigravityRuntimeTarget) {
    const { instances } = getStore(target);
    const selectedId = selectedIds[target];
    return selectedId ? instances.find((instance) => instance.id === selectedId) : instances.find((instance) => instance.isDefault);
  }

  const cliInstance = getStore(source.target).instances.find((instance) => instance.id === source.instanceId);
  const cliAccountId = cliInstance?.bindAccountId;

  function accountBadge(account: Account | undefined) {
    if (!account) return undefined;
    const tier = getAntigravityTierBadge(account.quota);
    return <span className={`tier-badge ${tier.className}`} title={tier.rawTier || tier.label}>{tier.label}</span>;
  }

  function renderDetails(client: AntigravityLaunchClient, disabled: boolean) {
    const isCli = client === 'cli';
    const target = isCli ? source.target : client;
    const store = getStore(target);
    const instance = isCli ? cliInstance : resolveInstance(target);
    const accountId = instance?.bindAccountId;
    const account = accounts.find((item) => item.id === accountId);
    const label = account
      ? maskSensitiveValue(account.email, privacyMode)
      : t(accountId ? 'instances.quota.accountMissing' : 'instances.quota.unbound');

    return <div className="antigravity-launch-instance-details">
      {!isCli && <SingleSelectDropdown
        value={instance?.id ?? ''}
        options={store.instances.map((item) => ({
          value: item.id,
          label: item.isDefault ? t('instances.defaultName') : item.name,
        }))}
        onChange={(id) => setSelectedIds((current) => ({ ...current, [target]: id }))}
        ariaLabel={t('instances.columns.instance')}
        placeholder={t(store.loading ? 'common.loading' : 'common.noData')}
        disabled={disabled || store.instances.length === 0}
      />}
      {!isCli && !instance && store.error && <>
        <p className="client-launch-hint" role="alert">{store.error}</p>
        <button type="button" className="btn btn-secondary antigravity-launch-retry"
          onClick={() => void store.fetchInstances()}>{t('common.retry')}</button>
      </>}
      {isCli && instance && <div className="client-launch-account">
        <span>{t('instances.columns.instance')}</span>
        <strong title={instance.name}>{instance.isDefault ? t('instances.defaultName') : instance.name}</strong>
      </div>}
      <div className="client-launch-account">
        <span>{t('common.shared.columns.account')}</span>
        <strong title={label}>{label}</strong>
        {accountBadge(account)}
      </div>
      {isCli && accountId && <p className="client-launch-hint">{t('antigravityLaunch.sharedHint')}</p>}
    </div>;
  }

  async function switchCliAccount(accountId: string, launch: boolean, options?: AntigravityCliLaunchOptions) {
    try {
      await switchAntigravityCli(accountId, launch, options);
    } finally {
      const store = useAccountStore.getState();
      void Promise.allSettled([store.fetchAccounts(), store.fetchCurrentAccount('antigravity'), store.fetchCurrentAccount('antigravity_ide')]);
      void Promise.allSettled([legacyStore.fetchInstances(), ideStore.fetchInstances()]);
    }
  }

  const ideInstance = resolveInstance('antigravity_ide');
  return <AntigravityLaunchDialog
    initialClient={source.target}
    ideInstanceId={ideInstance?.id}
    ideInstanceHasBoundAccount={!!ideInstance?.bindAccountId}
    onClose={onClose}
    renderClientDetails={renderDetails}
    canSubmit={(client) => client === 'cli'
      ? !!cliInstance
      : !!resolveInstance(client)}
    primaryLabel={(client) => {
      if (client === 'cli') return t(cliAccountId ? 'antigravityLaunch.switchAndLaunch' : 'instances.launchDialog.runInTerminal');
      return t(resolveInstance(client)?.running ? 'instances.runningDialog.restart' : 'instances.actions.start');
    }}
    onPrepareCopy={cliAccountId ? () => switchCliAccount(cliAccountId, false) : undefined}
    onLaunch={async (client, options) => {
      if (client === 'cli') {
        if (!cliAccountId) {
          await launchAntigravityCli(options);
          return;
        }
        await switchCliAccount(cliAccountId, true, options);
        return;
      }
      const instance = resolveInstance(client);
      if (!instance) throw new Error(t('common.noData'));
      await getStore(client).startInstance(instance.id);
    }}
  />;
}
