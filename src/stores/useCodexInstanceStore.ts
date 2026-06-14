import * as codexInstanceService from '../services/codexInstanceService';
import type {
  CodexSessionVisibilityRepairSummary,
  CodexInstanceThreadSyncSummary,
  CodexInstanceTargetThreadSyncSummary,
  CodexSessionRecord,
  CodexSessionTokenStats,
  CodexSessionTrashSummary,
  CodexTrashedSessionRecord,
  CodexSessionRestoreSummary,
  CodexSessionRestorePreviewSummary,
  CodexSessionSourceRepairPreviewSummary,
  CodexSessionRolloutBackupBatch,
} from '../types/codex';
import { createInstanceStore, type InstanceStoreState } from './createInstanceStore';

type CodexInstanceStoreState = InstanceStoreState & {
  syncThreadsAcrossInstances: () => Promise<CodexInstanceThreadSyncSummary>;
  syncSessionsToInstance: (
    sessionIds: string[],
    targetInstanceId: string,
  ) => Promise<CodexInstanceTargetThreadSyncSummary>;
  repairSessionVisibilityAcrossInstances: (normalizeSources?: boolean) => Promise<CodexSessionVisibilityRepairSummary>;
  previewSessionVisibilitySourceRepairs: () => Promise<CodexSessionSourceRepairPreviewSummary>;
  listSessionsAcrossInstances: () => Promise<CodexSessionRecord[]>;
  getSessionTokenStatsAcrossInstances: (sessionIds: string[]) => Promise<CodexSessionTokenStats[]>;
  moveSessionsToTrashAcrossInstances: (sessionIds: string[]) => Promise<CodexSessionTrashSummary>;
  listTrashedSessionsAcrossInstances: () => Promise<CodexTrashedSessionRecord[]>;
  previewRestoreSessionsFromTrashAcrossInstances: (sessionIds: string[]) => Promise<CodexSessionRestorePreviewSummary>;
  restoreSessionsFromTrashAcrossInstances: (
    sessionIds: string[],
    forceOverwrite?: boolean,
    normalizeSources?: boolean,
  ) => Promise<CodexSessionRestoreSummary>;
  listSessionRestoreRolloutBackups: () => Promise<CodexSessionRolloutBackupBatch[]>;
  restoreSessionRestoreRolloutBackup: (batchId: string) => Promise<CodexSessionRestoreSummary>;
};

type CodexInstanceStoreHook = {
  (): CodexInstanceStoreState;
  <T>(selector: (state: CodexInstanceStoreState) => T): T;
  getState: () => CodexInstanceStoreState;
  setState: (partial: Partial<CodexInstanceStoreState>) => void;
};

const baseStore = createInstanceStore(codexInstanceService, 'agtools.codex.instances.cache');
const typedBaseStore = baseStore as unknown as CodexInstanceStoreHook;

const syncThreadsAcrossInstances = async (): Promise<CodexInstanceThreadSyncSummary> => {
  const summary = await codexInstanceService.syncThreadsAcrossInstances();
  await typedBaseStore.getState().fetchInstances();
  return summary;
};

const syncSessionsToInstance = async (
  sessionIds: string[],
  targetInstanceId: string,
): Promise<CodexInstanceTargetThreadSyncSummary> => {
  const summary = await codexInstanceService.syncSessionsToInstance(sessionIds, targetInstanceId);
  await typedBaseStore.getState().fetchInstances();
  return summary;
};

const repairSessionVisibilityAcrossInstances = async (
  normalizeSources = false,
): Promise<CodexSessionVisibilityRepairSummary> => {
  const summary = await codexInstanceService.repairSessionVisibilityAcrossInstances(normalizeSources);
  await typedBaseStore.getState().fetchInstances();
  return summary;
};

const previewSessionVisibilitySourceRepairs = async (): Promise<CodexSessionSourceRepairPreviewSummary> => {
  return await codexInstanceService.previewSessionVisibilitySourceRepairs();
};

const listSessionsAcrossInstances = async (): Promise<CodexSessionRecord[]> => {
  return await codexInstanceService.listSessionsAcrossInstances();
};

const getSessionTokenStatsAcrossInstances = async (
  sessionIds: string[],
): Promise<CodexSessionTokenStats[]> => {
  return await codexInstanceService.getSessionTokenStatsAcrossInstances(sessionIds);
};

const moveSessionsToTrashAcrossInstances = async (
  sessionIds: string[],
): Promise<CodexSessionTrashSummary> => {
  const summary = await codexInstanceService.moveSessionsToTrashAcrossInstances(sessionIds);
  await typedBaseStore.getState().fetchInstances();
  return summary;
};

const listTrashedSessionsAcrossInstances = async (): Promise<CodexTrashedSessionRecord[]> => {
  return await codexInstanceService.listTrashedSessionsAcrossInstances();
};

const previewRestoreSessionsFromTrashAcrossInstances = async (
  sessionIds: string[],
): Promise<CodexSessionRestorePreviewSummary> => {
  return await codexInstanceService.previewRestoreSessionsFromTrashAcrossInstances(sessionIds);
};

const restoreSessionsFromTrashAcrossInstances = async (
  sessionIds: string[],
  forceOverwrite = false,
  normalizeSources = false,
): Promise<CodexSessionRestoreSummary> => {
  const summary = await codexInstanceService.restoreSessionsFromTrashAcrossInstances(
    sessionIds,
    forceOverwrite,
    normalizeSources,
  );
  await typedBaseStore.getState().fetchInstances();
  return summary;
};

const listSessionRestoreRolloutBackups = async (): Promise<CodexSessionRolloutBackupBatch[]> => {
  return await codexInstanceService.listSessionRestoreRolloutBackups();
};

const restoreSessionRestoreRolloutBackup = async (batchId: string): Promise<CodexSessionRestoreSummary> => {
  const summary = await codexInstanceService.restoreSessionRestoreRolloutBackup(batchId);
  await typedBaseStore.getState().fetchInstances();
  return summary;
};

typedBaseStore.setState({
  syncThreadsAcrossInstances,
  syncSessionsToInstance,
  repairSessionVisibilityAcrossInstances,
  previewSessionVisibilitySourceRepairs,
  listSessionsAcrossInstances,
  getSessionTokenStatsAcrossInstances,
  moveSessionsToTrashAcrossInstances,
  listTrashedSessionsAcrossInstances,
  previewRestoreSessionsFromTrashAcrossInstances,
  restoreSessionsFromTrashAcrossInstances,
  listSessionRestoreRolloutBackups,
  restoreSessionRestoreRolloutBackup,
});

export const useCodexInstanceStore = typedBaseStore;
