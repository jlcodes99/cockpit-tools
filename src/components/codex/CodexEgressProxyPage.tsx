import { CodexProxyWorkspace } from './CodexProxyWorkspace';
import { CodexProxyWorkspaceProvider } from './CodexProxyWorkspaceContext';
import type { CodexAccount } from '../../types/codex';

interface Props {
  accounts: CodexAccount[];
  accountId?: string | null;
  resolveDisplayName: (account: CodexAccount) => string;
  onBack: () => void;
}

/** Resources remain accessible before the first eligible account is added. */
export function CodexEgressProxyPage({ accounts, accountId, onBack }: Props) {
  return <CodexProxyWorkspaceProvider accounts={accounts} accountId={accountId}>
    <CodexProxyWorkspace onBack={onBack} />
  </CodexProxyWorkspaceProvider>;
}
