/**
 * 模型探测结果本地持久化：按「账号 + 模型」记录最近一次不可用（限流/错误）的结果，
 * 用于在账号卡片上展示最近一次探测的失败信息，不代表模型当前必然不可用。
 *
 * 存储：localStorage（纯前端状态，不写账号文件）。
 * 清除：探测成功（available）时删除该模型记录；用户在卡片上点 × 也会删除（下次探测会重新写入）。
 */

export type WorkbuddyProbeStatusKind = 'rate_limited' | 'error';

/** 探测平台：WorkBuddy 与 CodeBuddy CN 额度共享，但账号体系各自独立 */
export type WorkbuddyProbePlatform = 'workbuddy' | 'codebuddy_cn';

export interface WorkbuddyProbeRecord {
  accountId: string;
  model: string;
  status: WorkbuddyProbeStatusKind;
  /** 服务端返回的错误/限流文案 */
  message: string;
  /** 从文案里解析出的额度恢复时间（没有则为 null） */
  resetAt: number | null;
  /** 探测时间 */
  checkedAt: number;
  /** 恢复时间过后自动复测确认已恢复的时间（null/缺省 = 尚未复测） */
  recoveredAt?: number | null;
}

type ProbeStore = Record<string, Record<string, WorkbuddyProbeRecord>>;

const STORAGE_KEY = 'agtools.workbuddy.probe_status.v1';
const CHANGE_EVENT = 'agtools:workbuddy-probe-status';

/** WorkBuddy 沿用 accountId 作 key（兼容旧数据）；其它平台加前缀隔离。 */
function storageKey(accountId: string, platform: WorkbuddyProbePlatform): string {
  return platform === 'workbuddy' ? accountId : `${platform}:${accountId}`;
}

let cachedRaw: string | null | undefined;
let cachedStore: ProbeStore = {};

function readStore(): ProbeStore {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === cachedRaw) return cachedStore;
    const parsed = raw ? JSON.parse(raw) : {};
    cachedRaw = raw;
    cachedStore = parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed : {};
    return cachedStore;
  } catch {
    return {};
  }
}

function writeStore(store: ProbeStore) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(store));
  } catch {
    /* localStorage 不可用时忽略 */
  }
  try {
    window.dispatchEvent(new CustomEvent(CHANGE_EVENT));
  } catch {
    /* SSR/测试环境忽略 */
  }
}

/** 仅解析带时区且明确说明重置或恢复的 ISO 时间，不猜测服务端时区。 */
export function parseProbeResetAt(message: string): number | null {
  if (!message) return null;
  const match = message.match(/(?:reset|恢复|重置).*?(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:Z|[+-]\d{2}:\d{2}))/i);
  const time = match ? Date.parse(match[1]) : NaN;
  return Number.isFinite(time) ? time : null;
}

export function setWorkbuddyProbeRecord(
  record: WorkbuddyProbeRecord,
  platform: WorkbuddyProbePlatform = 'workbuddy',
) {
  const store = readStore();
  const key = storageKey(record.accountId, platform);
  writeStore({
    ...store,
    [key]: { ...store[key], [record.model]: record },
  });
}

/** 探测成功时清除该模型的不可用记录。 */
export function clearWorkbuddyProbeRecord(
  accountId: string,
  model: string,
  platform: WorkbuddyProbePlatform = 'workbuddy',
) {
  const store = readStore();
  const key = storageKey(accountId, platform);
  const perAccount = store[key];
  if (!perAccount || typeof perAccount !== 'object' || Array.isArray(perAccount) || !Object.prototype.hasOwnProperty.call(perAccount, model)) return;
  delete perAccount[model];
  if (Object.keys(perAccount).length === 0) delete store[key];
  writeStore(store);
}

/** 用户点 × 关闭提示（删除记录，下次探测再写入）。 */
export function dismissWorkbuddyProbeRecord(
  accountId: string,
  model: string,
  platform: WorkbuddyProbePlatform = 'workbuddy',
) {
  clearWorkbuddyProbeRecord(accountId, model, platform);
}

