import { invoke } from '@tauri-apps/api/core';
import type { CodexPelicanArtifact, CodexPelicanBatch, CodexPelicanRequest } from '../types/codexPelican';

export const startPelican = (request: CodexPelicanRequest) =>
  invoke<CodexPelicanBatch>('codex_pelican_start', { request });
export const retryPelican = (batchId: string, itemId: string) =>
  invoke<CodexPelicanBatch>('codex_pelican_retry', { batchId, itemId });
export const activePelican = () => invoke<CodexPelicanBatch | null>('codex_pelican_active');
export const getPelican = (batchId: string) => invoke<CodexPelicanBatch>('codex_pelican_get', { batchId });
export const historyPelican = (offset: number, limit = 20) =>
  invoke<{ items: CodexPelicanBatch[]; hasMore: boolean }>('codex_pelican_history', { offset, limit });
export const retentionSettingsPelican = () =>
  invoke<{ days: number }>('codex_pelican_retention_settings');
export const updateRetentionDaysPelican = (days: number) =>
  invoke<{ days: number }>('codex_pelican_update_retention_days', { days });
export const cleanupExpiredPelican = () =>
  invoke<{ deletedCount: number }>('codex_pelican_cleanup_expired');
export const clearAllPelican = () =>
  invoke<{ deletedCount: number }>('codex_pelican_clear_all');
export const cancelPelican = (batchId: string) => invoke<CodexPelicanBatch>('codex_pelican_cancel', { batchId });
export const dismissPelican = (batchId: string) => invoke<void>('codex_pelican_dismiss', { batchId });
export const artifactPelican = (batchId: string, itemId: string) =>
  invoke<CodexPelicanArtifact>('codex_pelican_artifact', { batchId, itemId });
export const previewPelican = (batchId: string, itemId: string) =>
  invoke<void>('codex_pelican_preview', { batchId, itemId });
export const deletePelican = (batchId: string) => invoke<void>('codex_pelican_delete', { batchId });

export const browserPelican = (batchId: string, itemId: string) =>
  invoke<string>('codex_pelican_browser', { batchId, itemId });
