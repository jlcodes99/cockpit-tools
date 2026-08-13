'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const EXPORTER_VERSION = '1.1.0';
const EXPORT_SCHEMA_VERSION = 1;
const DEFAULT_STALE_AFTER_MINUTES = 15;
const DATASET_NAMES = Object.freeze(['accounts', 'quota', 'gateway']);
const SECURE_ACCOUNT_KEY_FILE = 'secure-account-storage.key';
const ACCOUNTS_INDEX_FILE = 'codex_accounts.json';
const ACCOUNTS_DIRECTORY = 'codex_accounts';
const LOCAL_ACCESS_FILE = 'codex_local_access.json';
const LOCAL_ACCESS_STATS_FILE = 'codex_local_access_stats.json';

const TEAM_PLAN_MARKERS = new Set([
  'team',
  'business',
  'enterprise',
  'edu',
  'education',
]);

const USAGE_FIELDS = [
  'requestCount',
  'successCount',
  'failureCount',
  'clientCanceledCount',
  'streamIncompleteCount',
  'upstreamResponseFailedCount',
  'imageRequestCount',
  'imageGenerationRequestCount',
  'imageEditRequestCount',
  'imageGenerationCapabilityFailureCount',
  'textRequestCount',
  'inputTokens',
  'outputTokens',
  'reasoningTokens',
  'cachedTokens',
  'totalTokens',
  'totalLatencyMs',
  'estimatedCostUsd',
];

const EXCLUDED_SENSITIVE_FIELDS = [
  'tokens.id_token',
  'tokens.access_token',
  'tokens.refresh_token',
  'openai_api_key',
  'agent_identity.agent_private_key',
  'agent_identity.task_id',
  'account_password',
  'two_factor_secret',
  'mail_url',
  'phone_number',
  'quota.raw_data',
  'quota_error.message',
];

const FORBIDDEN_OUTPUT_KEYS = new Set([
  'tokens',
  'id_token',
  'idtoken',
  'access_token',
  'accesstoken',
  'refresh_token',
  'refreshtoken',
  'openai_api_key',
  'openaiapikey',
  'agent_private_key',
  'agentprivatekey',
  'task_id',
  'taskid',
  'account_password',
  'accountpassword',
  'two_factor_secret',
  'twofactorsecret',
  'mail_url',
  'mailurl',
  'phone_number',
  'phonenumber',
  'raw_data',
  'rawdata',
]);

const ACCOUNT_CSV_COLUMNS = [
  'profile',
  'in_api_pool',
  'id',
  'email',
  'user_id',
  'chatgpt_account_id',
  'organization_id',
  'account_name',
  'account_structure',
  'plan_type',
  'plan_family',
  'subscription_active_until',
  'auth_mode',
  'authorization_status',
  'requires_reauth',
  'reauth_reason',
  'token_generation',
  'token_updated_at',
  'token_updated_at_utc',
  'token_source_mode',
  'tags',
  'created_at',
  'created_at_utc',
  'last_used_at',
  'last_used_at_utc',
  'usage_updated_at',
  'usage_updated_at_utc',
  'usage_updated_at_local',
  'quota_stale',
  'quota_error_code',
  'quota_error_at',
  'quota_error_at_utc',
  'reset_credits_available',
  'reset_credits_next_expires_at',
  'reset_credits_next_expires_at_utc',
];

const QUOTA_WINDOW_CSV_COLUMNS = [
  'profile',
  'account_id',
  'email',
  'plan_type',
  'plan_family',
  'source',
  'limit_name',
  'metered_feature',
  'slot',
  'classification',
  'present',
  'remaining_percent',
  'used_percent',
  'window_minutes',
  'reset_at',
  'reset_at_utc',
  'reset_at_local',
  'snapshot_updated_at',
  'snapshot_updated_at_utc',
  'stale',
];

const GATEWAY_USAGE_CSV_COLUMNS = [
  'profile',
  'account_id',
  'email',
  'plan_type',
  'plan_family',
  'period',
  'period_since',
  'period_since_utc',
  'period_updated_at',
  'period_updated_at_utc',
  'account_updated_at',
  'account_updated_at_utc',
  ...USAGE_FIELDS.map((field) => camelToSnake(field)),
];

function camelToSnake(value) {
  return String(value).replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
}

