import { useCallback, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { AlertTriangle, Check, ExternalLink, Plus, RefreshCw, Search, Settings2, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import type { Page } from '../../types/navigation';
import type { T3CodeDriverKind, T3CodeProviderInstanceView, T3CodeProviderWorkspaceView } from '../../types/t3code';
import { getAvailableProviderAccounts, getT3codePlatformSpec } from './t3codePlatformRegistry';
import { formatAccountLabel, T3codeQuotaSection } from './t3codePresentation';
import { PlatformLaunchPathField } from '../PlatformLaunchPathField';
import { T3codeRuntimeSetup } from './T3codeRuntimeSetup';

export type T3codeModalState =
  | { kind: 'add'; driver: T3CodeDriverKind }
  | { kind: 'runtime'; instance: T3CodeProviderInstanceView }
  | { kind: 'remove'; instance: T3CodeProviderInstanceView };

interface T3codeModalsProps {
  modal: T3codeModalState;
  workspace: T3CodeProviderWorkspaceView | null;
  gatewayName: string;
  maskAccountText: (value?: string | null) => string;
  renderPlanBadge: (driver: T3CodeDriverKind, id?: string, plan?: string, apiService?: boolean) => ReactNode;
  onClose: () => void;
  onAdd: (driver: T3CodeDriverKind, accountId: string) => Promise<unknown>;
  onRemove: (instance: T3CodeProviderInstanceView) => Promise<unknown>;
  onNavigate?: (page: Page) => void;
}

export function T3codeModals({
  modal, workspace, gatewayName, maskAccountText, renderPlanBadge, onClose, onAdd, onRemove, onNavigate,
}: T3codeModalsProps) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [pathSaving, setPathSaving] = useState(false);
  const [search, setSearch] = useState('');
  const [selectedId, setSelectedId] = useState('');
  const [missingCli, setMissingCli] = useState(false);
  const modalError = useModalErrorState();
  const dialogRef = useRef<HTMLFormElement>(null);
  const add = modal.kind === 'add';
  const runtimeSetup = modal.kind === 'runtime';
  const remove = modal.kind === 'remove';
  const driver = add ? modal.driver : modal.instance.driver;
  const spec = getT3codePlatformSpec(driver);
  const accounts = add ? getAvailableProviderAccounts(workspace, driver) : [];
  const matches = accounts.filter((account) => {
    const query = search.trim().toLowerCase();
    return [formatAccountLabel(account, gatewayName, maskAccountText), account.planBadge]
      .some((value) => value?.toLowerCase().includes(query));
  });
  const selected = accounts.some((account) => account.id === selectedId);
  const disabled = busy || pathSaving;
  const action = t(add ? 't3code.providerWorkspace.addAccount'
    : runtimeSetup ? 'appPath.missing.configurePath' : 't3code.overview.removeAccount');
  const HeaderIcon = add ? Plus : runtimeSetup ? Settings2 : AlertTriangle;

  const close = useCallback(() => {
    if (!disabled) onClose();
  }, [disabled, onClose]);

  useEscCloseTopmost(true, close, dialogRef);
  useModalFocusTrap(dialogRef, true);

  const submit = async () => {
    if (runtimeSetup || busy || (add && !selected)) return;
    modalError.clear();
    setBusy(true);
    try {
      if (add) await onAdd(driver, selectedId);
      else await onRemove(modal.instance);
      onClose();
    } catch (error) {
      if (add && String(error).replace(/^Error: /, '') === 'CLI_PATH_NOT_FOUND:codebuddy_cli') {
        setMissingCli(true);
      } else {
        modalError.report(String(error));
      }
      setBusy(false);
    }
  };

  if (typeof document === 'undefined') return null;
  return createPortal(
    <div className="modal-overlay t3code-modal-overlay">
      <form
        className={`modal t3code-dialog${remove ? ' is-compact' : ''}`}
        ref={dialogRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby="t3code-dialog-title"
        onSubmit={(event) => {
          event.preventDefault();
          if (!disabled) void submit();
        }}
      >
        <div className="modal-header t3code-dialog-header">
          <div className="t3code-dialog-title-wrap">
            <span className={`t3code-dialog-title-icon${remove ? ' is-danger' : ''}`} aria-hidden="true">
              <HeaderIcon size={17} />
            </span>
            <h2 id="t3code-dialog-title">{action} · {spec.label}</h2>
          </div>
          <button className="modal-close" type="button" aria-label={t('common.close')} disabled={disabled} onClick={close}>
            <X size={16} />
          </button>
        </div>
        <div className="modal-body t3code-dialog-body">
          <ModalErrorMessage message={modalError.message} scrollKey={modalError.scrollKey} />
          {missingCli && (
            <PlatformLaunchPathField target="codebuddy_cli" recovery disabled={busy} onSaving={setPathSaving}
              onSaved={() => {
                setMissingCli(false);
                void submit();
              }} />
          )}
          {add ? (
            <div className="t3code-switch-picker">
              {(accounts.length > 0 || onNavigate) && (
                <div className="t3code-switch-picker-top">
                  {accounts.length > 0 && (
                    <div className="search-box t3code-modal-search">
                      <Search size={15} className="search-icon" />
                      <input type="search" value={search} disabled={disabled} aria-label={t('common.search')}
                        placeholder={t('common.search')} onChange={(event) => {
                          setSearch(event.target.value);
                          setSelectedId('');
                          modalError.clear();
                        }} />
                    </div>
                  )}
                  {onNavigate && (
                    <button type="button" className="btn btn-secondary btn-sm t3code-inline-link-btn" disabled={disabled}
                      onClick={() => {
                        onClose();
                        onNavigate(spec.navPage);
                      }}>
                      <ExternalLink size={13} />
                      {t('t3code.overview.goAddSourceAccount', { platform: spec.label })}
                    </button>
                  )}
                </div>
              )}
              {matches.length ? (
                <div className="t3code-picker-list" role="group" aria-label={t('t3code.providerWorkspace.platformAccount')}>
                  {matches.map((account) => {
                    const isSelected = account.id === selectedId;
                    const label = formatAccountLabel(account, gatewayName, maskAccountText);
                    return (
                      <button key={account.id} type="button" aria-pressed={isSelected} disabled={disabled}
                        className={`t3code-picker-option${isSelected ? ' is-selected' : ''}`}
                        onClick={() => {
                          modalError.clear();
                          setSelectedId(account.id);
                        }}>
                        <span className="t3code-picker-option-head">
                          <span className={`t3code-picker-radio${isSelected ? ' is-checked' : ''}`} aria-hidden="true">
                            {isSelected && <Check size={12} className="t3code-picker-check" />}
                          </span>
                          <span className="t3code-picker-option-name" title={label}>{label}</span>
                          {renderPlanBadge(driver, account.id, account.planBadge, account.isApiService)}
                        </span>
                        <div className="t3code-picker-option-quota">
                          <T3codeQuotaSection driver={driver} windows={account.windows}
                            quotaSummaryGroups={account.quotaSummaryGroups} providerQuota={account.providerQuota}
                            allowQuotaOverflow={account.isApiService}
                            compact emptyLabel={t('t3code.quotaQuery.empty')} />
                        </div>
                      </button>
                    );
                  })}
                </div>
              ) : (
                <div className="t3code-picker-empty-box">
                  <p className="t3code-picker-empty">
                    {t(accounts.length ? 'common.shared.noMatch.title' : 't3code.providerWorkspace.noAccounts')}
                  </p>
                </div>
              )}
            </div>
          ) : modal.kind === 'runtime' ? (
            <T3codeRuntimeSetup instance={modal.instance} workspace={workspace} />
          ) : (
            <div className="t3code-confirm-card">
              <div className="t3code-confirm-account">
                <span className="t3code-confirm-account-name">
                  {maskAccountText(modal.instance.boundAccountEmail || modal.instance.displayName)}
                </span>
                {renderPlanBadge(
                  modal.instance.driver,
                  modal.instance.boundAccountId,
                  modal.instance.boundAccountPlan,
                  modal.instance.boundIsApiService,
                )}
              </div>
              <p className="t3code-confirm-text">
                {t('t3code.providerWorkspace.deleteConfirmDesc', {
                  name: maskAccountText(modal.instance.boundAccountEmail || modal.instance.displayName),
                })}
              </p>
            </div>
          )}
        </div>
        <div className="modal-footer t3code-dialog-footer">
          <button className="btn btn-secondary" type="button" disabled={disabled} onClick={close}>
            {t(runtimeSetup ? 'common.close' : 'common.cancel')}
          </button>
          {!runtimeSetup && <button className={`btn ${add ? 'btn-primary' : 'btn-danger'} t3code-dialog-submit`}
            type="submit" aria-busy={busy} disabled={disabled || (add && !selected)}>
            <span className="t3code-dialog-action-label" aria-hidden={busy}>{action}</span>
            <span className="t3code-dialog-action-label" aria-hidden={!busy}>
              <RefreshCw size={14} aria-hidden="true" className={busy ? 'loading-spinner' : undefined} />
              <span>{t('common.loading')}</span>
            </span>
          </button>}
        </div>
      </form>
    </div>,
    document.body,
  );
}
