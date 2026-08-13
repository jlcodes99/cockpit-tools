'use strict';

const childProcess = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');

const {
  DATASET_NAMES,
  DEFAULT_STALE_AFTER_MINUTES,
  EXCLUDED_SENSITIVE_FIELDS,
  EXPORTER_VERSION,
  exportCockpitAccounts,
  normalizeDatasets,
} = require('./exporter.cjs');

const MAX_REQUEST_BYTES = 32 * 1024;
const SESSION_COOKIE_NAME = 'cockpit_exporter_session';
const PLAN_FAMILIES = Object.freeze(['team', 'pro', 'plus', 'free', 'all']);
const OUTPUT_FORMATS = Object.freeze(['json', 'csv']);
const STATIC_ASSETS = Object.freeze({
  '/': ['index.html', 'text/html; charset=utf-8'],
  '/index.html': ['index.html', 'text/html; charset=utf-8'],
  '/styles.css': ['styles.css', 'text/css; charset=utf-8'],
  '/app.js': ['app.js', 'text/javascript; charset=utf-8'],
  '/favicon.svg': ['favicon.svg', 'image/svg+xml; charset=utf-8'],
});

class InputError extends Error {}

function defaultDataDirectories(homeDirectory = os.homedir()) {
  return {
    production: path.join(homeDirectory, '.antigravity_cockpit'),
    development: path.join(homeDirectory, '.antigravity_cockpit_dev'),
  };
}

function normalizeFormats(value) {
  const raw = Array.isArray(value) ? value : value === undefined ? OUTPUT_FORMATS : [value];
  const formats = [...new Set(raw.map((item) => String(item).trim().toLowerCase()).filter(Boolean))];
  if (formats.length === 0) {
    throw new InputError('请至少选择一种输出格式。');
  }
  const unsupported = formats.filter((format) => !OUTPUT_FORMATS.includes(format));
  if (unsupported.length > 0) {
    throw new InputError(`不支持的输出格式：${unsupported.join(', ')}`);
  }
  return formats;
}

function normalizeRequestOptions(payload, defaults) {
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) {
    throw new InputError('请求内容必须是 JSON 对象。');
  }
  const profile = String(payload.profile || 'production').trim().toLowerCase();
  if (!['production', 'development'].includes(profile)) {
    throw new InputError(`不支持的数据配置：${profile}`);
  }
  const planFamily = String(payload.planFamily || 'team').trim().toLowerCase();
  if (!PLAN_FAMILIES.includes(planFamily)) {
    throw new InputError(`不支持的套餐范围：${planFamily}`);
  }
  const formats = normalizeFormats(payload.formats);
  let datasets;
  try {
    datasets = normalizeDatasets(payload.datasets);
  } catch (error) {
    throw new InputError(error.message);
  }
  const staleAfterMinutes = Number(
    payload.staleAfterMinutes ?? DEFAULT_STALE_AFTER_MINUTES,
  );
  if (
    !Number.isInteger(staleAfterMinutes) ||
    staleAfterMinutes < 1 ||
    staleAfterMinutes > 10080
  ) {
    throw new InputError('陈旧阈值必须是 1 到 10080 之间的整数分钟。');
  }
  const configuredDataDirectory = String(payload.dataDirectory || '').trim();
  const configuredOutputRoot = String(payload.outputRoot || '').trim();
  return {
    profile,
    planFamily,
    formats,
    format: formats.length === 2 ? 'both' : formats[0],
    datasets,
    staleAfterMinutes,
    skipInvalid: payload.skipInvalid === true,
    dataDirectory: path.resolve(
      configuredDataDirectory || defaults.dataDirectories[profile],
    ),
    outputRoot: path.resolve(configuredOutputRoot || defaults.outputRoot),
  };
}

function pathIsInside(candidatePath, parentPath) {
  const relative = path.relative(path.resolve(parentPath), path.resolve(candidatePath));
  return relative === '' || (!relative.startsWith('..') && !path.isAbsolute(relative));
}