function hasOwn(value, key) {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function nonEmptyString(value) {
  if (value === null || value === undefined) {
    return null;
  }
  const normalized = String(value).trim();
  return normalized || null;
}

function normalizeDatasets(value) {
  const rawValues =
    value === null || value === undefined
      ? DATASET_NAMES
      : Array.isArray(value)
        ? value
        : String(value).split(',');
  const datasets = [...new Set(rawValues.map((item) => String(item).trim().toLowerCase()).filter(Boolean))];
  if (datasets.length === 0) {
    throw new Error('At least one dataset must be selected');
  }
  const unsupported = datasets.filter((dataset) => !DATASET_NAMES.includes(dataset));
  if (unsupported.length > 0) {
    throw new Error(`Unsupported dataset: ${unsupported.join(', ')}`);
  }
  return datasets;
}

function readTextFile(filePath) {
  return fs.readFileSync(filePath, 'utf8').replace(/^\uFEFF/, '');
}

function readJsonFile(filePath, { optional = false, fallback = null } = {}) {
  if (!fs.existsSync(filePath)) {
    if (optional) {
      return fallback;
    }
    throw new Error(`Required file does not exist: ${filePath}`);
  }

  const text = readTextFile(filePath);
  if (!text.trim()) {
    if (optional) {
      return fallback;
    }
    throw new Error(`Required JSON file is empty: ${filePath}`);
  }

  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`Invalid JSON in ${filePath}: ${error.message}`);
  }
}

function decodeStrictBase64(value, label) {
  const normalized = nonEmptyString(value);
  if (!normalized) {
    throw new Error(`${label} is empty`);
  }
  const decoded = Buffer.from(normalized, 'base64');
  if (decoded.length === 0 || decoded.toString('base64').replace(/=+$/u, '') !== normalized.replace(/=+$/u, '')) {
    throw new Error(`${label} is not valid base64`);
  }
  return decoded;
}

function loadSecureAccountKey(dataDirectory) {
  const keyPath = path.join(dataDirectory, SECURE_ACCOUNT_KEY_FILE);
  const key = decodeStrictBase64(readTextFile(keyPath), SECURE_ACCOUNT_KEY_FILE);
  if (key.length !== 32) {
    throw new Error(`${SECURE_ACCOUNT_KEY_FILE} must decode to 32 bytes; got ${key.length}`);
  }
  return key;
}

function decryptSecureAccountEnvelope(envelope, key, sourcePath) {
  if (Number(envelope.version) !== 1) {
    throw new Error(`Unsupported secure account envelope version in ${sourcePath}`);
  }
  if (envelope.kind && String(envelope.kind).toLowerCase() !== 'codex') {
    throw new Error(`Unexpected secure account envelope kind in ${sourcePath}`);
  }
  if (envelope.algorithm && String(envelope.algorithm).toUpperCase() !== 'AES-256-GCM') {
    throw new Error(`Unsupported secure account algorithm in ${sourcePath}`);
  }

  const nonce = decodeStrictBase64(envelope.nonce, `${sourcePath} nonce`);
  if (nonce.length !== 12) {
    throw new Error(`AES-GCM nonce in ${sourcePath} must be 12 bytes; got ${nonce.length}`);
  }

  const combined = decodeStrictBase64(envelope.ciphertext, `${sourcePath} ciphertext`);
  if (combined.length <= 16) {
    throw new Error(`AES-GCM ciphertext in ${sourcePath} is too short`);
  }
  const ciphertext = combined.subarray(0, combined.length - 16);
  const authTag = combined.subarray(combined.length - 16);

  try {
    const decipher = crypto.createDecipheriv('aes-256-gcm', key, nonce);
    decipher.setAuthTag(authTag);
    return Buffer.concat([decipher.update(ciphertext), decipher.final()]).toString('utf8');
  } catch (error) {
    throw new Error(`Unable to decrypt ${sourcePath}: ${error.message}`);
  }
}

function parseAccountDetail(detailPath, keyProvider) {
  const parsed = readJsonFile(detailPath);
  const isEnvelope =
    parsed &&
    typeof parsed === 'object' &&
    nonEmptyString(parsed.nonce) &&
    nonEmptyString(parsed.ciphertext);

  if (!isEnvelope) {
    return parsed;
  }

  const plaintext = decryptSecureAccountEnvelope(parsed, keyProvider(), detailPath);
  try {
    return JSON.parse(plaintext);
  } catch (error) {
    throw new Error(`Decrypted account JSON is invalid in ${detailPath}: ${error.message}`);
  }
}

function loadCodexAccounts(dataDirectory, { skipInvalid = false } = {}) {
  const indexPath = path.join(dataDirectory, ACCOUNTS_INDEX_FILE);
  const detailDirectory = path.join(dataDirectory, ACCOUNTS_DIRECTORY);
  const index = readJsonFile(indexPath);
  if (!Array.isArray(index.accounts)) {
    throw new Error(`${indexPath} does not contain an accounts array`);
  }

  let secureKey;
  const keyProvider = () => {
    secureKey ||= loadSecureAccountKey(dataDirectory);
    return secureKey;
  };

  const accounts = [];
  const skipped = [];
  const seenIds = new Set();
  for (const summary of index.accounts) {
    const summaryId = nonEmptyString(summary && summary.id);
    if (!summaryId) {
      const message = 'Account index contains an entry without an id';
      if (!skipInvalid) {
        throw new Error(message);
      }
      skipped.push({ accountId: null, reason: message });
      continue;
    }
    if (seenIds.has(summaryId)) {
      throw new Error(`Duplicate account id in index: ${summaryId}`);
    }
    seenIds.add(summaryId);

    const detailPath = path.join(detailDirectory, `${summaryId}.json`);
    try {
      const account = parseAccountDetail(detailPath, keyProvider);
      if (!account || typeof account !== 'object' || Array.isArray(account)) {
        throw new Error('Account detail is not a JSON object');
      }
      const detailId = nonEmptyString(account.id);
      if (detailId && detailId !== summaryId) {
        throw new Error(`Account detail id ${detailId} does not match index id ${summaryId}`);
      }
      if (!detailId) {
        account.id = summaryId;
      }
      if (!nonEmptyString(account.email) && nonEmptyString(summary.email)) {
        account.email = String(summary.email);
      }
      if (!nonEmptyString(account.plan_type) && nonEmptyString(summary.plan_type)) {
        account.plan_type = String(summary.plan_type);
      }
      accounts.push(account);
    } catch (error) {
      if (!skipInvalid) {
        throw new Error(`Unable to load account ${summaryId}: ${error.message}`);
      }
      skipped.push({ accountId: summaryId, reason: error.message });
    }
  }

  return { accounts, skipped };
}

