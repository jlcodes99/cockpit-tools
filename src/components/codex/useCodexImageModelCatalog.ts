import { useEffect, useMemo, useRef, useState } from 'react';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import { updateCodexLocalAccessImageGenerationModelCatalog } from '../../services/codexLocalAccessService';
import type { CodexLocalAccessState } from '../../types/codexLocalAccess';
import { DEFAULT_CODEX_IMAGE_MODELS, discoverImageModels } from '../../utils/codexImageAccounts';

type Snapshot = { models: string[]; deleted: string[]; updatedAt: number; hydrated: boolean };
let shared: Snapshot | null = null;
const listeners = new Set<(snapshot: Snapshot) => void>();
let catalogQueue: Promise<unknown> = Promise.resolve();
let discoveryFlight: { ids: Set<string>; promise: Promise<CodexLocalAccessState> } | null = null;
function publish(snapshot: Snapshot) { shared = snapshot; listeners.forEach((listener) => listener(snapshot)); }
function fromState(state: CodexLocalAccessState) {
  const collection = state.collection;
  if (!collection?.imageGenerationModels || !collection.imageGenerationDeletedModels) return;
  // 所有调用串行完成，响应是后端权威快照；空数组同样覆盖旧墓碑。
  publish({ models: collection.imageGenerationModels, deleted: collection.imageGenerationDeletedModels, updatedAt: collection.updatedAt, hydrated: true });
}
function requestCatalog(discovered?: string[], add?: string, remove?: string): Promise<CodexLocalAccessState> {
  let expired = false;
  let timer: ReturnType<typeof setTimeout>;
  const timeout = new Error('codex.localAccess.imageGenerationModel.syncFailed');
  const deadline = new Promise<never>((_, reject) => {
    timer = setTimeout(() => { expired = true; reject(timeout); }, 10000);
  });
  const queued = catalogQueue.then(() => {
    // 排队时间计入 deadline；已向 UI 报超时的操作不得迟到启动。
    if (expired) throw timeout;
    // deadline 位于串行链内，释放队列；迟到 IPC 不再发布旧快照。
    return Promise.race([updateCodexLocalAccessImageGenerationModelCatalog(discovered, add, remove), deadline]);
  }).then((state) => { fromState(state); return state; });
  const request = Promise.race([queued, deadline]).finally(() => clearTimeout(timer));
  catalogQueue = request.catch(() => {});
  return request;
}
function syncModels(ids: string[]): Promise<CodexLocalAccessState> {
  if (discoveryFlight) {
    if (ids.every((id) => discoveryFlight!.ids.has(id))) return discoveryFlight.promise;
    return discoveryFlight.promise.then(() => syncModels(ids));
  }
  const promise = requestCatalog(ids);
  const flight = { ids: new Set(ids), promise };
  discoveryFlight = flight;
  void promise.finally(() => { if (discoveryFlight === flight) discoveryFlight = null; }).catch(() => {});
  return promise;
}
/** 多入口共享后台单飞和权威目录；账号刷新只追加发现，删除与恢复以服务器为准。 */
export function useCodexImageModelCatalog(selectedModel?: string, persistedModels?: string[], persistedDeleted?: string[], updatedAt?: number, enabled = true) {
  const accounts = useCodexAccountStore((state) => state.accounts);
  const [snapshot, setSnapshot] = useState<Snapshot>(() => shared ?? {
    models: persistedModels && persistedDeleted ? persistedModels : [selectedModel || DEFAULT_CODEX_IMAGE_MODELS[0]],
    deleted: persistedDeleted ?? [], updatedAt: updatedAt ?? 0, hydrated: Boolean(persistedModels && persistedDeleted),
  });
  const [syncError, setSyncError] = useState('');
  const [attempt, setAttempt] = useState(0);
  const hydratedAttempt = useRef(-1);
  useEffect(() => {
    if (!enabled) return;
    listeners.add(setSnapshot);
    if (shared) setSnapshot(shared);
    return () => { listeners.delete(setSnapshot); };
  }, [enabled]);
  useEffect(() => {
    if (enabled && persistedModels && persistedDeleted && (!shared || (updatedAt !== undefined && updatedAt > shared.updatedAt))) {
      publish({ models: persistedModels, deleted: persistedDeleted, updatedAt: updatedAt ?? 0, hydrated: true });
    }
  }, [persistedModels, persistedDeleted, updatedAt, enabled]);
  const discovered = useMemo(() => enabled ? discoverImageModels(accounts) : [], [accounts, enabled]);
  const pending = discovered.filter((id) => !snapshot.models.includes(id) && !snapshot.deleted.includes(id));
  const discoveryKey = JSON.stringify(pending);
  useEffect(() => {
    if (!enabled || (!pending.length && hydratedAttempt.current === attempt)) return;
    hydratedAttempt.current = attempt;
    let active = true;
    setSyncError('');
    // 每次挂载都回读，即使已发现型号全在缓存中，也要恢复持久化删除标记。
    void syncModels(pending).catch((cause) => { if (active) setSyncError(String(cause).replace(/^Error:\s*/, '')); });
    return () => { active = false; };
    // 目录键消除纯账号刷新带来的重复写入；首次 pending=[] 是一次后台回读。
  }, [discoveryKey, attempt, enabled]);
  const update = async (addModel?: string, removeModel?: string) => {
    await requestCatalog(undefined, addModel, removeModel);
  };
  return { models: snapshot.models.filter((id) => !snapshot.deleted.includes(id)), deleted: snapshot.deleted, ready: enabled && snapshot.hydrated, syncError, retry: () => setAttempt((value) => value + 1), update };
}