function compactTimestamp(date = new Date()) {
  return date
    .toISOString()
    .replace(/[-:]/gu, '')
    .replace(/\.\d{3}Z$/u, 'Z')
    .replace('T', '-');
}

let cachedWindowsUserSid;

function windowsUserSid() {
  if (cachedWindowsUserSid) {
    return cachedWindowsUserSid;
  }
  const result = childProcess.spawnSync(
    'whoami.exe',
    ['/user', '/fo', 'csv', '/nh'],
    { encoding: 'utf8', windowsHide: true },
  );
  const match = String(result.stdout || '').match(/S-1-(?:\d+-)+\d+/u);
  if (result.status !== 0 || !match) {
    throw new Error('无法确定当前 Windows 用户 SID，导出已停止。');
  }
  cachedWindowsUserSid = match[0];
  return cachedWindowsUserSid;
}

function runIcacls(args, failureMessage) {
  const result = childProcess.spawnSync('icacls.exe', args, {
    encoding: 'utf8',
    windowsHide: true,
  });
  if (result.status !== 0) {
    throw new Error(failureMessage);
  }
}

function hardenOutputDirectory(outputDirectory) {
  if (process.platform !== 'win32') {
    fs.chmodSync(outputDirectory, 0o700);
    return;
  }
  const sid = windowsUserSid();
  runIcacls(
    [outputDirectory, '/inheritance:r'],
    '无法关闭输出目录的继承权限，导出已停止。',
  );
  runIcacls(
    [
      outputDirectory,
      '/grant:r',
      `*${sid}:(OI)(CI)(F)`,
      '*S-1-5-18:(OI)(CI)(F)',
      '*S-1-5-32-544:(OI)(CI)(F)',
    ],
    '无法限制输出目录权限，导出已停止。',
  );
}

function prepareOutputDirectory(options, now = new Date()) {
  if (pathIsInside(options.outputRoot, options.dataDirectory)) {
    throw new InputError('输出根目录不能位于 Cockpit 数据目录内部。');
  }
  fs.mkdirSync(options.outputRoot, { recursive: true, mode: 0o700 });
  const realDataDirectory = fs.realpathSync.native(options.dataDirectory);
  const realOutputRoot = fs.realpathSync.native(options.outputRoot);
  if (pathIsInside(realOutputRoot, realDataDirectory)) {
    throw new InputError('输出根目录解析后位于 Cockpit 数据目录内部。');
  }
  const directoryName = [
    'account-export',
    options.profile,
    compactTimestamp(now),
    crypto.randomBytes(3).toString('hex'),
  ].join('-');
  const outputDirectory = path.join(realOutputRoot, directoryName);
  fs.mkdirSync(outputDirectory, { recursive: false, mode: 0o700 });
  hardenOutputDirectory(outputDirectory);
  return outputDirectory;
}

function createOperationRunner({
  defaultOutputRoot,
  dataDirectories = defaultDataDirectories(),
  exportFunction = exportCockpitAccounts,
  now = () => new Date(),
} = {}) {
  const defaults = {
    dataDirectories,
    outputRoot: path.resolve(
      defaultOutputRoot || path.join(process.cwd(), 'cockpit-account-exports'),
    ),
  };
  return {
    defaults,
    run(mode, payload) {
      const options = normalizeRequestOptions(payload, defaults);
      if (!fs.existsSync(options.dataDirectory)) {
        throw new InputError(`Cockpit 数据目录不存在：${options.dataDirectory}`);
      }
      if (!fs.statSync(options.dataDirectory).isDirectory()) {
        throw new InputError(`Cockpit 数据路径不是目录：${options.dataDirectory}`);
      }
      if (mode === 'validate') {
        return exportFunction({ ...options, validateOnly: true });
      }
      if (mode !== 'export') {
        throw new InputError(`不支持的操作：${mode}`);
      }
      const outputDirectory = prepareOutputDirectory(options, now());
      return exportFunction({ ...options, outputDirectory, validateOnly: false });
    },
  };
}