function planFamily(planType) {
  const normalized = String(planType || '').trim().toLowerCase();
  if (!normalized) {
    return 'unknown';
  }
  const words = normalized.replaceAll('_', ' ').replaceAll('-', ' ').split(/\s+/u);
  if (TEAM_PLAN_MARKERS.has(normalized) || words.some((word) => TEAM_PLAN_MARKERS.has(word))) {
    return 'team';
  }
  if (normalized === 'pro' || normalized === 'plus' || normalized === 'free') {
    return normalized;
  }
  return normalized;
}

function matchesPlanFamily(account, requestedFamily) {
  const requested = String(requestedFamily || 'team').trim().toLowerCase();
  return requested === 'all' || planFamily(account.plan_type) === requested;
}

function unixSeconds(value) {
  if (value === null || value === undefined || value === '') {
    return null;
  }
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) {
    return null;
  }
  const integer = Math.trunc(numeric);
  return Math.abs(integer) > 100_000_000_000 ? Math.trunc(integer / 1000) : integer;
}

function utcIso(value) {
  const seconds = unixSeconds(value);
  if (seconds === null) {
    return null;
  }
  const date = new Date(seconds * 1000);
  return Number.isNaN(date.getTime()) ? null : date.toISOString().replace('.000Z', 'Z');
}

function localIso(value) {
  const seconds = unixSeconds(value);
  if (seconds === null) {
    return null;
  }
  const date = new Date(seconds * 1000);
  if (Number.isNaN(date.getTime())) {
    return null;
  }
  const offsetMinutes = -date.getTimezoneOffset();
  const sign = offsetMinutes >= 0 ? '+' : '-';
  const absoluteOffset = Math.abs(offsetMinutes);
  const hours = String(Math.trunc(absoluteOffset / 60)).padStart(2, '0');
  const minutes = String(absoluteOffset % 60).padStart(2, '0');
  const localDate = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return `${localDate.toISOString().slice(0, 19)}${sign}${hours}:${minutes}`;
}

function normalizePercent(value) {
  const numeric = Number(value);
  if (!Number.isFinite(numeric)) {
    return null;
  }
  return Math.max(0, Math.min(100, Math.round(numeric)));
}

function normalizeWindowMinutes(value) {
  const numeric = Number(value);
  if (!Number.isFinite(numeric) || numeric <= 0) {
    return null;
  }
  return Math.trunc(numeric);
}

function classifyWindow(windowMinutes) {
  const minutes = normalizeWindowMinutes(windowMinutes);
  if (minutes === null) {
    return null;
  }
  const day = 24 * 60;
  const week = 7 * day;
  if (minutes >= week - 1) {
    const weeks = Math.max(1, Math.round(minutes / week));
    return weeks === 1 ? 'weekly' : `${weeks}week`;
  }
  if (minutes >= day - 1) {
    return `${Math.max(1, Math.round(minutes / day))}d`;
  }
  if (minutes >= 240 && minutes <= 360) {
    return 'five_hour';
  }
  if (minutes >= 60) {
    return `${Math.max(1, Math.round(minutes / 60))}h`;
  }
  return `${minutes}m`;
}

function normalizeStoredQuotaWindow(quota, slot, prefix) {
  const remainingKey = `${prefix}_percentage`;
  const minutesKey = `${prefix}_window_minutes`;
  const presentKey = `${prefix}_window_present`;
  const resetKey = `${prefix}_reset_time`;
  let present = quota[presentKey];
  if (present === null || present === undefined) {
    present = [remainingKey, minutesKey, resetKey].some((key) => hasOwn(quota, key) && quota[key] !== null);
  }
  present = Boolean(present);
  const remainingPercent = present ? normalizePercent(quota[remainingKey]) : null;
  const windowMinutes = present ? normalizeWindowMinutes(quota[minutesKey]) : null;
  const resetAt = present ? unixSeconds(quota[resetKey]) : null;
  return {
    source: 'rate_limit',
    limitName: null,
    meteredFeature: null,
    slot,
    classification: classifyWindow(windowMinutes),
    present,
    remainingPercent,
    usedPercent: remainingPercent === null ? null : 100 - remainingPercent,
    windowMinutes,
    resetAt,
    resetAtUtc: utcIso(resetAt),
    resetAtLocal: localIso(resetAt),
  };
}

