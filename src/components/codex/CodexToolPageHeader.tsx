import type { ReactNode } from 'react';
import { ArrowLeft } from 'lucide-react';
import { useTranslation } from 'react-i18next';

interface Props {
  title: string;
  titleId: string;
  description: string;
  icon: ReactNode;
  onBack: () => void;
}

/** Shared visual scale for the dedicated tools under Codex's More navigation. */
export function CodexToolPageHeader({ title, titleId, description, icon, onBack }: Props) {
  const { t } = useTranslation();
  return <header className="codex-tool-header">
    <button type="button" className="btn btn-secondary codex-tool-back" onClick={onBack}>
      <ArrowLeft size={15} />{t('codex.proxy.backToAccounts')}
    </button>
    <div className="codex-tool-heading">
      <div className="codex-tool-heading-icon" aria-hidden="true">{icon}</div>
      <div><h1 id={titleId}>{title}</h1><p>{description}</p></div>
    </div>
    <div className="codex-tool-header-art" aria-hidden="true"><i /><i /><i /><span /></div>
  </header>;
}
