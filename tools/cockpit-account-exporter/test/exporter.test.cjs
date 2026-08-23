'use strict';

const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const {
  classifyWindow,
  exportCockpitAccounts,
  normalizeDatasets,
  planFamily,
} = require('../exporter.cjs');
const { parseArguments } = require('../cockpit-account-exporter.cjs');

function writeJson(filePath, value) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, `${JSON.stringify(value, null, 2)}\n`, 'utf8');
}

function encryptAccount(account, key) {
  const nonce = crypto.randomBytes(12);
  const cipher = crypto.createCipheriv('aes-256-gcm', key, nonce);
  const ciphertext = Buffer.concat([
    cipher.update(Buffer.from(JSON.stringify(account), 'utf8')),
    cipher.final(),
    cipher.getAuthTag(),
  ]);
  return {
    version: 1,
    kind: 'codex',
    algorithm: 'AES-256-GCM',
    key_id: 'test-key',
    nonce: nonce.toString('base64'),
    ciphertext: ciphertext.toString('base64'),
    encrypted_at: 1_786_375_000,
  };
}

function makeFixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-account-exporter-'));
  const dataDirectory = path.join(root, '.antigravity_cockpit');
  const detailsDirectory = path.join(dataDirectory, 'codex_accounts');
  const key = crypto.randomBytes(32);
  fs.mkdirSync(detailsDirectory, { recursive: true });
  fs.writeFileSync(
    path.join(dataDirectory, 'secure-account-storage.key'),
    `${key.toString('base64')}\n`,
    'utf8',
  );

  const nowSeconds = 1_786_376_000;
  const team = {
    id: 'team-account-1',
    email: 'owner@example.com',
    auth_mode: 'oauth',
    openai_api_key: 'sk-must-not-export',
    user_id: 'user-full-value',
    plan_type: 'team',
    subscription_active_until: '2026-09-01T00:00:00Z',
    account_id: 'chatgpt-account-full-value',
    organization_id: 'organization-full-value',
    account_name: '=HYPERLINK("https://example.invalid")',
    account_structure: 'workspace',
    account_password: 'password-must-not-export',
    two_factor_secret: 'totp-must-not-export',
    phone_number: '+1-555-must-not-export',
    mail_url: 'https://mail.invalid/secret-must-not-export',
    tokens: {
      id_token: 'id-token-must-not-export',
      access_token: 'access-token-must-not-export',
      refresh_token: 'refresh-token-must-not-export',
    },
    agent_identity: {
      agent_private_key: 'private-key-must-not-export',
      task_id: 'task-id-must-not-export',
    },
    token_generation: 7,
    token_updated_at: nowSeconds - 30,
    token_source_mode: 'managed',
    authorization_status: 'active',
    requires_reauth: false,
    quota: {
      hourly_percentage: 0,
      hourly_reset_time: nowSeconds + 3600,
      hourly_window_minutes: 10080,
      hourly_window_present: true,
      weekly_percentage: 100,
      weekly_reset_time: null,
      weekly_window_minutes: null,
      weekly_window_present: false,
      reset_credits_available: 2,
      reset_credits: [
        {
          id: 'credit-1',
          status: 'available',
          reset_type: 'weekly',
          granted_at: nowSeconds - 60,
          expires_at: nowSeconds + 7200,
        },
      ],
      reset_credits_next_expires_at: nowSeconds + 7200,
      raw_data: {
        email: 'raw-email-must-not-export@example.com',
        access_token: 'raw-access-token-must-not-export',
        rate_limit_reached_type: 'weekly',
        code_review_rate_limit: {
          primary_window: {
            used_percent: 20,
            limit_window_seconds: 300 * 60,
            reset_at: nowSeconds + 1800,
          },
        },
        additional_rate_limits: [
          {
            limit_name: 'GPT-Spark',
            metered_feature: 'codex_spark',
            rate_limit: {
              primary_window: {
                used_percent: 25,
                limit_window_seconds: 60 * 60,
                reset_after_seconds: 900,
              },
            },
          },
        ],
      },
    },
    quota_error: null,
    usage_updated_at: nowSeconds - 60,
    tags: ['=TAG()', 'production', 'team'],
    created_at: nowSeconds - 10_000,
    last_used: nowSeconds - 10,
  };
  const pro = {
    ...team,
    id: 'pro-account-1',
    email: 'pro@example.com',
    plan_type: 'pro',
    account_id: 'pro-chatgpt-account',
    organization_id: 'pro-organization',
    account_name: 'Pro workspace',
    tokens: {
      id_token: 'pro-id-token',
      access_token: 'pro-access-token',
      refresh_token: 'pro-refresh-token',
    },
  };

  writeJson(path.join(dataDirectory, 'codex_accounts.json'), {
    version: '1.0',
    detail_schema_version: 2,
    current_account_id: team.id,
    accounts: [
      { id: team.id, email: team.email, plan_type: team.plan_type, created_at: team.created_at, last_used: team.last_used },
      { id: pro.id, email: pro.email, plan_type: pro.plan_type, created_at: pro.created_at, last_used: pro.last_used },
    ],
  });
  writeJson(path.join(detailsDirectory, `${team.id}.json`), encryptAccount(team, key));
  writeJson(path.join(detailsDirectory, `${pro.id}.json`), pro);
  writeJson(path.join(dataDirectory, 'codex_local_access.json'), {
    enabled: true,
    accountIds: [team.id],
  });
  writeJson(path.join(dataDirectory, 'codex_local_access_stats.json'), {
    daily: {
      since: (nowSeconds - 3600) * 1000,
      updatedAt: nowSeconds * 1000,
      accounts: [
        {
          accountId: team.id,
          email: team.email,
          updatedAt: nowSeconds * 1000,
          usage: {
            requestCount: 10,
            successCount: 9,
            failureCount: 1,
            inputTokens: 1000,
            outputTokens: 100,
            reasoningTokens: 50,
            cachedTokens: 800,
            totalTokens: 1100,
            estimatedCostUsd: 1.25,
          },
        },
      ],
    },
    weekly: { since: (nowSeconds - 7200) * 1000, updatedAt: nowSeconds * 1000, accounts: [] },
    monthly: { since: (nowSeconds - 10800) * 1000, updatedAt: nowSeconds * 1000, accounts: [] },
  });

  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  return { root, dataDirectory, detailsDirectory, key, nowSeconds, team, pro };
}