function normalizeRawQuotaWindow(rawWindow, { source, slot, limitName, meteredFeature, snapshotUpdatedAt }) {
  if (!rawWindow || typeof rawWindow !== 'object' || Array.isArray(rawWindow)) {
    return null;
  }
  const usedPercent = normalizePercent(rawWindow.used_percent);
  const windowSeconds = Number(rawWindow.limit_window_seconds);
  const windowMinutes =
    Number.isFinite(windowSeconds) && windowSeconds > 0 ? Math.ceil(windowSeconds / 60) : null;
  let resetAt = unixSeconds(rawWindow.reset_at);
  const resetAfterSeconds = Number(rawWindow.reset_after_seconds);
  if (
    resetAt === null &&
    Number.isFinite(resetAfterSeconds) &&
    resetAfterSeconds >= 0 &&
    snapshotUpdatedAt !== null
  ) {
    resetAt = snapshotUpdatedAt + Math.trunc(resetAfterSeconds);
  }
  return {
    source,
    limitName: nonEmptyString(limitName),
    meteredFeature: nonEmptyString(meteredFeature),
    slot,
    classification: classifyWindow(windowMinutes),
    present: true,
    remainingPercent: usedPercent === null ? null : 100 - usedPercent,
    usedPercent,
    windowMinutes,
    resetAt,
    resetAtUtc: utcIso(resetAt),
    resetAtLocal: localIso(resetAt),
  };
}

function normalizeRawRateLimit(rateLimit, metadata) {
  if (!rateLimit || typeof rateLimit !== 'object' || Array.isArray(rateLimit)) {
    return [];
  }
  return [
    normalizeRawQuotaWindow(rateLimit.primary_window, { ...metadata, slot: 'primary' }),
    normalizeRawQuotaWindow(rateLimit.secondary_window, { ...metadata, slot: 'secondary' }),
  ].filter(Boolean);
}

function normalizeAdditionalLimits(rawData, snapshotUpdatedAt) {
  const rawLimits = Array.isArray(rawData && rawData.additional_rate_limits)
    ? rawData.additional_rate_limits
    : [];
  return rawLimits.map((limit, index) => {
    const limitName = nonEmptyString(limit && (limit.limit_name || limit.limitName)) || `additional-${index + 1}`;
    const meteredFeature = nonEmptyString(limit && (limit.metered_feature || limit.meteredFeature));
    return {
      limitName,
      meteredFeature,
      windows: normalizeRawRateLimit(limit && limit.rate_limit, {
        source: 'additional_rate_limit',
        limitName,
        meteredFeature,
        snapshotUpdatedAt,
      }),
    };
  });
}

function normalizeResetCredits(quota) {
  const credits = Array.isArray(quota.reset_credits) ? quota.reset_credits : [];
  return credits.map((credit) => ({
    id: nonEmptyString(credit && credit.id),
    status: nonEmptyString(credit && credit.status),
    resetType: nonEmptyString(credit && credit.reset_type),
    grantedAt: unixSeconds(credit && credit.granted_at),
    grantedAtUtc: utcIso(credit && credit.granted_at),
    expiresAt: unixSeconds(credit && credit.expires_at),
    expiresAtUtc: utcIso(credit && credit.expires_at),
    redeemedAt: unixSeconds(credit && credit.redeemed_at),
    redeemedAtUtc: utcIso(credit && credit.redeemed_at),
    statusSource: nonEmptyString(credit && credit.raw_status),
  }));
}

function safeQuotaError(account) {
  const error = account && account.quota_error;
  if (!error || typeof error !== 'object' || Array.isArray(error)) {
    return null;
  }
  const timestamp = unixSeconds(error.timestamp);
  return {
    code: nonEmptyString(error.code),
    timestamp,
    timestampUtc: utcIso(timestamp),
    timestampLocal: localIso(timestamp),
  };
}

function loadPoolAccountIds(dataDirectory) {
  const collection = readJsonFile(path.join(dataDirectory, LOCAL_ACCESS_FILE), {
    optional: true,
    fallback: {},
  });
  return new Set(
    Array.isArray(collection && collection.accountIds)
      ? collection.accountIds.map(nonEmptyString).filter(Boolean)
      : [],
  );
}

function emptyUsage() {
  return {
    updatedAt: null,
    ...Object.fromEntries(USAGE_FIELDS.map((field) => [field, 0])),
  };
}

