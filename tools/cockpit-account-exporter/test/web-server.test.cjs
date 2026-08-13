'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const { parseArguments } = require('../cockpit-account-exporter-web.cjs');
const {
  InputError,
  createExporterWebServer,
  normalizeFormats,
  normalizeRequestOptions,
  pathIsInside,
} = require('../web-server.cjs');

function httpRequest({ origin, pathname, method = 'GET', cookie, requestOrigin, body }) {
  const target = new URL(pathname, origin);
  return new Promise((resolve, reject) => {
    const headers = {};
    if (cookie) {
      headers.Cookie = cookie;
    }
    if (requestOrigin) {
      headers.Origin = requestOrigin;
    }
    let encodedBody;
    if (body !== undefined) {
      encodedBody = Buffer.from(JSON.stringify(body), 'utf8');
      headers['Content-Type'] = 'application/json';
      headers['Content-Length'] = encodedBody.length;
    }
    const request = http.request(
      {
        hostname: target.hostname,
        port: target.port,
        path: target.pathname,
        method,
        headers,
      },
      (response) => {
        const chunks = [];
        response.on('data', (chunk) => chunks.push(chunk));
        response.on('end', () => {
          const text = Buffer.concat(chunks).toString('utf8');
          resolve({ statusCode: response.statusCode, headers: response.headers, text });
        });
      },
    );
    request.on('error', reject);
    if (encodedBody) {
      request.write(encodedBody);
    }
    request.end();
  });
}

test('normalizes web options and rejects unsupported or unsafe selections', () => {
  const defaults = {
    dataDirectories: {
      production: 'C:\\data\\production',
      development: 'C:\\data\\development',
    },
    outputRoot: 'C:\\exports',
  };
  const options = normalizeRequestOptions(
    {
      profile: 'production',
      planFamily: 'all',
      datasets: ['quota', 'gateway'],
      formats: ['csv'],
      staleAfterMinutes: 60,
      skipInvalid: true,
    },
    defaults,
  );
  assert.equal(options.format, 'csv');
  assert.deepEqual(options.datasets, ['quota', 'gateway']);
  assert.equal(options.skipInvalid, true);
  assert.deepEqual(normalizeFormats(['csv', 'csv', 'json']), ['csv', 'json']);
  assert.throws(
    () => normalizeRequestOptions({ formats: [], datasets: ['accounts'] }, defaults),
    InputError,
  );
  assert.throws(
    () => normalizeRequestOptions({ formats: ['json'], datasets: [] }, defaults),
    InputError,
  );
  assert.equal(pathIsInside('C:\\data\\production\\exports', 'C:\\data\\production'), true);
  assert.equal(pathIsInside('C:\\exports', 'C:\\data\\production'), false);
  assert.equal(parseArguments(['--port', '0', '--no-open']).openBrowser, false);
  assert.throws(() => parseArguments(['--port', '70000']), /between 0 and 65535/u);
});

test('serves the local page and protects API operations with Host, cookie, and Origin checks', async (t) => {
  const temporaryDirectory = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-export-web-test-'));
  t.after(() => fs.rmSync(temporaryDirectory, { recursive: true, force: true }));
  const secretRecord = { tokens: { access_token: 'must-never-reach-browser' } };
  const fakeExport = (options) => ({
    outputDirectory: null,
    records: [secretRecord],
    summary: {
      accountCount: 2,
      apiPoolAccountCount: 1,
      exhaustedAccountCount: 1,
      staleCount: 2,
      requiresReauthCount: 0,
      skippedAccountCount: 0,
      selectedDatasets: options.datasets,
      outputFiles: [],
    },
  });
  const application = createExporterWebServer({
    defaultOutputRoot: path.join(temporaryDirectory, 'exports'),
    dataDirectories: {
      production: temporaryDirectory,
      development: temporaryDirectory,
    },
    exportFunction: fakeExport,
  });
  const address = await application.listen(0);
  t.after(
    () =>
      new Promise((resolve) => {
        if (!application.server.listening) {
          resolve();
          return;
        }
        application.server.close(resolve);
      }),
  );

  const page = await httpRequest({ origin: address.origin, pathname: '/' });
  assert.equal(page.statusCode, 200);
  assert.match(page.text, /选择需要的数据/u);
  assert.match(page.headers['content-security-policy'], /default-src 'self'/u);
  const cookie = page.headers['set-cookie'][0].split(';')[0];
  assert.match(cookie, /^cockpit_exporter_session=/u);

  const unauthenticated = await httpRequest({
    origin: address.origin,
    pathname: '/api/defaults',
  });
  assert.equal(unauthenticated.statusCode, 401);

  const defaults = await httpRequest({
    origin: address.origin,
    pathname: '/api/defaults',
    cookie,
  });
  assert.equal(defaults.statusCode, 200);
  assert.equal(JSON.parse(defaults.text).defaults.profile, 'production');

  const foreignOrigin = await httpRequest({
    origin: address.origin,
    pathname: '/api/validate',
    method: 'POST',
    cookie,
    requestOrigin: 'https://example.invalid',
    body: { datasets: ['accounts'], formats: ['json'] },
  });
  assert.equal(foreignOrigin.statusCode, 403);

  const validated = await httpRequest({
    origin: address.origin,
    pathname: '/api/validate',
    method: 'POST',
    cookie,
    requestOrigin: address.origin,
    body: {
      profile: 'production',
      planFamily: 'team',
      datasets: ['accounts'],
      formats: ['json'],
      dataDirectory: temporaryDirectory,
      outputRoot: path.join(temporaryDirectory, 'exports'),
      staleAfterMinutes: 15,
    },
  });
  assert.equal(validated.statusCode, 200);
  const validatedPayload = JSON.parse(validated.text);
  assert.equal(validatedPayload.summary.accountCount, 2);
  assert.equal(Object.hasOwn(validatedPayload, 'records'), false);
  assert.equal(validated.text.includes(secretRecord.tokens.access_token), false);
});
