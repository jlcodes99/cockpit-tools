import { invoke } from '@tauri-apps/api/core';
import { parseUpdaterReleaseNotes } from '../utils/updaterReleaseNotes';

/** Save the updater's existing notes in the background, without delaying download. */
export function savePendingUpdateNotes(version: string, body?: string) {
  const { releaseNotes, releaseNotesZh } = parseUpdaterReleaseNotes(body);
  void invoke('save_pending_update_notes', { version, releaseNotes, releaseNotesZh })
    .catch((error) => console.error('Failed to save update notes:', error));
}
