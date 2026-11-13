/**
 * 模型探测结果本地持久化：按「账号 + 模型」记录最近一次不可用（限流/错误）的结果，
 * 用于在账号卡片上展示最近一次探测的失败信息，不代表模型当前必然不可用。
 *
 * 存储：localStorage（纯前端状态，不写账号文件）。
 * 跨平台同步：CodeBuddy CN 与 WorkBuddy 共享同一后端与额度，同一账号在两平台的
 * 本地 id 都是 `平台前缀_ + md5(uid||email)`，后缀完全一致。因此记录以「后缀哈希」
 * 作共享键存储——任一平台探测出的受限/恢复，两边的账号卡片同时显示，互相同步。
 * 清除：探测成功（available）时删除该模型记录；用户在卡片上点 × 也会删除（下次探测会重新写入）。
 */

export type WorkbuddyProbeStatusKind = 'rate_limited' | 'error';

/** 探测平台：WorkBuddy 与 CodeBuddy CN 同一后端、额度共享 */
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

/**
 * 跨平台共享身份键：账号 id 去掉平台前缀后的 md5 后缀
 * （workbuddy_0fadf… ↔ codebuddy_cn_0fadf… 是同一账号）。无法识别时返回 null。
 */
export function probeSharedIdentityKey(accountId: string): string | null {
  const suffix = accountId
    .replace(/^workbuddy_/, '')
    .replace(/^codebuddy_cn_/, '');
  return suffix && suffix !== accountId ? suffix : null;
}

/** 旧版存储键：WorkBuddy 用裸 accountId，CN 用 `codebuddy_cn:<accountId>`。 */
function legacyStorageKeys(accountId: string): string[] {
  return [`codebuddy_cn:${accountId}`, accountId];
}

/** 记录实际存放的位置：优先共享键，其次旧键（兼容升级前的数据）。 */
function resolveExistingKey(store: ProbeStore, accountId: string): string | null {
  const identity = probeSharedIdentityKey(accountId);
  if (identity && store[identity]) return identity;
  for (const key of legacyStorageKeys(accountId)) {
    if (store[key]) return key;
  }
  return null;
}

/** 写入目标键：有共享身份就用共享键（首次写入时把旧平台本地记录一并迁移过来）。 */
function writeKeyFor(store: ProbeStore, accountId: string): string {
  const identity = probeSharedIdentityKey(accountId);
  if (!identity) return accountId;
  if (store[identity]) return identity;
  const legacyKey = resolveExistingKey(store, accountId);
  if (legacyKey) {
    // 迁移旧记录到共享键（保留旧键不动，读取时共享键优先）
    store[identity] = { ...store[legacyKey] };
  }
  return identity;
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

/**
 * 从限流文案解析额度恢复时间。
 * 服务端真实文案是「将在 2026-11-13 18:00:00 重置」（空格分隔、无时区、时间在关键词前），
 * 按本地时区解释；也兼容带时区的 ISO 格式（2026-11-13T18:00:00+08:00）。
 */
export function parseProbeResetAt(message: string): number | null {
  if (!message) return null;
  // ISO 带时区：按时区解析
  const iso = message.match(/(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:Z|[+-]\d{2}:\d{2}))/);
  if (iso) {
    const time = Date.parse(iso[1]);
    if (Number.isFinite(time)) return time;
  }
  // 本地时间格式（v1.3.82 曾收紧为「关键词在前 + 必须带时区 ISO」导致时间丢失，此处恢复宽松匹配）
  const local = message.match(/(\d{4})-(\d{2})-(\d{2})[ T](\d{1,2}):(\d{2})(?::(\d{2}))?/);
  if (local) {
    const time = new Date(
      Number(local[1]),
      Number(local[2]) - 1,
      Number(local[3]),
      Number(local[4]),
      Number(local[5]),
      Number(local[6] ?? 0),
    ).getTime();
    if (Number.isFinite(time)) return time;
  }
  return null;
}