function loadUsageMaps(dataDirectory) {
  const stats = readJsonFile(path.join(dataDirectory, LOCAL_ACCESS_STATS_FILE), {
    optional: true,
    fallback: {},
  });
  const maps = {};
  for (const period of ['daily', 'weekly', 'monthly']) {
    const window = stats && typeof stats[period] === 'object' ? stats[period] : {};
    const accountMap = new Map();
    const rows = Array.isArray(window.accounts) ? window.accounts : [];
    for (const row of rows) {
      const accountId = nonEmptyString(row && row.accountId);
      if (!accountId) {
        continue;
      }
      const usage = row && typeof row.usage === 'object' ? row.usage : {};
      accountMap.set(accountId, {
        updatedAt: row.updatedAt ?? null,
        ...Object.fromEntries(USAGE_FIELDS.map((field) => [field, Number(usage[field]) || 0])),
      });
    }
    maps[period] = {
      since: window.since ?? null,
      updatedAt: window.updatedAt ?? null,
      accounts: accountMap,
    };
  }
  return {
    available: fs.existsSync(path.join(dataDirectory, LOCAL_ACCESS_STATS_FILE)),
    periods: maps,
  };
}

function accountUsageFor(usageMaps, period, accountId) {
  const periodState = usageMaps.periods[period];
  return periodState && periodState.accounts.get(accountId)
    ? { ...periodState.accounts.get(accountId) }
    : emptyUsage();
}

function buildAccountRecord(account, context) {
  const accountId = nonEmptyString(account.id);
  const quota = account.quota && typeof account.quota === 'object' ? account.quota : {};
  const usageUpdatedAt = unixSeconds(account.usage_updated_at);
  const ageSeconds = usageUpdatedAt === null ? null : Math.trunc(context.nowMs / 1000) - usageUpdatedAt;
  const stale =
    usageUpdatedAt === null ||
    usageUpdatedAt <= 0 ||
    ageSeconds < 0 ||
    ageSeconds > context.staleAfterMinutes * 60;
  const rawData = quota.raw_data && typeof quota.raw_data === 'object' ? quota.raw_data : {};
  const primary = normalizeStoredQuotaWindow(quota, 'primary', 'hourly');
  const secondary = normalizeStoredQuotaWindow(quota, 'secondary', 'weekly');
  const codeReviewWindows = normalizeRawRateLimit(rawData.code_review_rate_limit, {
    source: 'code_review_rate_limit',
    limitName: 'code_review',
    meteredFeature: 'code_review',
    snapshotUpdatedAt: usageUpdatedAt,
  });
  const additionalLimits = normalizeAdditionalLimits(rawData, usageUpdatedAt);
  const localGatewayUsage = Object.fromEntries(
    ['daily', 'weekly', 'monthly'].map((period) => [
      period,
      accountUsageFor(context.usageMaps, period, accountId),
    ]),
  );

  return {
    profile: context.profile,
    inApiPool: context.poolAccountIds.has(accountId),
    id: accountId,
    email: nonEmptyString(account.email),
    userId: nonEmptyString(account.user_id),
    chatgptAccountId: nonEmptyString(account.account_id),
    organizationId: nonEmptyString(account.organization_id),
    accountName: nonEmptyString(account.account_name),
    accountStructure: nonEmptyString(account.account_structure),
    planType: nonEmptyString(account.plan_type),
    planFamily: planFamily(account.plan_type),
    subscriptionActiveUntil: account.subscription_active_until ?? null,
    authMode: nonEmptyString(account.auth_mode),
    authorizationStatus: nonEmptyString(account.authorization_status),
    requiresReauth: Boolean(account.requires_reauth),
    reauthReason: nonEmptyString(account.reauth_reason),
    tokenGeneration: Number.isFinite(Number(account.token_generation))
      ? Math.trunc(Number(account.token_generation))
      : null,
    tokenUpdatedAt: unixSeconds(account.token_updated_at),
    tokenUpdatedAtUtc: utcIso(account.token_updated_at),
    tokenSourceMode: nonEmptyString(account.token_source_mode),
    tags: Array.isArray(account.tags) ? account.tags.map(nonEmptyString).filter(Boolean) : [],
    createdAt: unixSeconds(account.created_at),
    createdAtUtc: utcIso(account.created_at),
    lastUsedAt: unixSeconds(account.last_used),
    lastUsedAtUtc: utcIso(account.last_used),
    usageUpdatedAt,
    usageUpdatedAtUtc: utcIso(usageUpdatedAt),
    usageUpdatedAtLocal: localIso(usageUpdatedAt),
    stale,
    upstreamQuota: {
      windows: [primary, secondary],
      codeReviewWindows,
      additionalLimits,
      rateLimitReachedType: nonEmptyString(rawData.rate_limit_reached_type),
      resetCreditsAvailable: Number.isFinite(Number(quota.reset_credits_available))
        ? Math.trunc(Number(quota.reset_credits_available))
        : null,
      resetCredits: normalizeResetCredits(quota),
      resetCreditsNextExpiresAt: unixSeconds(quota.reset_credits_next_expires_at),
      resetCreditsNextExpiresAtUtc: utcIso(quota.reset_credits_next_expires_at),
      error: safeQuotaError(account),
    },
    localGatewayUsage,
  };
}

