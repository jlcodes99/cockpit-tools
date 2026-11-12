import { invoke } from '@tauri-apps/api/core';

export function createCheckinConfigMigration<T>(
  command: string,
  readLegacy: () => T | undefined,
): () => Promise<T | undefined> {
  let inFlight: Promise<T | undefined> | undefined;
  let complete = false;

  return () => {
    if (complete) return Promise.resolve(undefined);
    if (inFlight) return inFlight;

    const legacyConfig = readLegacy();
    if (!legacyConfig) {
      complete = true;
      return Promise.resolve(undefined);
    }

    inFlight = invoke<T>(command, { legacyConfig })
      .then((config) => {
        complete = true;
        return config;
      })
      .finally(() => {
        inFlight = undefined;
      });
    return inFlight;
  };
}
