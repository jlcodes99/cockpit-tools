import { previewDurableUiSetting, readDurableUiSetting } from './durableUiSettings';

const CODEX_IMPORT_SYNC_API_SERVICE_STORAGE_KEY =
  "agtools.codex.import.sync_api_service.v1";

export const readCodexImportSyncApiService = (): boolean => {
  try {
    return readDurableUiSetting(CODEX_IMPORT_SYNC_API_SERVICE_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
};

export const writeCodexImportSyncApiService = (enabled: boolean): void => {
  previewDurableUiSetting(CODEX_IMPORT_SYNC_API_SERVICE_STORAGE_KEY, enabled ? "true" : "false");
};