export function setWorkbuddyProbeRecord(record: WorkbuddyProbeRecord) {
  const store = readStore();
  const key = writeKeyFor(store, record.accountId);
  writeStore({
    ...store,
    [key]: { ...store[key], [record.model]: record },
  });
}

/** 探测成功时清除该模型的不可用记录（两平台共享：任一侧成功即双侧消失）。 */
export function clearWorkbuddyProbeRecord(accountId: string, model: string) {
  const store = readStore();
  const key = resolveExistingKey(store, accountId);
  if (!key) return;
  const perAccount = store[key];
  if (!perAccount || typeof perAccount !== 'object' || Array.isArray(perAccount) || !Object.prototype.hasOwnProperty.call(perAccount, model)) return;
  delete perAccount[model];
  if (Object.keys(perAccount).length === 0) delete store[key];
  writeStore(store);
}

/** 用户点 × 关闭提示（删除记录，下次探测再写入；两平台同步消失）。 */
export function dismissWorkbuddyProbeRecord(accountId: string, model: string) {
  clearWorkbuddyProbeRecord(accountId, model);
}

/** 某账号最近的探测失败记录（按探测时间倒序；读取共享键，旧键兜底）。 */
export function getWorkbuddyProbeRecords(accountId: string): WorkbuddyProbeRecord[] {
  const store = readStore();
  const key = resolveExistingKey(store, accountId);
  if (!key) return [];
  const perAccount = store[key];
  if (!perAccount || typeof perAccount !== 'object' || Array.isArray(perAccount)) return [];
  return Object.values(perAccount).filter((row) => row && typeof row.model === 'string' && typeof row.checkedAt === 'number' && (row.status === 'rate_limited' || row.status === 'error')).sort((a, b) => b.checkedAt - a.checkedAt);
}

/**
 * 自动复测确认恢复：保留原记录（含恢复时间），仅补记 recoveredAt，
 * 让卡片提示变为「…，已恢复」，由用户自行关闭或过期清理（两平台同步显示）。
 */
export function markWorkbuddyProbeRecovered(accountId: string, model: string) {
  const store = readStore();
  const key = resolveExistingKey(store, accountId);
  const perAccount = key ? store[key] : undefined;
  const record = perAccount?.[model];
  if (!record) return; // 用户已关闭提示：不再重建
  if (record.recoveredAt) return;
  writeStore({
    ...store,
    [key!]: { ...perAccount, [model]: { ...record, recoveredAt: Date.now() } },
  });
}

/** 复测失败（协议/网络错误）时仅推进探测时间，避免下一轮立即重试。 */
export function bumpWorkbuddyProbeCheckedAt(accountId: string, model: string) {
  const store = readStore();
  const key = resolveExistingKey(store, accountId);
  const perAccount = key ? store[key] : undefined;
  const record = perAccount?.[model];
  if (!record) return;
  writeStore({
    ...store,
    [key!]: { ...perAccount, [model]: { ...record, checkedAt: Date.now() } },
  });
}

/** 清理超过 ttl 的「已恢复」记录（提示意义已尽）。返回是否有变更。 */
export function expireWorkbuddyProbeRecoveries(
  ttlMs = 24 * 60 * 60 * 1000,
): boolean {
  const store = readStore();
  const now = Date.now();
  let changed = false;
  const next: ProbeStore = { ...store };
  for (const [key, perAccount] of Object.entries(store)) {
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

/** 订阅探测结果变化（跨组件刷新）。返回取消订阅函数。 */
export function subscribeWorkbuddyProbeStatus(listener: () => void): () => void {
  window.addEventListener(CHANGE_EVENT, listener);
  // 其他窗口/标签页修改 localStorage 时同步
  const onStorage = (event: StorageEvent) => {
    if (event.key === STORAGE_KEY) listener();
  };
  window.addEventListener('storage', onStorage);
  return () => {
    window.removeEventListener('storage', onStorage);
    window.removeEventListener(CHANGE_EVENT, listener);
  };
}
