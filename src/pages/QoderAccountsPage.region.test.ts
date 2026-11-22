import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  consumeQueuedExternalProviderImportForPlatform,
  queueExternalProviderImport,
} from '../utils/externalProviderImport';

test('Qoder CN external import payload is not consumed by the global Qoder page', () => {
  queueExternalProviderImport({
    providerId: 'qoder_cn',
    page: 'qoder-cn',
    token: 'cn-fixture-token',
    autoImport: true,
    activate: false,
  });

  // 队列是单槽且平台不匹配时保留 payload：若 CN 页面按 'qoder' 消费就会串区，
  // 随后切到国际版页面时 CN 的 token 会被填进国际版表单。
  assert.equal(consumeQueuedExternalProviderImportForPlatform('qoder'), null);

  const consumed = consumeQueuedExternalProviderImportForPlatform('qoder_cn');
  assert.equal(consumed?.token, 'cn-fixture-token');
  assert.equal(consumeQueuedExternalProviderImportForPlatform('qoder_cn'), null);
});

test('global Qoder import payload is not consumed by the Qoder CN page', () => {
  queueExternalProviderImport({
    providerId: 'qoder',
    page: 'qoder',
    token: 'global-fixture-token',
    autoImport: true,
    activate: false,
  });

  assert.equal(consumeQueuedExternalProviderImportForPlatform('qoder_cn'), null);
  assert.equal(
    consumeQueuedExternalProviderImportForPlatform('qoder')?.token,
    'global-fixture-token',
  );
});

// QoderAccountsPage 同时承载 Global / CN 两个 region，任何写死 Global store 的读取
// 都会让 CN 主流程（OAuth 授权、外部导入）失效或串区。store 是 hook 返回值（state），
// 没有 getState()，异步回调必须用按 region 选出的 store API。
test('shared Qoder accounts page never reads the hardcoded global store api', () => {
  const source = readFileSync(new URL('./QoderAccountsPage.tsx', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /useQoderAccountStore\.getState\(\)/);
  assert.doesNotMatch(source, /consumeQueuedExternalProviderImportForPlatform\('qoder'\)/);
  assert.match(source, /consumeQueuedExternalProviderImportForPlatform\(platformId\)/);
});
