import type { CodexQuickConfig } from "../types/codex";
import { getCodexInstanceQuickConfig } from "./codexInstanceService";

export const CODEX_LAUNCH_PREVIEW_CONFIG_TIMEOUT =
  "CODEX_LAUNCH_PREVIEW_CONFIG_TIMEOUT";

interface ConfigServiceOptions {
  loader: (
    instanceId: string,
    apiServicePreview?: boolean,
  ) => Promise<CodexQuickConfig>;
  timeoutMs?: number;
  maxEntries?: number;
}

interface PendingRead {
  promise: Promise<CodexQuickConfig>;
  resolve: (config: CodexQuickConfig) => void;
  reject: (error: unknown) => void;
  timer?: ReturnType<typeof setTimeout>;
}

/** 启动预览只缓存实例配置快照；后台读超时不影响已有快照。 */
export function createCodexLaunchPreviewConfigService({
  loader,
  timeoutMs = 10_000,
  maxEntries = 20,
}: ConfigServiceOptions) {
  const cache = new Map<string, CodexQuickConfig>();
  const pendingReads = new Map<string, PendingRead>();

  const cacheConfig = (instanceId: string, config: CodexQuickConfig) => {
    cache.delete(instanceId);
    cache.set(instanceId, config);
    while (cache.size > Math.max(1, maxEntries)) {
      const oldestId = cache.keys().next().value;
      if (oldestId === undefined) break;
      cache.delete(oldestId);
    }
  };

  const getCachedCodexLaunchPreviewConfig = (
    instanceId: string,
  ): CodexQuickConfig | null => {
    const config = cache.get(instanceId);
    if (!config) return null;
    // 最近打开的实例保留，避免多开实例的历史缓存无限增长。
    cache.delete(instanceId);
    cache.set(instanceId, config);
    return config;
  };

  const finishRead = (instanceId: string, request: PendingRead): boolean => {
    if (pendingReads.get(instanceId) !== request) return false;
    pendingReads.delete(instanceId);
    clearTimeout(request.timer);
    return true;
  };

  const rememberCodexLaunchPreviewConfig = (
    instanceId: string,
    config: CodexQuickConfig,
  ): void => {
    cacheConfig(instanceId, config);
    const request = pendingReads.get(instanceId);
    if (request && finishRead(instanceId, request)) {
      // 保存结果比进行中的读取更新：让等待读取的界面也收到保存后的值。
      request.resolve(config);
    }
  };

  const loadCodexLaunchPreviewConfig = (
    instanceId: string,
    apiServicePreview = false,
  ): Promise<CodexQuickConfig> => {
    const existing = pendingReads.get(instanceId);
    if (existing) return existing.promise;

    let resolve!: PendingRead["resolve"];
    let reject!: PendingRead["reject"];
    const promise = new Promise<CodexQuickConfig>((onResolve, onReject) => {
      resolve = onResolve;
      reject = onReject;
    });
    const request: PendingRead = { promise, resolve, reject };
    pendingReads.set(instanceId, request);
    request.timer = setTimeout(() => {
      if (!finishRead(instanceId, request)) return;
      reject(new Error(CODEX_LAUNCH_PREVIEW_CONFIG_TIMEOUT));
    }, timeoutMs);

    // IPC 无法取消；身份检查防止超时、重试或保存后的旧结果覆盖新状态。
    void Promise.resolve()
      .then(() => loader(instanceId, apiServicePreview))
      .then(
        (config) => {
          if (!finishRead(instanceId, request)) return;
          cacheConfig(instanceId, config);
          resolve(config);
        },
        (error: unknown) => {
          if (finishRead(instanceId, request)) reject(error);
        },
      );
    return promise;
  };

  return {
    getCachedCodexLaunchPreviewConfig,
    loadCodexLaunchPreviewConfig,
    rememberCodexLaunchPreviewConfig,
  };
}

export const {
  getCachedCodexLaunchPreviewConfig,
  loadCodexLaunchPreviewConfig,
  rememberCodexLaunchPreviewConfig,
} = createCodexLaunchPreviewConfigService({ loader: getCodexInstanceQuickConfig });