/** 某账号最近的探测失败记录（按探测时间倒序）。 */
export function getWorkbuddyProbeRecords(
  accountId: string,
  platform: WorkbuddyProbePlatform = 'workbuddy',
): WorkbuddyProbeRecord[] {
  const perAccount = readStore()[storageKey(accountId, platform)];
  if (!perAccount || typeof perAccount !== 'object' || Array.isArray(perAccount)) return [];
  return Object.values(perAccount).filter((row) => row && typeof row.model === 'string' && typeof row.checkedAt === 'number' && (row.status === 'rate_limited' || row.status === 'error')).sort((a, b) => b.checkedAt - a.checkedAt);
}

/**
 * 自动复测确认恢复：保留原记录（含恢复时间），仅补记 recoveredAt，
 * 让卡片提示变为「…，已恢复」，由用户自行关闭或过期清理。
 */
export function markWorkbuddyProbeRecovered(
  accountId: string,
  model: string,
  platform: WorkbuddyProbePlatform = 'workbuddy',
) {
  const store = readStore();
  const key = storageKey(accountId, platform);
  const perAccount = store[key];
  const record = perAccount?.[model];
  if (!record) return; // 用户已关闭提示：不再重建
  if (record.recoveredAt) return;
  writeStore({
    ...store,
    [key]: { ...perAccount, [model]: { ...record, recoveredAt: Date.now() } },
  });
}

/** 复测失败（协议/网络错误）时仅推进探测时间，避免下一轮立即重试。 */
export function bumpWorkbuddyProbeCheckedAt(
  accountId: string,
  model: string,
  platform: WorkbuddyProbePlatform = 'workbuddy',
) {
  const store = readStore();
  const key = storageKey(accountId, platform);
  const perAccount = store[key];
  const record = perAccount?.[model];
  if (!record) return;
  writeStore({
    ...store,
    [key]: { ...perAccount, [model]: { ...record, checkedAt: Date.now() } },
  });
}

/** 清理超过 ttl 的「已恢复」记录（提示意义已尽）。返回是否有变更。 */
export function expireWorkbuddyProbeRecoveries(
  platform: WorkbuddyProbePlatform = 'workbuddy',
  ttlMs = 24 * 60 * 60 * 1000,
): boolean {
  const store = readStore();
  const now = Date.now();
  let changed = false;
  const next: ProbeStore = { ...store };
  for (const [key, perAccount] of Object.entries(store)) {
    if (!keyMatchesPlatform(key, platform)) continue;
    for (const [model, record] of Object.entries(perAccount)) {
      if (record?.recoveredAt && now - record.recoveredAt > ttlMs) {
        const remainder = { ...next[key] };
        delete remainder[model];
        if (Object.keys(remainder).length === 0) delete next[key];
        else next[key] = remainder;
        changed = true;
      }
    }
  }
  if (changed) writeStore(next);
  return changed;
}

/** WorkBuddy 的 key 是裸 accountId；其它平台带前缀。 */
function keyMatchesPlatform(key: string, platform: WorkbuddyProbePlatform): boolean {
  return platform === 'workbuddy' ? !key.includes(':') : key.startsWith(`${platform}:`);
}

/** 订阅探测结果变化（跨组件刷新）。返回取消订阅函数。 */
export function subscribeWorkbuddyProbeStatus(listener: () => void): () => void {
  window.addEventListener(CHANGE_EVENT, listener);
  // 其他窗口/标签页修改 localStorage 时同步
  const onStorage = (event: StorageEvent) => {
    if (event.key === STORAGE_KEY) listener();
  };
  window.addEventListener('storage', onStorage);
  return () => {
    window.removeEventListener(CHANGE_EVENT, listener);
    window.removeEventListener('storage', onStorage);
  };
}
