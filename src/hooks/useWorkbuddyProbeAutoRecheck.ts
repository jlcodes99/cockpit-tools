/**
 * 模型受限提示的自动复测：
 *
 * - 扫描当前账号列表的探测记录，找「限流且恢复时间已过、尚未确认恢复」的记录
 * - 按平台复用对应探测命令，真实发送一句 "Reply with OK."（并发 1，逐条串行）
 * - 探测可用 → markWorkbuddyProbeRecovered（卡片提示追加「已恢复」）
 * - 仍在限流 → 用新文案/新恢复时间覆盖记录；其它失败 → 仅推进 checkedAt 节流
 * - 「已恢复」记录展示 24h 后自动清理
 */
import { useEffect, useRef } from 'react';
import {
  probeCodebuddyCnModel,
  probeWorkbuddyModel,
} from '../services/workbuddyService';
import {
  bumpWorkbuddyProbeCheckedAt,
  expireWorkbuddyProbeRecoveries,
  getWorkbuddyProbeRecords,
  markWorkbuddyProbeRecovered,
  parseProbeResetAt,
  probeSharedIdentityKey,
  setWorkbuddyProbeRecord,
  type WorkbuddyProbePlatform,
} from '../utils/workbuddyProbeStatus';

const SCAN_INTERVAL_MS = 60_000;
const FIRST_SCAN_DELAY_MS = 8_000;
/** 同一条记录两次复测的最小间隔（协议/网络异常时避免密集重试） */
const RETRY_THROTTLE_MS = 5 * 60_000;
/** 「已恢复」提示保留时长 */
const RECOVERED_TTL_MS = 24 * 60 * 60 * 1000;
/** 没解析出恢复时间的记录：用更长的间隔自愈复测（刷新文案与时间） */
const NO_RESET_RETRY_MS = 30 * 60_000;

const pendingRechecks = new Set<string>();

/** 只需要 id，任意账号对象都可用 */
interface AccountLike {
  id: string;
}

export function useWorkbuddyProbeAutoRecheck(
  accounts: AccountLike[],
  platform: WorkbuddyProbePlatform = 'workbuddy',
) {
  const accountsRef = useRef<AccountLike[]>(accounts);
  accountsRef.current = accounts;
  const platformRef = useRef(platform);
  platformRef.current = platform;
  const busyRef = useRef(false);

  useEffect(() => {
    let stopped = false;

    const recheckOne = async (accountId: string, model: string, checkedAt: number) => {
      const currentPlatform = platformRef.current;
      const key = JSON.stringify([probeSharedIdentityKey(accountId) ?? accountId, model]);
      if (pendingRechecks.has(key)) return;
      const currentRecord = () => getWorkbuddyProbeRecords(accountId).find((record) => record.model === model);
      const original = currentRecord();
      if (!original || original.checkedAt !== checkedAt || original.status !== 'rate_limited' || original.recoveredAt) return;
      const stillCurrent = () => {
        const latest = currentRecord();
        return latest?.checkedAt === original.checkedAt
          && latest.status === original.status
          && latest.message === original.message
          && !latest.recoveredAt;
      };

      pendingRechecks.add(key);
      try {
        const probe =
          currentPlatform === 'codebuddy_cn' ? probeCodebuddyCnModel : probeWorkbuddyModel;
        const result = await probe(accountId, model, false);
        if (!stillCurrent()) return;
        if (result.status === 'available') {
          markWorkbuddyProbeRecovered(accountId, model);
        } else if (result.status === 'rate_limited') {
          setWorkbuddyProbeRecord({
            accountId,
            model,
            status: 'rate_limited',
            message: result.message,
            resetAt: parseProbeResetAt(result.message),
            checkedAt: Date.now(),
          });
        } else {
          bumpWorkbuddyProbeCheckedAt(accountId, model);
        }
      } catch {
        if (stillCurrent()) bumpWorkbuddyProbeCheckedAt(accountId, model);
      } finally {
        pendingRechecks.delete(key);
      }
    };

    const scan = async () => {
      if (stopped || busyRef.current) return;
      expireWorkbuddyProbeRecoveries(RECOVERED_TTL_MS);
      const now = Date.now();
      const due: Array<{ accountId: string; model: string; checkedAt: number }> = [];
      for (const account of accountsRef.current) {
        for (const record of getWorkbuddyProbeRecords(account.id)) {
          if (record.status !== 'rate_limited' || record.recoveredAt) continue;
          if (record.resetAt !== null) {
            if (record.resetAt > now) continue;
            if (now - record.checkedAt < RETRY_THROTTLE_MS) continue;
          } else {
            // 没解析出恢复时间的旧记录（如旧版解析回归丢失的时间）：低频自愈复测刷新
            if (now - record.checkedAt < NO_RESET_RETRY_MS) continue;
          }
          due.push({ accountId: account.id, model: record.model, checkedAt: record.checkedAt });
        }
      }
      if (due.length === 0) return;
      busyRef.current = true;
      try {
        for (const item of due) {
          if (stopped) return;
          await recheckOne(item.accountId, item.model, item.checkedAt);
        }
      } finally {
        busyRef.current = false;
      }
    };

    const first = window.setTimeout(() => { void scan(); }, FIRST_SCAN_DELAY_MS);
    const timer = window.setInterval(() => { void scan(); }, SCAN_INTERVAL_MS);
    return () => {
      stopped = true;
      window.clearTimeout(first);
      window.clearInterval(timer);
    };
  }, []);
}
