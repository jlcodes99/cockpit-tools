const CODEX_IMPORT_SYNC_API_SERVICE_STORAGE_KEY =
  "agtools.codex.import.sync_api_service.v1";

export const resolveCodexImportSyncApiServicePreference = (
  storedValue: string | null,
): boolean => {
  if (storedValue === null) return true;
  return storedValue === "true";
};

export const readCodexImportSyncApiService = (): boolean => {
  try {
    return resolveCodexImportSyncApiServicePreference(
      localStorage.getItem(CODEX_IMPORT_SYNC_API_SERVICE_STORAGE_KEY),
    );
  } catch {
    return true;
  }
};

export const writeCodexImportSyncApiService = (enabled: boolean): void => {
  try {
    localStorage.setItem(
      CODEX_IMPORT_SYNC_API_SERVICE_STORAGE_KEY,
      enabled ? "true" : "false",
    );
  } catch {
    // Keep the in-memory choice when storage is unavailable.
  }
};
