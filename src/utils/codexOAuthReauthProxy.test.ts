import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import {
  oauthProxyUseAfterStart,
  oauthStartProxyArgs,
  shouldDefaultReauthProxy,
} from './codexOAuthReauthProxy';

function oauthAccount(overrides: Partial<CodexAccount> = {}): CodexAccount {
  return {
    id: 'account-1',
    email: 'user@example.com',
    created_at: 0,
    last_used: 0,
    tokens: { access_token: 'access', id_token: 'id' },
    ...overrides,
  };
}

test('重新授权默认沿用账号生效出口，首次添加与 macOS 保持原行为', () => {
  const account = oauthAccount();
  assert.equal(shouldDefaultReauthProxy({ active: true, isMacOS: false, account }), true);
  // 首次添加没有账号记录，不推断任何账号代理。
  assert.equal(shouldDefaultReauthProxy({ active: true, isMacOS: false, account: null }), false);
  // macOS 当前构建不支持内置授权窗口代理。
  assert.equal(shouldDefaultReauthProxy({ active: true, isMacOS: true, account }), false);
  // 弹框未打开在 OAuth 页签时不改默认值。
  assert.equal(shouldDefaultReauthProxy({ active: false, isMacOS: false, account }), false);
  // 非普通 OAuth 账号没有账号代理。
  for (const overrides of [
    { auth_mode: 'apikey' },
    { token_source_mode: 'chatgpt_web_session' },
    {
      agent_identity: {
        agent_runtime_id: 'agent-runtime',
        agent_private_key: 'key',
        account_id: 'account-1',
        chatgpt_user_id: 'user-1',
      },
    },
    { upstream_grok_account_id: 'grok-1' },
    { api_provider_mode: 'custom' as const },
    { api_provider_id: 'provider-1' },
  ] satisfies Partial<CodexAccount>[]) {
    assert.equal(
      shouldDefaultReauthProxy({ active: true, isMacOS: false, account: oauthAccount(overrides) }),
      false,
    );
  }
});

test('只有开启代理时才发送出口，留空由后端按账号解析', () => {
  // 关闭开关：不发送任何出口。
  assert.deepEqual(
    oauthStartProxyArgs({ enabled: false, input: 'socks5://127.0.0.1:1080', usesAccountExit: true, reauthAccountId: 'account-1' }),
    { proxyUrl: null, reauthAccountId: null },
  );
  // 账号出口模式且输入框留空：只发送账号 ID。
  assert.deepEqual(
    oauthStartProxyArgs({ enabled: true, input: '  ', usesAccountExit: true, reauthAccountId: 'account-1' }),
    { proxyUrl: null, reauthAccountId: 'account-1' },
  );
  // 用户写了别的地址：按地址授权，不再按账号解析。
  assert.deepEqual(
    oauthStartProxyArgs({ enabled: true, input: ' http://127.0.0.1:8080 ', usesAccountExit: true, reauthAccountId: 'account-1' }),
    { proxyUrl: 'http://127.0.0.1:8080', reauthAccountId: null },
  );
  // 首次添加：只有用户填写的地址。
  assert.deepEqual(
    oauthStartProxyArgs({ enabled: true, input: 'http://127.0.0.1:8080', usesAccountExit: false, reauthAccountId: '' }),
    { proxyUrl: 'http://127.0.0.1:8080', reauthAccountId: null },
  );
  // 账号出口模式但输入框留空且没有账号 ID：不发送出口。
  assert.deepEqual(
    oauthStartProxyArgs({ enabled: true, input: '', usesAccountExit: true, reauthAccountId: '' }),
    { proxyUrl: null, reauthAccountId: null },
  );
});

test('后端确认出口后显示并回填地址，用户输入不被覆盖', () => {
  const accountExit = oauthProxyUseAfterStart({
    usesAccountExit: true,
    input: '',
    proxy: {
      source: 'account',
      summary: { protocol: 'HTTP', server: 'proxy.example', port: 8080 },
      input: 'http://user:secret@proxy.example:8080',
    },
  });
  assert.deepEqual(accountExit, {
    enabled: true,
    usesAccountExit: true,
    label: 'HTTP · proxy.example:8080',
    input: 'http://user:secret@proxy.example:8080',
  });

  const resourceExit = oauthProxyUseAfterStart({
    usesAccountExit: true,
    input: 'http://127.0.0.1:8080',
    proxy: {
      source: 'account',
      summary: { protocol: 'RESOURCE', name: 'Alpha', sourceName: 'Demo' },
    },
  });
  assert.equal(resourceExit?.label, 'Demo · Alpha');
  // 用户已填写地址时只补充出口说明，不回填资源快照。
  assert.equal(resourceExit?.input, null);
});

test('账号没有生效出口时回到默认授权路径，手填地址模式不受影响', () => {
  assert.deepEqual(
    oauthProxyUseAfterStart({ usesAccountExit: true, input: '', proxy: null }),
    { enabled: false, usesAccountExit: false, label: null, input: null },
  );
  // 用户自己填写的地址：后端返回的出口不覆盖输入框与开关状态。
  assert.equal(
    oauthProxyUseAfterStart({
      usesAccountExit: false,
      input: 'http://127.0.0.1:8080',
      proxy: { source: 'explicit', summary: { protocol: 'HTTP', server: '127.0.0.1', port: 8080 } },
    }),
    null,
  );
});
