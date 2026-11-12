import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { applyClaudeCliAccountToUserConfig } from '../../services/claudeService';

export function ClaudeCliUserConfigAction({ accountId, disabled, onApplied }: {
  accountId: string;
  disabled: boolean;
  onApplied: () => void;
}) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [applied, setApplied] = useState(false);
  const error = useModalErrorState();
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const apply = async () => {
    error.clear();
    setApplied(false);
    setBusy(true);
    try {
      await applyClaudeCliAccountToUserConfig(accountId);
      if (!mounted.current) return;
      setApplied(true);
      onApplied();
    } catch (cause) {
      error.report(String(cause).replace(/^Error:\s*/, ''));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="form-group">
      <p className="section-desc">{t('claude.cli.userConfigHint')}</p>
      <button type="button" className="btn btn-secondary" onClick={() => void apply()} disabled={busy || disabled}>
        {busy ? t('common.loading') : t('claude.cli.applyUserConfig')}
      </button>
      {applied && <p className="add-status success" role="status">{t('claude.cli.userConfigApplied')}</p>}
      <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} position="bottom" />
    </div>
  );
}
