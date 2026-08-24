import { invoke } from "@tauri-apps/api/core";
import { useWindowsOperationDialogStore } from "../stores/useWindowsOperationDialogStore";
import { parseWindowsOperationError } from "./windowsOperationError";
import type { PresentWindowsOperationErrorOptions } from "../stores/useWindowsOperationDialogStore";

export function presentWindowsOperationError(
  options: PresentWindowsOperationErrorOptions,
): boolean {
  const error = parseWindowsOperationError(options.error, {
    operation: options.operation,
    target: options.target,
    summary: options.summary,
  });
  if (!error) return false;

  const authorize =
    error.canElevate && error.pids.length > 0
      ? async () => {
          await invoke<number>("windows_elevated_close_processes", {
            pids: error.pids,
          });
          if (options.retry) {
            await options.retry();
          }
        }
      : undefined;
  const openTarget =
    options.openTarget ??
    (error.target
      ? async () => {
          await invoke("open_local_path", { path: error.target });
        }
      : undefined);

  useWindowsOperationDialogStore.getState().open({
    error,
    retry: options.retry,
    manualContinue: options.manualContinue,
    authorize,
    openTarget,
    onResolved: options.onResolved,
  });
  return true;
}
