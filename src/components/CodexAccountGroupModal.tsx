/**
 * Codex 账号分组管理弹窗（重定向至通用 AccountGroupModal）
 */

import { AccountGroupModal, AddToGroupModal } from './AccountGroupModal';
import type { AccountGroup } from '../services/platformGroupService';

export type CodexAccountGroup = AccountGroup;

export interface CodexAccountGroupModalProps {
  isOpen: boolean;
  onClose: () => void;
  onGroupsChanged: () => Promise<void> | void;
  onAddAccounts?: (group: AccountGroup) => void;
}

export const CodexAccountGroupModal = ({
  isOpen,
  onClose,
  onGroupsChanged,
  onAddAccounts,
}: CodexAccountGroupModalProps) => {
  return (
    <AccountGroupModal
      isOpen={isOpen}
      onClose={onClose}
      onGroupsChanged={onGroupsChanged}
      platform="codex"
      onAddAccounts={onAddAccounts}
    />
  );
};

export interface CodexAddToGroupModalProps {
  isOpen: boolean;
  onClose: () => void;
  accountIds: string[];
  sourceGroupId?: string;
  onAdded: () => Promise<void> | void;
}

export const CodexAddToGroupModal = ({
  isOpen,
  onClose,
  accountIds,
  sourceGroupId,
  onAdded,
}: CodexAddToGroupModalProps) => {
  return (
    <AddToGroupModal
      isOpen={isOpen}
      onClose={onClose}
      platform="codex"
      accountIds={accountIds}
      sourceGroupId={sourceGroupId}
      onAdded={onAdded}
    />
  );
};

export default CodexAccountGroupModal;
