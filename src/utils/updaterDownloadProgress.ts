import type { DownloadEvent } from '@tauri-apps/plugin-updater';

/** Count each download event once, before React can replay a state updater. */
export function createUpdaterDownloadProgress(): (event: DownloadEvent) => number {
  let downloaded = 0;
  let contentLength = 0;
  let progress = 0;

  return (event) => {
    if (event.event === 'Started') {
      downloaded = 0;
      contentLength = event.data.contentLength ?? 0;
      progress = 0;
    } else if (event.event === 'Progress') {
      downloaded += event.data.chunkLength;
      progress = contentLength > 0
        ? Math.min(100, Math.round((downloaded / contentLength) * 100))
        : Math.min(95, progress + 1);
    } else {
      progress = 100;
    }
    return progress;
  };
}