function projectRecordForDatasets(record, selectedDatasets) {
  const datasetSet = new Set(selectedDatasets);
  const { upstreamQuota, localGatewayUsage, ...identity } = record;
  const projected = datasetSet.has('accounts')
    ? { ...identity }
    : {
        profile: record.profile,
        inApiPool: record.inApiPool,
        id: record.id,
        email: record.email,
        planType: record.planType,
        planFamily: record.planFamily,
      };
  if (datasetSet.has('quota')) {
    projected.upstreamQuota = upstreamQuota;
  }
  if (datasetSet.has('gateway')) {
    projected.localGatewayUsage = localGatewayUsage;
  }
  return projected;
}

function assertNoForbiddenOutputKeys(value, currentPath = '$') {
  if (Array.isArray(value)) {
    value.forEach((item, index) => assertNoForbiddenOutputKeys(item, `${currentPath}[${index}]`));
    return;
  }
  if (!value || typeof value !== 'object') {
    return;
  }
  for (const [key, child] of Object.entries(value)) {
    const normalizedKey = key.replaceAll('-', '_').toLowerCase();
    if (FORBIDDEN_OUTPUT_KEYS.has(normalizedKey)) {
      throw new Error(`Sensitive field was about to be exported at ${currentPath}.${key}`);
    }
    assertNoForbiddenOutputKeys(child, `${currentPath}.${key}`);
  }
}

function spreadsheetSafeString(value) {
  const text = String(value);
  return /^[=+\-@\t\r\n]/u.test(text) ? `'${text}` : text;
}

function csvCell(value) {
  let text;
  if (value === null || value === undefined) {
    text = '';
  } else if (Array.isArray(value)) {
    text = value.join(';');
  } else if (typeof value === 'object') {
    text = JSON.stringify(value);
  } else {
    text = String(value);
  }
  text = spreadsheetSafeString(text);
  return `"${text.replaceAll('"', '""')}"`;
}

function renderCsv(columns, rows) {
  const lines = [columns.map(csvCell).join(',')];
  for (const row of rows) {
    lines.push(columns.map((column) => csvCell(row[column])).join(','));
  }
  return `\uFEFF${lines.join('\r\n')}\r\n`;
}

function accountCsvRows(records) {
  return records.map((record) => {
    const error = record.upstreamQuota.error || {};
    return {
      profile: record.profile,
      in_api_pool: record.inApiPool,
      id: record.id,
      email: record.email,
      user_id: record.userId,
      chatgpt_account_id: record.chatgptAccountId,
      organization_id: record.organizationId,
      account_name: record.accountName,
      account_structure: record.accountStructure,
      plan_type: record.planType,
      plan_family: record.planFamily,
      subscription_active_until: record.subscriptionActiveUntil,
      auth_mode: record.authMode,
      authorization_status: record.authorizationStatus,
      requires_reauth: record.requiresReauth,
      reauth_reason: record.reauthReason,
      token_generation: record.tokenGeneration,
      token_updated_at: record.tokenUpdatedAt,
      token_updated_at_utc: record.tokenUpdatedAtUtc,
      token_source_mode: record.tokenSourceMode,
      tags: record.tags,
      created_at: record.createdAt,
      created_at_utc: record.createdAtUtc,
      last_used_at: record.lastUsedAt,
      last_used_at_utc: record.lastUsedAtUtc,
      usage_updated_at: record.usageUpdatedAt,
      usage_updated_at_utc: record.usageUpdatedAtUtc,
      usage_updated_at_local: record.usageUpdatedAtLocal,
      quota_stale: record.stale,
      quota_error_code: error.code,
      quota_error_at: error.timestamp,
      quota_error_at_utc: error.timestampUtc,
      reset_credits_available: record.upstreamQuota.resetCreditsAvailable,
      reset_credits_next_expires_at: record.upstreamQuota.resetCreditsNextExpiresAt,
      reset_credits_next_expires_at_utc: record.upstreamQuota.resetCreditsNextExpiresAtUtc,
    };
  });
}

function allQuotaWindows(record) {
  const rows = [...record.upstreamQuota.windows, ...record.upstreamQuota.codeReviewWindows];
  for (const limit of record.upstreamQuota.additionalLimits) {
    rows.push(...limit.windows);
  }
  return rows;
}

function quotaWindowCsvRows(records) {
  const rows = [];
  for (const record of records) {
    for (const window of allQuotaWindows(record)) {
      rows.push({
        profile: record.profile,
        account_id: record.id,
        email: record.email,
        plan_type: record.planType,
        plan_family: record.planFamily,
        source: window.source,
        limit_name: window.limitName,
        metered_feature: window.meteredFeature,
        slot: window.slot,
        classification: window.classification,
        present: window.present,
        remaining_percent: window.remainingPercent,
        used_percent: window.usedPercent,
        window_minutes: window.windowMinutes,
        reset_at: window.resetAt,
        reset_at_utc: window.resetAtUtc,
        reset_at_local: window.resetAtLocal,
        snapshot_updated_at: record.usageUpdatedAt,
        snapshot_updated_at_utc: record.usageUpdatedAtUtc,
        stale: record.stale,
      });
    }
  }
  return rows;
}

