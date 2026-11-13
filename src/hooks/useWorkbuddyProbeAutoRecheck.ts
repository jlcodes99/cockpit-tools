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
  setWorkbuddyProbeRecord,
  type WorkbuddyProbePlatform,
} from '../utils/workbuddyProbeStatus';

const SCAN_INTERVAL_MS = 60_000;
const FIRST_SCAN_DELAY_MS = 8_000;
/** 同一条记录两次复测的最小间隔（协议/网络异常时避免密集重试） */
const RETRY_THROTTLE_MS = 5 * 60_000;
/** 「已恢复」提示保留时长 */
const RECOVERED_TTL_MS = 24 * 60 * 60 * 1000;

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
  const pendingRef = useRef(new Set<string>());

  useEffect(() => {
    let stopped = false;

    const recheckOne = async (accountId: string, model: string) => {
      const currentPlatform = platformRef.current;
      const key = `${currentPlatform}:${accountId}:${model}`;
      pendingRef.current.add(key);
      try {
        const probe =
          currentPlatform === 'codebuddy_cn' ? probeCodebuddyCnModel : probeWorkbuddyModel;
        const result = await probe(accountId, model, false);
        if (result.status === 'available') {
          markWorkbuddyProbeRecovered(accountId, model, currentPlatform);
        } else if (result.status === 'rate_limited') {
          setWorkbuddyProbeRecord(
            {
              accountId,
              model,
              status: 'rate_limited',
              message: result.message,
              resetAt: parseProbeResetAt(result.message),
              checkedAt: Date.now(),
            },
            currentPlatform,
          );
        } else {
          bumpWorkbuddyProbeCheckedAt(accountId, model, currentPlatform);
        }
      } catch {
        /* 命令失败（如 PROBE_BUSY）：忽略，等下一轮 */
      } finally {
        pendingRef.current.delete(key);
      }
    };

    const scan = async () => {
      if (stopped || busyRef.current) return;
      const currentPlatform = platformRef.current;
      expireWorkbuddyProbeRecoveries(currentPlatform, RECOVERED_TTL_MS);
      const now = Date.now();
      const due: Array<{ accountId: string; model: string }> = [];
      for (const account of accountsRef.current) {
        for (const record of getWorkbuddyProbeRecords(account.id, currentPlatform)) {
          if (record.status !== 'rate_limited' || record.recoveredAt) continue;
          // 没有明确恢复时间的记录无法判断何时复测，交给手动探测
          if (record.resetAt === null || record.resetAt > now) continue;
          if (now - record.checkedAt < RETRY_THROTTLE_MS) continue;
          if (pendingRef.current.has(`${currentPlatform}:${account.id}:${record.model}`)) continue;
          due.push({ accountId: account.id, model: record.model });
        }
      }
      if (due.length === 0) return;
      busyRef.current = true;
      try {
        for (const item of due) {
          if (stopped) return;
          await recheckOne(item.accountId, item.model);
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