function constantTimeEquals(left, right) {
  const leftBuffer = Buffer.from(String(left || ''));
  const rightBuffer = Buffer.from(String(right || ''));
  return (
    leftBuffer.length === rightBuffer.length &&
    crypto.timingSafeEqual(leftBuffer, rightBuffer)
  );
}

function cookieValue(request, name) {
  const cookies = String(request.headers.cookie || '').split(';');
  for (const cookie of cookies) {
    const separator = cookie.indexOf('=');
    if (separator < 0) {
      continue;
    }
    if (cookie.slice(0, separator).trim() === name) {
      return decodeURIComponent(cookie.slice(separator + 1).trim());
    }
  }
  return null;
}

function applySecurityHeaders(response, contentType = 'application/json; charset=utf-8') {
  response.setHeader('Content-Type', contentType);
  response.setHeader('Cache-Control', 'no-store, max-age=0');
  response.setHeader('Pragma', 'no-cache');
  response.setHeader('X-Content-Type-Options', 'nosniff');
  response.setHeader('X-Frame-Options', 'DENY');
  response.setHeader('Referrer-Policy', 'no-referrer');
  response.setHeader('Cross-Origin-Resource-Policy', 'same-origin');
  response.setHeader(
    'Content-Security-Policy',
    "default-src 'self'; base-uri 'none'; connect-src 'self'; img-src 'self' data:; " +
      "object-src 'none'; frame-ancestors 'none'; form-action 'self'; " +
      "script-src 'self'; style-src 'self'",
  );
}

function sendJson(response, statusCode, payload) {
  applySecurityHeaders(response);
  response.statusCode = statusCode;
  response.end(`${JSON.stringify(payload)}\n`);
}

function readJsonRequest(request) {
  return new Promise((resolve, reject) => {
    if (!String(request.headers['content-type'] || '').toLowerCase().startsWith('application/json')) {
      reject(new InputError('请求 Content-Type 必须是 application/json。'));
      return;
    }
    const chunks = [];
    let receivedBytes = 0;
    request.on('data', (chunk) => {
      receivedBytes += chunk.length;
      if (receivedBytes > MAX_REQUEST_BYTES) {
        reject(new InputError('请求内容过大。'));
        request.destroy();
        return;
      }
      chunks.push(chunk);
    });
    request.on('end', () => {
      try {
        const text = Buffer.concat(chunks).toString('utf8');
        resolve(text ? JSON.parse(text) : {});
      } catch {
        reject(new InputError('请求 JSON 无法解析。'));
      }
    });
    request.on('error', reject);
  });
}

function publicOperationResult(result, validateOnly) {
  return {
    ok: true,
    validateOnly,
    outputDirectory: result.outputDirectory,
    summary: result.summary,
  };
}

