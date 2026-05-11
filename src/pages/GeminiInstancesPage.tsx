import { useMemo, useState } from "react";
import { Check, Copy, Play, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { PlatformInstancesContent } from "../components/platform/PlatformInstancesContent";
import { SingleSelectDropdown } from "../components/SingleSelectDropdown";
import { useLaunchTerminalOptions } from "../hooks/useLaunchTerminalOptions";
import { usePlatformRuntimeSupport } from "../hooks/usePlatformRuntimeSupport";
import {
  buildGeminiAccountPresentation,
  buildQuotaPreviewLines,
} from "../presentation/platformAccountPresentation";
import * as geminiInstanceService from "../services/geminiInstanceService";
import { useGeminiAccountStore } from "../stores/useGeminiAccountStore";
import { useGeminiInstanceStore } from "../stores/useGeminiInstanceStore";
import type { GeminiAccount } from "../types/gemini";
import type { InstanceProfile } from "../types/instance";

interface GeminiInstancesContentProps {
  accountsForSelect?: GeminiAccount[];
}

interface GeminiLaunchModalState {
  instanceId: string;
  instanceName: string;
  switchMessage: string;
  launchCommand: string;
  copied: boolean;
  executing: boolean;
  executeMessage: string | null;
  executeError: string | null;
}

export function GeminiInstancesContent({
  accountsForSelect,
}: GeminiInstancesContentProps = {}) {
  const { t } = useTranslation();
  const instanceStore = useGeminiInstanceStore();
  const { accounts: storeAccounts, fetchAccounts } = useGeminiAccountStore();
  const accounts = accountsForSelect ?? storeAccounts;
  const isSupportedPlatform = usePlatformRuntimeSupport("desktop");
  const [launchModal, setLaunchModal] = useState<GeminiLaunchModalState | null>(
    null,
  );
  const { terminalOptions, selectedTerminal, setSelectedTerminal } =
    useLaunchTerminalOptions();

  const accountMap = useMemo(() => {
    const map = new Map<string, GeminiAccount>();
    accounts.forEach((account) => map.set(account.id, account));
    return map;
  }, [accounts]);

  const renderGeminiQuotaPreview = (account: GeminiAccount) => {
    const presentation = buildGeminiAccountPresentation(account, t);
    const lines = buildQuotaPreviewLines(presentation.quotaItems, 3);
    if (lines.length === 0) {
      return (
        <span className="account-quota-empty">
          {t("instances.quota.empty", "No cached quota")}
        </span>
      );
    }
    return (
      <div className="account-quota-preview">
        {lines.map((line) => (
          <span className="account-quota-item" key={line.key}>
            <span className={`quota-dot ${line.quotaClass}`} />
            <span className={`quota-text ${line.quotaClass}`}>{line.text}</span>
          </span>
        ))}
      </div>
    );
  };

  const handleInstanceStarted = async (instance: InstanceProfile) => {
    const launchInfo =
      await geminiInstanceService.getGeminiInstanceLaunchCommand(instance.id);
    const boundAccount = instance.bindAccountId
      ? accountMap.get(instance.bindAccountId)
      : undefined;
    const instanceName = instance.isDefault
      ? t("instances.defaultName", "Default Instance")
      : instance.name || t("instances.defaultName", "Default Instance");
    setLaunchModal({
      instanceId: instance.id,
      instanceName,
      switchMessage: boundAccount
        ? t("accounts.switched", "Switched to {{email}}", {
            email: boundAccount.email,
          })
        : t("gemini.switch.success", "Account switched successfully"),
      launchCommand: launchInfo.launchCommand,
      copied: false,
      executing: false,
      executeMessage: null,
      executeError: null,
    });
  };

  const handleCopyLaunchCommand = async () => {
    if (!launchModal) return;
    try {
      await navigator.clipboard.writeText(launchModal.launchCommand);
      setLaunchModal((prev) => (prev ? { ...prev, copied: true } : prev));
      window.setTimeout(() => {
        setLaunchModal((prev) => (prev ? { ...prev, copied: false } : prev));
      }, 1200);
    } catch {
      setLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executeError: t(
                "common.shared.export.copyFailed",
                "Copy failed, please copy manually",
              ),
            }
          : prev,
      );
    }
  };

  const handleExecuteInTerminal = async () => {
    if (!launchModal || launchModal.executing) return;
    setLaunchModal((prev) =>
      prev
        ? { ...prev, executing: true, executeError: null, executeMessage: null }
        : prev,
    );
    try {
      const result =
        await geminiInstanceService.executeGeminiInstanceLaunchCommand(
          launchModal.instanceId,
          selectedTerminal,
        );
      setLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executing: false,
              executeMessage: result,
            }
          : prev,
      );
    } catch (error) {
      setLaunchModal((prev) =>
        prev
          ? {
              ...prev,
              executing: false,
              executeError: String(error),
            }
          : prev,
      );
    }
  };

  return (
    <>
      <PlatformInstancesContent<GeminiAccount>
        instanceStore={instanceStore}
        accounts={accounts}
        fetchAccounts={fetchAccounts}
        renderAccountQuotaPreview={renderGeminiQuotaPreview}
        renderAccountBadge={(account) => {
          const presentation = buildGeminiAccountPresentation(account, t);
          return (
            <span className={`instance-plan-badge ${presentation.planClass}`}>
              {presentation.planLabel}
            </span>
          );
        }}
        getAccountSearchText={(account) => {
          const presentation = buildGeminiAccountPresentation(account, t);
          return `${presentation.displayName} ${presentation.planLabel}`;
        }}
        appType="gemini"
        isSupported={isSupportedPlatform}
        unsupportedTitleKey="common.shared.instances.unsupported.title"
        unsupportedTitleDefault="暂不支持当前系统"
        unsupportedDescKey="gemini.instances.unsupportedDescPlatform"
        unsupportedDescDefault="Gemini Cli 多开实例仅支持 macOS、Windows 和 Linux。"
        onInstanceStarted={handleInstanceStarted}
        resolveStartSuccessMessage={() =>
          t("gemini.switch.success", "Account switched successfully")
        }
      />

      {launchModal && (
        <div className="modal-overlay" onClick={() => setLaunchModal(null)}>
          <div
            className="modal modal-lg"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>{t("gemini.instances.launchDialogTitle", "Launch Instance")}</h2>
              <button
                className="modal-close"
                onClick={() => setLaunchModal(null)}
                aria-label={t("common.close", "Close")}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="add-status success">
                <Check size={16} />
                <span>{launchModal.switchMessage}</span>
              </div>
              <div className="form-group">
                <label>{t("instances.columns.instance", "Instance")}</label>
                <input
                  className="form-input"
                  value={launchModal.instanceName}
                  readOnly
                />
              </div>
              <div className="form-group">
                <label>{t("instances.form.extraArgs", "Custom launch args")}</label>
                <textarea
                  className="form-input instance-args-input"
                  value={launchModal.launchCommand}
                  readOnly
                />
                <p className="form-hint">
                  {t(
                    "gemini.instances.launchHint",
                    "Copy the command and run manually, or execute it directly in terminal.",
                  )}
                </p>
              </div>
              <div className="form-group">
                <label>{t("instances.launchDialog.terminal", "Terminal")}</label>
                <SingleSelectDropdown
                  value={selectedTerminal}
                  onChange={setSelectedTerminal}
                  options={terminalOptions}
                  disabled={launchModal.executing}
                  ariaLabel={t("instances.launchDialog.terminal", "Terminal")}
                />
              </div>
              {launchModal.executeMessage && (
                <div className="add-status success">
                  <Check size={16} />
                  <span>{launchModal.executeMessage}</span>
                </div>
              )}
              {launchModal.executeError && (
                <div className="form-error">{launchModal.executeError}</div>
              )}
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={handleCopyLaunchCommand}
              >
                <Copy size={16} />
                {launchModal.copied
                  ? t("common.success", "Success")
                  : t("common.copy", "Copy")}
              </button>
              <button
                className="btn btn-primary"
                onClick={handleExecuteInTerminal}
                disabled={launchModal.executing}
              >
                <Play size={16} />
                {launchModal.executing
                  ? t("common.loading", "Loading...")
                  : t("gemini.instances.runInTerminal", "Run in Terminal")}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
