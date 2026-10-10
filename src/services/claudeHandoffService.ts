import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface ClaudeHandoffProgress {
  stage: string;
  completed: number;
  total: number;
}

export function watchClaudeHandoffProgress(callback: (progress: ClaudeHandoffProgress) => void) {
  return listen<ClaudeHandoffProgress>('claude-handoff-progress', event => callback(event.payload));
}
import type {
  ClaudeHandoffApplyInput,
  ClaudeHandoffMutationResult,
  ClaudeHandoffPair,
  ClaudeHandoffPreview,
  ClaudeHandoffStatus,
} from '../types/claudeHandoff';

export function getClaudeHandoffStatus(): Promise<ClaudeHandoffStatus> {
  return invoke('claude_handoff_status', {});
}

export function previewClaudeHandoff(pair: ClaudeHandoffPair): Promise<ClaudeHandoffPreview> {
  return invoke('claude_handoff_preview', { ...pair });
}

export function applyClaudeHandoff(input: ClaudeHandoffApplyInput): Promise<ClaudeHandoffMutationResult> {
  return invoke('claude_handoff_apply', { ...input });
}

export function applyAndSwitchClaudeHandoff(input: ClaudeHandoffApplyInput): Promise<ClaudeHandoffMutationResult> {
  return invoke('claude_handoff_apply_and_switch', { ...input });
}

export function rollbackClaudeHandoff(runId: string): Promise<ClaudeHandoffMutationResult> {
  return invoke('claude_handoff_rollback', { runId });
}
