/**
 * 模型探测结果本地持久化：按「账号 + 模型」记录最近一次不可用（限流/错误）的结果，
 * 用于在账号卡片上展示最近一次探测的失败信息，不代表模型当前必然不可用。
 *
 * 存储：localStorage（纯前端状态，不写账号文件）。
 * 清除：探测成功（available）时删除该模型记录；用户在卡片上点 × 也会删除（下次探测会重新写入）。
 */

export type WorkbuddyProbeStatusKind = 'rate_limited' | 'error';

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
}

type ProbeStore = Record<string, Record<string, WorkbuddyProbeRecord>>;

const STORAGE_KEY = 'agtools.workbuddy.probe_status.v1';
const CHANGE_EVENT = 'agtools:workbuddy-probe-status';

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

export function setWorkbuddyProbeRecord(record: WorkbuddyProbeRecord) {
  const store = readStore();
  writeStore({
    ...store,
    [record.accountId]: { ...store[record.accountId], [record.model]: record },
  });
}

/** 探测成功时清除该模型的不可用记录。 */
export function clearWorkbuddyProbeRecord(accountId: string, model: string) {
  const store = readStore();
  const perAccount = store[accountId];
  if (!perAccount || typeof perAccount !== 'object' || Array.isArray(perAccount) || !Object.prototype.hasOwnProperty.call(perAccount, model)) return;
  delete perAccount[model];
  if (Object.keys(perAccount).length === 0) delete store[accountId];
  writeStore(store);
}

/** 用户点 × 关闭提示（删除记录，下次探测再写入）。 */
export function dismissWorkbuddyProbeRecord(accountId: string, model: string) {
  clearWorkbuddyProbeRecord(accountId, model);
}

/** 某账号最近的探测失败记录（按探测时间倒序）。 */
export function getWorkbuddyProbeRecords(accountId: string): WorkbuddyProbeRecord[] {
  const perAccount = readStore()[accountId];
  if (!perAccount || typeof perAccount !== 'object' || Array.isArray(perAccount)) return [];
  return Object.values(perAccount).filter((row) => row && typeof row.model === 'string' && typeof row.checkedAt === 'number' && (row.status === 'rate_limited' || row.status === 'error')).sort((a, b) => b.checkedAt - a.checkedAt);
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