function createExporterWebServer({
  assetsDirectory = path.join(__dirname, 'web'),
  defaultOutputRoot,
  dataDirectories,
  exportFunction,
  now,
} = {}) {
  const operationRunner = createOperationRunner({
    defaultOutputRoot,
    dataDirectories,
    exportFunction,
    now,
  });
  const sessionToken = crypto.randomBytes(32).toString('base64url');
  let expectedHost = null;
  let expectedOrigin = null;
  let operationInProgress = false;

  const server = http.createServer(async (request, response) => {
    if (!expectedHost || String(request.headers.host || '').toLowerCase() !== expectedHost) {
      sendJson(response, 421, { ok: false, error: '无效的本机 Host。' });
      return;
    }
    let pathname;
    try {
      pathname = new URL(request.url, expectedOrigin).pathname;
    } catch {
      sendJson(response, 400, { ok: false, error: '无效的请求地址。' });
      return;
    }

    if (request.method === 'GET' && Object.hasOwn(STATIC_ASSETS, pathname)) {
      const [assetName, contentType] = STATIC_ASSETS[pathname];
      const assetPath = path.join(assetsDirectory, assetName);
      try {
        const bytes = fs.readFileSync(assetPath);
        applySecurityHeaders(response, contentType);
        if (pathname === '/' || pathname === '/index.html') {
          response.setHeader(
            'Set-Cookie',
            `${SESSION_COOKIE_NAME}=${encodeURIComponent(sessionToken)}; HttpOnly; SameSite=Strict; Path=/`,
          );
        }
        response.statusCode = 200;
        response.end(bytes);
      } catch {
        sendJson(response, 500, { ok: false, error: '页面资源读取失败。' });
      }
      return;
    }
    if (request.method === 'GET' && pathname === '/favicon.ico') {
      response.statusCode = 204;
      response.end();
      return;
    }

    const authenticated = constantTimeEquals(
      cookieValue(request, SESSION_COOKIE_NAME),
      sessionToken,
    );
    if (!authenticated) {
      sendJson(response, 401, { ok: false, error: '本地页面会话无效，请刷新页面。' });
      return;
    }

    if (request.method === 'GET' && pathname === '/api/defaults') {
      sendJson(response, 200, {
        ok: true,
        exporterVersion: EXPORTER_VERSION,
        defaults: {
          profile: 'production',
          planFamily: 'team',
          formats: [...OUTPUT_FORMATS],
          datasets: [...DATASET_NAMES],
          staleAfterMinutes: DEFAULT_STALE_AFTER_MINUTES,
          skipInvalid: false,
          dataDirectories: operationRunner.defaults.dataDirectories,
          outputRoot: operationRunner.defaults.outputRoot,
        },
        sensitiveFieldsExcluded: EXCLUDED_SENSITIVE_FIELDS,
      });
      return;
    }

    if (request.method !== 'POST' || !['/api/validate', '/api/export', '/api/shutdown'].includes(pathname)) {
      sendJson(response, 404, { ok: false, error: '没有找到该页面或接口。' });
      return;
    }
    if (String(request.headers.origin || '') !== expectedOrigin) {
      sendJson(response, 403, { ok: false, error: '拒绝跨来源操作。' });
      return;
    }
    if (pathname === '/api/shutdown') {
      sendJson(response, 200, { ok: true, message: '本地导出页面服务正在关闭。' });
      setImmediate(() => server.close());
      return;
    }
    if (operationInProgress) {
      sendJson(response, 409, { ok: false, error: '另一个导出操作正在执行，请稍候。' });
      return;
    }

    operationInProgress = true;
    try {
      const payload = await readJsonRequest(request);
      const mode = pathname === '/api/validate' ? 'validate' : 'export';
      const result = operationRunner.run(mode, payload);
      sendJson(response, 200, publicOperationResult(result, mode === 'validate'));
    } catch (error) {
      const statusCode = error instanceof InputError ? 400 : 422;
      sendJson(response, statusCode, {
        ok: false,
        error: error && error.message ? error.message : '导出操作失败。',
      });
    } finally {
      operationInProgress = false;
    }
  });

  server.on('listening', () => {
    const address = server.address();
    expectedHost = `127.0.0.1:${address.port}`;
    expectedOrigin = `http://${expectedHost}`;
  });
  server.keepAliveTimeout = 5000;
  server.requestTimeout = 30_000;

  return {
    server,
    listen(port = 0) {
      return new Promise((resolve, reject) => {
        const onError = (error) => reject(error);
        server.once('error', onError);
        server.listen({ host: '127.0.0.1', port, exclusive: true }, () => {
          server.off('error', onError);
          const address = server.address();
          resolve({
            host: '127.0.0.1',
            port: address.port,
            origin: `http://127.0.0.1:${address.port}`,
          });
        });
      });
    },
  };
}

module.exports = {
  InputError,
  MAX_REQUEST_BYTES,
  OUTPUT_FORMATS,
  PLAN_FAMILIES,
  compactTimestamp,
  createExporterWebServer,
  createOperationRunner,
  defaultDataDirectories,
  hardenOutputDirectory,
  normalizeFormats,
  normalizeRequestOptions,
  pathIsInside,
  prepareOutputDirectory,
  publicOperationResult,
};
