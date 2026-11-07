import { useAccountStore } from '../stores/useAccountStore';
import { useCodexAccountStore } from '../stores/useCodexAccountStore';
import { useZedAccountStore } from '../stores/useZedAccountStore';
import { useGitHubCopilotAccountStore } from '../stores/useGitHubCopilotAccountStore';
import { useWindsurfAccountStore } from '../stores/useWindsurfAccountStore';
import { useKiroAccountStore } from '../stores/useKiroAccountStore';
import { useCursorAccountStore } from '../stores/useCursorAccountStore';
import { useCodebuddyAccountStore } from '../stores/useCodebuddyAccountStore';
import { useCodebuddyCnAccountStore } from '../stores/useCodebuddyCnAccountStore';
import { useQoderAccountStore } from '../stores/useQoderAccountStore';
import { useZcodeAccountStore } from '../stores/useZcodeAccountStore';
import { useTraeAccountStore } from '../stores/useTraeAccountStore';
import { useWorkbuddyAccountStore } from '../stores/useWorkbuddyAccountStore';
import { useMinimaxCodeAccountStore } from '../stores/useMinimaxCodeAccountStore';

const ALL_STORES = [
  useAccountStore,
  useCodexAccountStore,
  useZedAccountStore,
  useGitHubCopilotAccountStore,
  useWindsurfAccountStore,
  useKiroAccountStore,
  useCursorAccountStore,
  useCodebuddyAccountStore,
  useCodebuddyCnAccountStore,
  useQoderAccountStore,
  useZcodeAccountStore,
  useTraeAccountStore,
  useWorkbuddyAccountStore,
  useMinimaxCodeAccountStore,
];

export const globalRenameTag = async (oldTag: string, newTag: string) => {
  for (const store of ALL_STORES) {
    const state = store.getState() as any;
    const accounts = state.accounts || [];
    for (const account of accounts) {
      if (account.tags?.includes(oldTag)) {
        const updatedTags = account.tags.map((t: string) => (t === oldTag ? newTag : t));
        const uniqueTags = Array.from(new Set(updatedTags));
        await state.updateAccountTags(account.id, uniqueTags);
      }
    }
  }
};

export const globalDeleteTag = async (targetTag: string) => {
  for (const store of ALL_STORES) {
    const state = store.getState() as any;
    const accounts = state.accounts || [];
    for (const account of accounts) {
      if (account.tags?.includes(targetTag)) {
        const updatedTags = account.tags.filter((t: string) => t !== targetTag);
        await state.updateAccountTags(account.id, updatedTags);
      }
    }
  }
};
