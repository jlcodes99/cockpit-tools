import { invoke } from "@tauri-apps/api/core";
import { useWindowsOperationDialogStore } from "../stores/useWindowsOperationDialogStore";
import { parseWindowsOperationError } from "./windowsOperationError";
import type { PresentWindowsOperationErrorOptions } from "../stores/useWindowsOperationDialogStore";

/**
 * 商店版 Codex 路径失效（如商店包更新后旧目录残留）时，重新探测一次路径并重试。
 *
 * `detect_app_path` 带 force 会同步写回用户配置，因此重试时会用新路径启动。
 */
async function repairCodexStoreLaunchPath(
  retry?: () => void | Promise<void>,
): Promise<void> {
  await invoke<string | null>("detect_app_path", { app: "codex", force: true });
  await retry?.();
}

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
  const repair =
    options.repair ??
    (error.code === "codex_store_launch_blocked"
      ? async () => {
          await repairCodexStoreLaunchPath(options.retry);
        }
      : undefined);

  useWindowsOperationDialogStore.getState().open({
    error,
    retry: options.retry,
    manualContinue: options.manualContinue,
    authorize,
    repair,
    openTarget,
    onResolved: options.onResolved,
  });
  return true;
}