function gatewayUsageCsvRows(records, usageMaps) {
  const rows = [];
  for (const record of records) {
    for (const period of ['daily', 'weekly', 'monthly']) {
      const periodState = usageMaps.periods[period];
      const usage = record.localGatewayUsage[period];
      const row = {
        profile: record.profile,
        account_id: record.id,
        email: record.email,
        plan_type: record.planType,
        plan_family: record.planFamily,
        period,
        period_since: periodState.since,
        period_since_utc: utcIso(periodState.since),
        period_updated_at: periodState.updatedAt,
        period_updated_at_utc: utcIso(periodState.updatedAt),
        account_updated_at: usage.updatedAt,
        account_updated_at_utc: utcIso(usage.updatedAt),
      };
      for (const field of USAGE_FIELDS) {
        row[camelToSnake(field)] = usage[field];
      }
      rows.push(row);
    }
  }
  return rows;
}

function ensureOutputDirectory(outputDirectory) {
  if (fs.existsSync(outputDirectory)) {
    const stat = fs.statSync(outputDirectory);
    if (!stat.isDirectory()) {
      throw new Error(`Output path is not a directory: ${outputDirectory}`);
    }
    const existing = fs.readdirSync(outputDirectory);
    if (existing.length > 0) {
      throw new Error(`Output directory must be empty: ${outputDirectory}`);
    }
  } else {
    fs.mkdirSync(outputDirectory, { recursive: true, mode: 0o700 });
  }
  try {
    fs.chmodSync(outputDirectory, 0o700);
  } catch {
    // Windows ACLs are hardened by the PowerShell launcher.
  }
}

function writeFileAtomic(filePath, content) {
  const tempPath = `${filePath}.tmp-${process.pid}-${crypto.randomBytes(6).toString('hex')}`;
  let descriptor;
  try {
    descriptor = fs.openSync(tempPath, 'wx', 0o600);
    fs.writeFileSync(descriptor, content, 'utf8');
    fs.fsyncSync(descriptor);
    fs.closeSync(descriptor);
    descriptor = undefined;
    fs.renameSync(tempPath, filePath);
    try {
      fs.chmodSync(filePath, 0o600);
    } catch {
      // Windows ACLs are hardened by the PowerShell launcher.
    }
  } catch (error) {
    if (descriptor !== undefined) {
      fs.closeSync(descriptor);
    }
    try {
      fs.rmSync(tempPath, { force: true });
    } catch {
      // Keep the original error.
    }
    throw error;
  }
}

function sha256File(filePath) {
  const content = fs.readFileSync(filePath);
  return crypto.createHash('sha256').update(content).digest('hex');
}

function outputFileMetadata(outputDirectory, fileName) {
  const filePath = path.join(outputDirectory, fileName);
  const stat = fs.statSync(filePath);
  return {
    name: fileName,
    bytes: stat.size,
    sha256: sha256File(filePath),
  };
}

function countRecordStates(records) {
  const quotaErrors = records.filter((record) => record.upstreamQuota.error).length;
  const requiresReauth = records.filter((record) => record.requiresReauth).length;
  const stale = records.filter((record) => record.stale).length;
  const exhausted = records.filter((record) =>
    record.upstreamQuota.windows.some(
      (window) => window.present && window.remainingPercent === 0,
    ),
  ).length;
  return { quotaErrors, requiresReauth, stale, exhausted };
}

