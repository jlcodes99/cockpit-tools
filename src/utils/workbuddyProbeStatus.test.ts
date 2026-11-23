/**
 * 探测状态共享键测试：
 * CodeBuddy CN 与 WorkBuddy 同一后端、额度共享，同一账号在两平台的本地 id
 * 后缀（md5(uid||email)）一致 → 记录按共享后缀存储，任一侧写入/恢复/关闭，
 * 另一侧卡片同步显示。
 */
import assert from 'node:assert/strict';
import test from 'node:test';

// node 测试环境没有 localStorage/window，先装最小 stub（模块在调用时才访问它们）
class FakeStorage {
  private map = new Map<string, string>();
  getItem(key: string): string | null {
    return this.map.has(key) ? this.map.get(key)! : null;
  }
  setItem(key: string, value: string): void {
    this.map.set(key, String(value));
  }
  removeItem(key: string): void {
    this.map.delete(key);
  }
}
const storage = new FakeStorage();
(globalThis as any).localStorage = storage;
(globalThis as any).window = {
  addEventListener() {},
  removeEventListener() {},
  dispatchEvent: () => true,
};

const store = await import('./workbuddyProbeStatus');

function limited(accountId: string, model: string, checkedAt: number) {
  return {
    accountId,
    model,
    status: 'rate_limited' as const,
    message: '您的使用量已超出频率限制，将在 2026-11-13T18:00:00+08:00 重置',
    resetAt: Date.parse('2026-11-13T18:00:00+08:00'),
    checkedAt,
  };
}

test('同一账号（同后缀）在两平台的受限提示互相同步', () => {
  storage.removeItem('agtools.workbuddy.probe_status.v1');

  // WorkBuddy 侧探测出限流
  store.setWorkbuddyProbeRecord(limited('workbuddy_0fadf1ec', 'hy4free', 100));

  // CodeBuddy CN 侧同后缀账号立即可见
  assert.equal(store.getWorkbuddyProbeRecords('codebuddy_cn_0fadf1ec').length, 1);

  // CN 侧自动复测确认恢复 → WB 侧同样标注
  store.markWorkbuddyProbeRecovered('codebuddy_cn_0fadf1ec', 'hy4free');
  const [fromWb] = store.getWorkbuddyProbeRecords('workbuddy_0fadf1ec');
  assert.ok(fromWb?.recoveredAt, 'recoveredAt should be visible from the workbuddy side');

  // WB 侧点 × 关闭 → CN 侧同步消失
  store.dismissWorkbuddyProbeRecord('workbuddy_0fadf1ec', 'hy4free');
  assert.equal(store.getWorkbuddyProbeRecords('codebuddy_cn_0fadf1ec').length, 0);
});

test('不同账号（不同后缀）互不影响', () => {
  storage.removeItem('agtools.workbuddy.probe_status.v1');
  store.setWorkbuddyProbeRecord(limited('workbuddy_aaaa', 'hy4free', 1));
  assert.equal(store.getWorkbuddyProbeRecords('workbuddy_bbbb').length, 0);
  assert.equal(store.getWorkbuddyProbeRecords('codebuddy_cn_bbbb').length, 0);
});

test('旧版平台本地记录在下次写入时迁移进共享键', () => {
  storage.removeItem('agtools.workbuddy.probe_status.v1');
  // 预置旧版 WB 本地键（裸 accountId）
  storage.setItem(
    'agtools.workbuddy.probe_status.v1',
    JSON.stringify({
      workbuddy_legacy0: {
        oldmodel: limited('workbuddy_legacy0', 'oldmodel', 1),
      },
    }),
  );

  // 旧键兜底：WB 侧可见
  assert.equal(store.getWorkbuddyProbeRecords('workbuddy_legacy0').length, 1);

  // 新探测写入 → 旧记录一并迁移进共享键，CN 侧随之可见
  store.setWorkbuddyProbeRecord(limited('workbuddy_legacy0', 'newmodel', 2));
  const fromCn = store.getWorkbuddyProbeRecords('codebuddy_cn_legacy0');
  assert.equal(fromCn.length, 2, 'legacy record should migrate alongside the new one');
  assert.ok(fromCn.some((r) => r.model === 'oldmodel'));
  assert.ok(fromCn.some((r) => r.model === 'newmodel'));
});

test('probeSharedIdentityKey 只剥平台前缀', () => {
  assert.equal(store.probeSharedIdentityKey('workbuddy_abc123'), 'abc123');
  assert.equal(store.probeSharedIdentityKey('codebuddy_cn_abc123'), 'abc123');
  assert.equal(store.probeSharedIdentityKey('something_else'), null);
  assert.equal(store.probeSharedIdentityKey('workbuddy_'), null);
});

test('parseProbeResetAt 兼容服务端真实文案格式', () => {
  // 服务端中文文案：空格分隔、无时区、时间在「重置」关键词前面（v1.3.82 曾解析失败）
  const local = store.parseProbeResetAt('您的使用量已超出频率限制，将在 2026-11-13 18:00:00 重置');
  assert.ok(local !== null, 'space-format message must parse');
  assert.equal(new Date(local!).toISOString().slice(0, 16), new Date(2026, 10, 13, 18, 0).toISOString().slice(0, 16));

  // 带时区的 ISO 仍然支持
  const iso = store.parseProbeResetAt('quota exceeded, resets 2026-11-13T18:00:00+08:00');
  assert.equal(iso, Date.parse('2026-11-13T18:00:00+08:00'));

  // 没有时间 → null
  assert.equal(store.parseProbeResetAt('您的使用量已超出频率限制'), null);
  assert.equal(store.parseProbeResetAt(''), null);
});