test('normalizes Team-family plans and classifies actual window duration', () => {
  assert.equal(planFamily('team'), 'team');
  assert.equal(planFamily('ChatGPT Business'), 'team');
  assert.equal(planFamily('enterprise'), 'team');
  assert.equal(planFamily('pro'), 'pro');
  assert.equal(classifyWindow(300), 'five_hour');
  assert.equal(classifyWindow(10080), 'weekly');
  assert.equal(classifyWindow(2880), '2d');
});

test('rejects unsupported CLI filters and stale thresholds before reading account data', () => {
  assert.throws(
    () => parseArguments(['--plan-family', 'unknown']),
    /Unsupported plan family/u,
  );
  assert.throws(
    () => parseArguments(['--format', 'xml']),
    /Unsupported format/u,
  );
  assert.throws(
    () => parseArguments(['--stale-after-minutes', '1.5']),
    /must be an integer/u,
  );
  assert.deepEqual(
    parseArguments(['--datasets', 'quota,gateway', '--validate-only']).datasets,
    ['quota', 'gateway'],
  );
  assert.throws(() => normalizeDatasets([]), /At least one dataset/u);
  assert.throws(() => normalizeDatasets(['quota', 'secrets']), /Unsupported dataset/u);
});

test('exports full account identity, normalized quota windows, and gateway usage without credentials', (t) => {
  const fixture = makeFixture(t);
  const outputDirectory = path.join(fixture.root, 'output');
  const result = exportCockpitAccounts({
    dataDirectory: fixture.dataDirectory,
    outputDirectory,
    profile: 'production',
    planFamily: 'team',
    format: 'both',
    staleAfterMinutes: 15,
    nowMs: fixture.nowSeconds * 1000,
  });

  assert.equal(result.summary.accountCount, 1);
  assert.equal(result.summary.apiPoolAccountCount, 1);
  assert.equal(result.summary.exhaustedAccountCount, 1);
  assert.equal(result.records[0].email, fixture.team.email);
  assert.equal(result.records[0].chatgptAccountId, fixture.team.account_id);
  assert.equal(result.records[0].organizationId, fixture.team.organization_id);
  assert.equal(result.records[0].accountName, fixture.team.account_name);
  assert.equal(result.records[0].upstreamQuota.windows[0].classification, 'weekly');
  assert.equal(result.records[0].upstreamQuota.windows[0].remainingPercent, 0);
  assert.equal(result.records[0].upstreamQuota.additionalLimits[0].windows[0].remainingPercent, 75);
  assert.equal(result.records[0].localGatewayUsage.daily.requestCount, 10);

  const jsonText = fs.readFileSync(path.join(outputDirectory, 'cockpit-account-export.json'), 'utf8');
  const accountsCsv = fs.readFileSync(path.join(outputDirectory, 'accounts.csv'), 'utf8');
  const quotaCsv = fs.readFileSync(path.join(outputDirectory, 'quota-windows.csv'), 'utf8');
  const usageCsv = fs.readFileSync(path.join(outputDirectory, 'gateway-usage.csv'), 'utf8');
  assert.match(jsonText, /owner@example\.com/u);
  assert.match(jsonText, /organization-full-value/u);
  assert.match(accountsCsv, /owner@example\.com/u);
  assert.match(accountsCsv, /"'=HYPERLINK/u);
  assert.match(accountsCsv, /"'=TAG\(\);production;team"/u);
  assert.match(quotaCsv, /"weekly"/u);
  assert.match(quotaCsv, /"75"/u);
  assert.match(usageCsv, /"10"/u);

  for (const secret of [
    fixture.team.openai_api_key,
    fixture.team.account_password,
    fixture.team.two_factor_secret,
    fixture.team.mail_url,
    fixture.team.tokens.id_token,
    fixture.team.tokens.access_token,
    fixture.team.tokens.refresh_token,
    fixture.team.agent_identity.agent_private_key,
    fixture.team.agent_identity.task_id,
    fixture.team.quota.raw_data.email,
    fixture.team.quota.raw_data.access_token,
  ]) {
    assert.equal(jsonText.includes(secret), false, `JSON leaked ${secret}`);
    assert.equal(accountsCsv.includes(secret), false, `CSV leaked ${secret}`);
    assert.equal(quotaCsv.includes(secret), false, `quota CSV leaked ${secret}`);
    assert.equal(usageCsv.includes(secret), false, `usage CSV leaked ${secret}`);
  }

  assert.equal(result.summary.outputFiles.length, 4);
  for (const file of result.summary.outputFiles) {
    assert.match(file.sha256, /^[a-f0-9]{64}$/u);
    assert.ok(file.bytes > 0);
  }
});

test('supports validate-only and all-plan exports without writing files', (t) => {
  const fixture = makeFixture(t);
  const result = exportCockpitAccounts({
    dataDirectory: fixture.dataDirectory,
    profile: 'production',
    planFamily: 'all',
    format: 'both',
    validateOnly: true,
    nowMs: fixture.nowSeconds * 1000,
  });
  assert.equal(result.outputDirectory, null);
  assert.equal(result.summary.accountCount, 2);
  assert.equal(result.records.some((record) => record.planFamily === 'team'), true);
  assert.equal(result.records.some((record) => record.planFamily === 'pro'), true);
  assert.equal(fs.existsSync(path.join(fixture.root, 'output')), false);
});

test('writes only selected datasets and limits JSON fields when account details are not selected', (t) => {
  const fixture = makeFixture(t);
  const outputDirectory = path.join(fixture.root, 'quota-only-output');
  const result = exportCockpitAccounts({
    dataDirectory: fixture.dataDirectory,
    outputDirectory,
    profile: 'production',
    planFamily: 'team',
    datasets: ['quota'],
    format: 'both',
    nowMs: fixture.nowSeconds * 1000,
  });

  assert.deepEqual(result.summary.selectedDatasets, ['quota']);
  assert.equal(fs.existsSync(path.join(outputDirectory, 'cockpit-account-export.json')), true);
  assert.equal(fs.existsSync(path.join(outputDirectory, 'quota-windows.csv')), true);
  assert.equal(fs.existsSync(path.join(outputDirectory, 'accounts.csv')), false);
  assert.equal(fs.existsSync(path.join(outputDirectory, 'gateway-usage.csv')), false);

  const payload = JSON.parse(
    fs.readFileSync(path.join(outputDirectory, 'cockpit-account-export.json'), 'utf8'),
  );
  assert.deepEqual(payload.selectedDatasets, ['quota']);
  assert.equal(payload.accounts[0].id, fixture.team.id);
  assert.equal(payload.accounts[0].email, fixture.team.email);
  assert.ok(payload.accounts[0].upstreamQuota);
  assert.equal(Object.hasOwn(payload.accounts[0], 'localGatewayUsage'), false);
  assert.equal(Object.hasOwn(payload.accounts[0], 'authorizationStatus'), false);
});

test('fails closed on unreadable details, supports explicit skip, and refuses non-empty destinations', (t) => {
  const fixture = makeFixture(t);
  const detailPath = path.join(fixture.detailsDirectory, `${fixture.team.id}.json`);
  fs.writeFileSync(detailPath, '{broken', 'utf8');

  assert.throws(
    () =>
      exportCockpitAccounts({
        dataDirectory: fixture.dataDirectory,
        profile: 'production',
        planFamily: 'team',
        format: 'json',
        validateOnly: true,
        nowMs: fixture.nowSeconds * 1000,
      }),
    /Unable to load account team-account-1/u,
  );

  const skipped = exportCockpitAccounts({
    dataDirectory: fixture.dataDirectory,
    profile: 'production',
    planFamily: 'team',
    format: 'json',
    validateOnly: true,
    skipInvalid: true,
    nowMs: fixture.nowSeconds * 1000,
  });
  assert.equal(skipped.summary.accountCount, 0);
  assert.equal(skipped.summary.skippedAccountCount, 1);

  const occupied = path.join(fixture.root, 'occupied');
  fs.mkdirSync(occupied);
  fs.writeFileSync(path.join(occupied, 'existing.txt'), 'keep me', 'utf8');
  assert.throws(
    () =>
      exportCockpitAccounts({
        dataDirectory: fixture.dataDirectory,
        outputDirectory: occupied,
        profile: 'production',
        planFamily: 'pro',
        format: 'json',
        skipInvalid: true,
        nowMs: fixture.nowSeconds * 1000,
      }),
    /Output directory must be empty/u,
  );
  assert.equal(fs.readFileSync(path.join(occupied, 'existing.txt'), 'utf8'), 'keep me');
});