function exportCockpitAccounts(options) {
  const dataDirectory = path.resolve(String(options.dataDirectory || ''));
  if (!options.dataDirectory) {
    throw new Error('dataDirectory is required');
  }
  const profile = String(options.profile || 'production').trim().toLowerCase();
  const requestedPlanFamily = String(options.planFamily || 'team').trim().toLowerCase();
  const format = String(options.format || 'both').trim().toLowerCase();
  const selectedDatasets = normalizeDatasets(options.datasets);
  if (!['both', 'json', 'csv'].includes(format)) {
    throw new Error(`Unsupported format: ${format}`);
  }
  const staleAfterMinutes = Number(options.staleAfterMinutes ?? DEFAULT_STALE_AFTER_MINUTES);
  if (!Number.isInteger(staleAfterMinutes) || staleAfterMinutes < 1 || staleAfterMinutes > 10080) {
    throw new Error('staleAfterMinutes must be an integer between 1 and 10080');
  }
  const nowMs = Number(options.nowMs ?? Date.now());
  if (!Number.isFinite(nowMs) || nowMs <= 0) {
    throw new Error('nowMs must be a positive timestamp');
  }

  const { accounts, skipped } = loadCodexAccounts(dataDirectory, {
    skipInvalid: Boolean(options.skipInvalid),
  });
  const poolAccountIds = loadPoolAccountIds(dataDirectory);
  const usageMaps = loadUsageMaps(dataDirectory);
  const records = accounts
    .filter((account) => matchesPlanFamily(account, requestedPlanFamily))
    .map((account) =>
      buildAccountRecord(account, {
        profile,
        poolAccountIds,
        usageMaps,
        staleAfterMinutes,
        nowMs,
      }),
    )
    .sort((left, right) =>
      String(left.email || '').localeCompare(String(right.email || ''), 'en', {
        sensitivity: 'base',
      }) || String(left.id || '').localeCompare(String(right.id || '')),
    );

  assertNoForbiddenOutputKeys(records);
  const generatedAt = new Date(nowMs).toISOString();
  const stateCounts = countRecordStates(records);
  const summary = {
    schemaVersion: EXPORT_SCHEMA_VERSION,
    exporterVersion: EXPORTER_VERSION,
    generatedAt,
    profile,
    planFamilyFilter: requestedPlanFamily,
    selectedDatasets,
    accountCount: records.length,
    apiPoolAccountCount: records.filter((record) => record.inApiPool).length,
    skippedAccountCount: skipped.length,
    skippedAccounts: skipped,
    quotaErrorCount: stateCounts.quotaErrors,
    requiresReauthCount: stateCounts.requiresReauth,
    staleCount: stateCounts.stale,
    exhaustedAccountCount: stateCounts.exhausted,
    localGatewayUsageAvailable: usageMaps.available,
    csvSpreadsheetFormulaProtection: format === 'both' || format === 'csv',
    sensitiveFieldsExcluded: EXCLUDED_SENSITIVE_FIELDS,
    outputFiles: [],
  };

  if (options.validateOnly) {
    return {
      outputDirectory: null,
      records,
      summary,
    };
  }

  if (!options.outputDirectory) {
    throw new Error('outputDirectory is required unless validateOnly is true');
  }
  const outputDirectory = path.resolve(String(options.outputDirectory));
  ensureOutputDirectory(outputDirectory);
  const fileNames = [];

  if (format === 'both' || format === 'json') {
    const jsonFile = 'cockpit-account-export.json';
    const payload = {
      schemaVersion: EXPORT_SCHEMA_VERSION,
      exporterVersion: EXPORTER_VERSION,
      generatedAt,
      profile,
      planFamilyFilter: requestedPlanFamily,
      selectedDatasets,
      accountCount: records.length,
      apiPoolAccountCount: summary.apiPoolAccountCount,
      staleAfterMinutes,
      localGatewayUsageAvailable: usageMaps.available,
      sensitiveFieldsExcluded: EXCLUDED_SENSITIVE_FIELDS,
      accounts: records.map((record) => projectRecordForDatasets(record, selectedDatasets)),
    };
    assertNoForbiddenOutputKeys(payload);
    writeFileAtomic(path.join(outputDirectory, jsonFile), `${JSON.stringify(payload, null, 2)}\n`);
    fileNames.push(jsonFile);
  }

  if (format === 'both' || format === 'csv') {
    const csvFiles = [];
    if (selectedDatasets.includes('accounts')) {
      csvFiles.push(['accounts.csv', ACCOUNT_CSV_COLUMNS, accountCsvRows(records)]);
    }
    if (selectedDatasets.includes('quota')) {
      csvFiles.push(['quota-windows.csv', QUOTA_WINDOW_CSV_COLUMNS, quotaWindowCsvRows(records)]);
    }
    if (selectedDatasets.includes('gateway')) {
      csvFiles.push([
        'gateway-usage.csv',
        GATEWAY_USAGE_CSV_COLUMNS,
        gatewayUsageCsvRows(records, usageMaps),
      ]);
    }
    for (const [fileName, columns, rows] of csvFiles) {
      writeFileAtomic(path.join(outputDirectory, fileName), renderCsv(columns, rows));
      fileNames.push(fileName);
    }
  }

  summary.outputFiles = fileNames.map((fileName) => outputFileMetadata(outputDirectory, fileName));
  const summaryFile = 'export-summary.json';
  writeFileAtomic(path.join(outputDirectory, summaryFile), `${JSON.stringify(summary, null, 2)}\n`);

  return {
    outputDirectory,
    records,
    summary: {
      ...summary,
      summaryFile,
    },
  };
}

module.exports = {
  ACCOUNT_CSV_COLUMNS,
  DATASET_NAMES,
  DEFAULT_STALE_AFTER_MINUTES,
  EXCLUDED_SENSITIVE_FIELDS,
  EXPORTER_VERSION,
  GATEWAY_USAGE_CSV_COLUMNS,
  QUOTA_WINDOW_CSV_COLUMNS,
  USAGE_FIELDS,
  assertNoForbiddenOutputKeys,
  classifyWindow,
  decryptSecureAccountEnvelope,
  exportCockpitAccounts,
  loadCodexAccounts,
  matchesPlanFamily,
  normalizeDatasets,
  planFamily,
  projectRecordForDatasets,
  renderCsv,
  unixSeconds,
  utcIso,
};
