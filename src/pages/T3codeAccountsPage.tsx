import type { Page } from '../types/navigation';
import { T3codeAccountLinkPage } from './T3codeAccountLinkPage';

interface T3codeAccountsPageProps {
  onNavigate?: (page: Page) => void;
}

export function T3codeAccountsPage({ onNavigate }: T3codeAccountsPageProps = {}) {
  return <T3codeAccountLinkPage onNavigate={onNavigate} />;
}
