const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const os = require('node:os');
const path = require('node:path');
const { execFile } = require('node:child_process');
const { promisify } = require('node:util');
const { refreshCatalog, parseArgs } = require('./refresh-codex-model-catalog.cjs');

const execFileAsync = promisify(execFile);

const MODEL_ID = 'gpt-6.1-sol';
const API_KEY = 'fixture-secret-do-not-log';
const CLIENT_VERSION = '0.153.4';

function model(overrides = {}) {
  return {
    slug: MODEL_ID,
    display_name: 'GPT-6.1 Sol',
    description: 'A complete fixture entry, not reconstructed from an OpenAI model ID.',
    visibility: 'list',
    context_window: 272000,
    max_context_window: 872000,
    auto_compact_token_limit: 244800,
    default_reasoning_level: 'low',
    supported_reasoning_levels: [
      { effort: 'low', description: 'Low effort' },
      { effort: 'ultra', description: 'Ultra effort' },
    ],
    minimal_client_version: '0.144.0',
    future_capability: { preserve: ['verbatim', 42] },
    ...overrides,
  };
}

function existingCatalog() {
  return {
    models: [
      {
        slug: 'gpt-6-astra',
        display_name: 'My unchanged Astra',
        context_window: 516000,
        default_reasoning_level: 'max',
        supported_reasoning_levels: [
          { effort: 'max', description: 'User custom Max' },
          { effort: 'ultra', description: 'User custom Ultra' },
        ],
        custom_field: { owner: 'user' },
      },
    ],
    etag: 'keep-user-metadata',
    future_root_field: { enabled: false, nested: [1, 2, 3] },
  };
}

function fixture(t, catalog = existingCatalog()) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cockpit-catalog-refresh-test-'));
  const catalogPath = path.join(dir, 'models.json');
  const original = Buffer.from(` ${JSON.stringify(catalog, null, 2)}\n\n`, 'utf8');
  fs.writeFileSync(catalogPath, original, { mode: 0o600 });
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return { dir, catalogPath, original, catalog };
}

async function serverFixture(t, responder = (_req, res) => {
  res.setHeader('Content-Type', 'application/json');
  res.end(JSON.stringify({ models: [model()] }));
}) {
  const requests = [];
  const server = http.createServer((req, res) => {
    requests.push({ url: req.url, method: req.method, authorization: req.headers.authorization });
    Promise.resolve().then(() => responder(req, res)).catch(() => {
      if (!res.headersSent) res.statusCode = 500;
      res.end('fixture responder failed');
    });
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  t.after(() => new Promise((resolve) => {
    server.close(resolve);
    server.closeAllConnections();
  }));
  const baseUrl = `http://127.0.0.1:${server.address().port}/v1`;
  return { baseUrl, requests };
}

function options(files, server, overrides = {}) {
  return {
    catalogPath: files.catalogPath,
    baseUrl: server.baseUrl,
    clientVersion: CLIENT_VERSION,
    modelIds: [MODEL_ID],
    apiKey: API_KEY,
    dryRun: false,
    allowInsecureHttp: false,
    timeoutMs: 2000,
    ...overrides,
  };
}

function assertOriginalOnly(files) {
  assert.deepEqual(fs.readFileSync(files.catalogPath), files.original);
  assert.deepEqual(fs.readdirSync(files.dir), [path.basename(files.catalogPath)]);
}

async function captureRejection(promise) {
  let caught;
  try {
    await promise;
  } catch (error) {
    caught = error;
  }
  assert.ok(caught instanceof Error, 'the operation must reject with an Error');
  return caught;
}

test('requests the authenticated Codex catalog and appends the complete selected entry', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t);
  const originalMode = fs.statSync(files.catalogPath).mode & 0o777;
  const result = await refreshCatalog(options(files, server));
  assert.deepEqual(server.requests, [{
    url: `/v1/models?client_version=${CLIENT_VERSION}`,
    method: 'GET',
    authorization: `Bearer ${API_KEY}`,
  }]);
  const updated = JSON.parse(fs.readFileSync(files.catalogPath, 'utf8'));
  assert.deepEqual(updated, { ...files.catalog, models: [...files.catalog.models, model()] });
  assert.deepEqual(result.added, [MODEL_ID]);
  assert.deepEqual(result.unchanged, []);
  assert.equal(result.dryRun, false);
  assert.equal(typeof result.backupPath, 'string');
  assert.notEqual(result.backupPath, files.catalogPath);
  assert.equal(path.dirname(result.backupPath), files.dir);
  assert.deepEqual(fs.readFileSync(result.backupPath), files.original);
  assert.equal(fs.statSync(result.backupPath).mode & 0o777, originalMode);
  assert.equal(fs.statSync(files.catalogPath).mode & 0o777, originalMode);
  assert.equal(fs.readdirSync(files.dir).length, 2, 'successful writes leave only catalog and backup');
});

test('dry-run reports additions without changing bytes or creating backup, lock, or temp files', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t);
  const result = await refreshCatalog(options(files, server, { dryRun: true }));
  assert.deepEqual(result.added, [MODEL_ID]);
  assert.equal(result.backupPath, null);
  assert.equal(result.dryRun, true);
  assertOriginalOnly(files);
});

test('an existing requested model is never overwritten and no backup is created', async (t) => {
  const original = existingCatalog();
  const customized = model({ display_name: 'User custom Sol', default_reasoning_level: 'ultra' });
  original.models.push(customized);
  const files = fixture(t, original);
  const server = await serverFixture(t);
  const result = await refreshCatalog(options(files, server));
  assert.deepEqual(result.added, []);
  assert.deepEqual(result.unchanged, [MODEL_ID]);
  assert.equal(result.backupPath, null);
  assert.equal(server.requests.length, 0, 'no-op runs do not need a gateway request');
  assertOriginalOnly(files);
});

test('only explicitly requested missing models are added, not every model returned by the server', async (t) => {
  const files = fixture(t);
  const selected = model();
  const server = await serverFixture(t, (_req, res) => {
    res.end(JSON.stringify({ models: [model({ slug: 'not-requested' }), selected] }));
  });
  const result = await refreshCatalog(options(files, server, { modelIds: [MODEL_ID, 'gpt-6-astra'] }));
  assert.deepEqual(result.added, [MODEL_ID]);
  assert.deepEqual(result.unchanged, ['gpt-6-astra']);
  const updated = JSON.parse(fs.readFileSync(files.catalogPath, 'utf8'));
  assert.deepEqual(updated.models.map((entry) => entry.slug), ['gpt-6-astra', MODEL_ID]);
  assert.deepEqual(updated.models[0], files.catalog.models[0]);
});

test('rejects a standard OpenAI data list instead of inventing a Codex entry', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t, (_req, res) => {
    res.end(JSON.stringify({ object: 'list', data: [{ id: MODEL_ID, object: 'model' }] }));
  });
  await assert.rejects(refreshCatalog(options(files, server)), /catalog|models|Codex/i);
  assertOriginalOnly(files);
});

test('rejects missing requested models without changing the original catalog', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t, (_req, res) => res.end(JSON.stringify({ models: [] })));
  await assert.rejects(refreshCatalog(options(files, server)), /missing|available|found|absent/i);
  assertOriginalOnly(files);
});

test('rejects ambiguous duplicate selected slugs in the upstream catalog', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t, (_req, res) => {
    res.end(JSON.stringify({ models: [model(), model({ display_name: 'Conflicting duplicate' })] }));
  });
  await assert.rejects(refreshCatalog(options(files, server)), /duplicate|ambiguous/i);
  assertOriginalOnly(files);
});

for (const [label, overrides] of [
  ['missing display name', { display_name: undefined }],
  ['missing description', { description: undefined }],
  ['hidden model', { visibility: 'hide' }],
  ['zero context window', { context_window: 0 }],
  ['fractional context window', { context_window: 1.5 }],
  ['missing default reasoning level', { default_reasoning_level: undefined }],
  ['missing reasoning levels', { supported_reasoning_levels: undefined }],
  ['invalid reasoning description', { supported_reasoning_levels: [{ effort: 'ultra', description: 123 }] }],
  ['unknown reasoning effort', { supported_reasoning_levels: [
    { effort: 'low', description: 'Low effort' },
    { effort: 'super-ultra', description: 'Unsupported effort' },
  ] }],
  ['duplicate reasoning effort', { supported_reasoning_levels: [
    { effort: 'low', description: 'Low effort' },
    { effort: 'low', description: 'Ambiguous duplicate' },
  ] }],
]) {
  test(`rejects an incomplete selected model: ${label}`, async (t) => {
    const files = fixture(t);
    const server = await serverFixture(t, (_req, res) => {
      res.end(JSON.stringify({ models: [model(overrides)] }));
    });
    await assert.rejects(refreshCatalog(options(files, server)));
    assertOriginalOnly(files);
  });
}

test('respects the official minimal_client_version requirement', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t, (_req, res) => {
    res.end(JSON.stringify({ models: [model({ minimal_client_version: '0.154.0' })] }));
  });
  await assert.rejects(refreshCatalog(options(files, server)), /version|client|requires/i);
  assertOriginalOnly(files);
});

test('compares client versions numerically rather than lexicographically', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t, (_req, res) => {
    res.end(JSON.stringify({ models: [model({ minimal_client_version: '0.99.0' })] }));
  });
  const result = await refreshCatalog(options(files, server, { dryRun: true }));
  assert.deepEqual(result.added, [MODEL_ID]);
  assertOriginalOnly(files);
});

test('HTTP authentication errors expose neither the response body nor the API key', async (t) => {
  const files = fixture(t);
  const responseMarker = 'private-response-body-do-not-log';
  const server = await serverFixture(t, (_req, res) => {
    res.statusCode = 401;
    res.end(`${responseMarker}: ${API_KEY}`);
  });
  const error = await captureRejection(refreshCatalog(options(files, server)));
  assert.match(error.message, /401/);
  assert.ok(!String(error.stack).includes(responseMarker));
  assert.ok(!String(error.stack).includes(API_KEY));
  assertOriginalOnly(files);
});

test('JSON parse failures expose no upstream response body', async (t) => {
  const files = fixture(t);
  const responseMarker = 'private-malformed-json-do-not-log';
  const server = await serverFixture(t, (_req, res) => res.end(`${responseMarker}: ${API_KEY}`));
  const error = await captureRejection(refreshCatalog(options(files, server)));
  assert.ok(!String(error.stack).includes(responseMarker));
  assert.ok(!String(error.stack).includes(API_KEY));
  assertOriginalOnly(files);
});

test('rejects redirects without forwarding the bearer token to another endpoint', async (t) => {
  const files = fixture(t);
  const destination = await serverFixture(t);
  const source = await serverFixture(t, (_req, res) => {
    res.statusCode = 302;
    res.setHeader('Location', `${destination.baseUrl}/models?client_version=${CLIENT_VERSION}`);
    res.end();
  });
  const error = await captureRejection(refreshCatalog(options(files, source)));
  assert.ok(!String(error.stack).includes(API_KEY));
  assert.equal(destination.requests.length, 0);
  assertOriginalOnly(files);
});

test('requires explicit insecure HTTP opt-in for a non-loopback LAN address', async (t) => {
  const files = fixture(t);
  await assert.rejects(refreshCatalog(options(files, { baseUrl: 'http://192.0.2.1:1/v1' })), /HTTP|insecure|allow-insecure/i);
  assertOriginalOnly(files);
});

for (const [label, baseUrl] of [
  ['embedded credentials', 'http://user:password@127.0.0.1:1/v1'],
  ['query strings', 'http://127.0.0.1:1/v1?api_key=do-not-log'],
  ['fragments', 'http://127.0.0.1:1/v1#token'],
  ['non-HTTP protocols', 'file:///tmp/models.json'],
]) {
  test(`rejects a Base URL containing ${label}`, async (t) => {
    const files = fixture(t);
    await assert.rejects(refreshCatalog(options(files, { baseUrl })));
    assertOriginalOnly(files);
  });
}

test('rejects a directory in place of a catalog before fetching', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t);
  const directoryPath = path.join(files.dir, 'not-a-file');
  fs.mkdirSync(directoryPath);
  await assert.rejects(refreshCatalog(options(files, server, { catalogPath: directoryPath })), /regular|file|directory/i);
  assert.equal(server.requests.length, 0);
  assert.deepEqual(fs.readFileSync(files.catalogPath), files.original);
});

test('rejects symlink catalogs without changing their target', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t);
  const linkPath = path.join(files.dir, 'catalog-link.json');
  try {
    fs.symlinkSync(files.catalogPath, linkPath, 'file');
  } catch (error) {
    if (process.platform === 'win32' && ['EPERM', 'EACCES', 'ENOTSUP'].includes(error.code)) {
      t.skip('Windows account does not have symbolic-link creation privileges');
      return;
    }
    throw error;
  }
  await assert.rejects(refreshCatalog(options(files, server, { catalogPath: linkPath })), /link|regular|file/i);
  assert.deepEqual(fs.readFileSync(files.catalogPath), files.original);
  assert.equal(server.requests.length, 0);
  assert.ok(fs.lstatSync(linkPath).isSymbolicLink());
});

test('aborts if catalog bytes change while the upstream request is in flight', async (t) => {
  const files = fixture(t);
  const externallyUpdated = Buffer.from(JSON.stringify({ ...files.catalog, external_writer: true }));
  const server = await serverFixture(t, (_req, res) => {
    fs.writeFileSync(files.catalogPath, externallyUpdated);
    res.end(JSON.stringify({ models: [model()] }));
  });
  await assert.rejects(refreshCatalog(options(files, server)), /changed|conflict|concurrent/i);
  assert.deepEqual(fs.readFileSync(files.catalogPath), externallyUpdated);
  assert.deepEqual(fs.readdirSync(files.dir), [path.basename(files.catalogPath)]);
});

test('aborts if a different file replaces the catalog with identical bytes during fetch', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t, (_req, res) => {
    const replacement = path.join(files.dir, 'external-replacement.json');
    fs.writeFileSync(replacement, files.original, { mode: 0o600 });
    fs.renameSync(replacement, files.catalogPath);
    res.end(JSON.stringify({ models: [model()] }));
  });
  await assert.rejects(refreshCatalog(options(files, server)), /changed|conflict|concurrent|identity/i);
  assertOriginalOnly(files);
});

test('a lock owned by another writer is not removed or overwritten', async (t) => {
  const files = fixture(t);
  const lockPath = `${files.catalogPath}.refresh.lock`;
  fs.writeFileSync(lockPath, 'other-writer-lock', { flag: 'wx' });
  const server = await serverFixture(t);
  await assert.rejects(refreshCatalog(options(files, server)), /lock|busy|another/i);
  assert.equal(fs.readFileSync(lockPath, 'utf8'), 'other-writer-lock');
  assert.deepEqual(fs.readFileSync(files.catalogPath), files.original);
  assert.equal(fs.readdirSync(files.dir).length, 2);
});

test('times out a stalled upstream without modifying the catalog or leaving temporary files', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t, () => {});
  const error = await captureRejection(refreshCatalog(options(files, server, { timeoutMs: 30 })));
  assert.ok(!String(error.stack).includes(API_KEY));
  assertOriginalOnly(files);
});

test('atomic replacement failures preserve the original and completed backup, removing owned lock and temp', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t);
  const originalRename = fs.renameSync;
  t.mock.method(fs, 'renameSync', (source, destination) => {
    if (destination === files.catalogPath) {
      const error = new Error('Injected atomic replacement failure');
      error.code = 'EACCES';
      throw error;
    }
    return originalRename(source, destination);
  });
  await assert.rejects(refreshCatalog(options(files, server)), /replacement failure/);
  assert.deepEqual(fs.readFileSync(files.catalogPath), files.original);
  const entries = fs.readdirSync(files.dir);
  const backups = entries.filter((entry) => entry.includes('.bak-'));
  assert.equal(backups.length, 1, 'a completed recovery backup must not be discarded');
  assert.deepEqual(fs.readFileSync(path.join(files.dir, backups[0])), files.original);
  assert.equal(entries.length, 2, 'own lock and temporary file must be cleaned');
});

test('a failed exclusive temp creation never removes a file owned by another writer', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t);
  const originalOpen = fs.openSync;
  let collidedPath;
  t.mock.method(fs, 'openSync', (filePath, flags, mode) => {
    if (!collidedPath && typeof filePath === 'string' && filePath.includes('.refresh-') && filePath.endsWith('.tmp')) {
      collidedPath = filePath;
      const fd = originalOpen(filePath, 'wx', 0o600);
      try {
        fs.writeFileSync(fd, 'another-writer-temp');
      } finally {
        fs.closeSync(fd);
      }
    }
    return originalOpen(filePath, flags, mode);
  });
  await assert.rejects(refreshCatalog(options(files, server)));
  assert.ok(collidedPath, 'the test must actually exercise exclusive temp creation');
  assert.equal(fs.readFileSync(collidedPath, 'utf8'), 'another-writer-temp');
  assert.deepEqual(fs.readFileSync(files.catalogPath), files.original);
  assert.equal(fs.existsSync(`${files.catalogPath}.refresh.lock`), false);
});

test('two simultaneous refreshes cannot append duplicates or overwrite each other', async (t) => {
  const files = fixture(t);
  let releaseResponses;
  const bothRequests = new Promise((resolve) => { releaseResponses = resolve; });
  let requestCount = 0;
  const server = await serverFixture(t, async (_req, res) => {
    requestCount += 1;
    if (requestCount === 2) releaseResponses();
    await bothRequests;
    res.end(JSON.stringify({ models: [model()] }));
  });
  const results = await Promise.allSettled([
    refreshCatalog(options(files, server)),
    refreshCatalog(options(files, server)),
  ]);
  assert.equal(results.filter((result) => result.status === 'fulfilled').length, 1);
  assert.equal(results.filter((result) => result.status === 'rejected').length, 1);
  const updated = JSON.parse(fs.readFileSync(files.catalogPath, 'utf8'));
  assert.deepEqual(updated.models, [...files.catalog.models, model()]);
  assert.equal(fs.readdirSync(files.dir).length, 2, 'one catalog and one recovery backup remain');
});

test('the real CLI reads its bearer key from an environment variable and dry-runs without exposing it', async (t) => {
  const files = fixture(t);
  const server = await serverFixture(t);
  const result = await execFileAsync(process.execPath, [
    path.join(__dirname, 'refresh-codex-model-catalog.cjs'),
    '--catalog', files.catalogPath,
    '--base-url', server.baseUrl,
    '--client-version', CLIENT_VERSION,
    '--model', MODEL_ID,
    '--api-key-env', 'COCKPIT_REFRESH_TEST_KEY',
    '--dry-run',
  ], { env: { ...process.env, COCKPIT_REFRESH_TEST_KEY: API_KEY }, timeout: 5000 });
  assert.deepEqual(JSON.parse(result.stdout).added, [MODEL_ID]);
  assert.equal(JSON.parse(result.stdout).dryRun, true);
  assert.ok(!result.stdout.includes(API_KEY));
  assert.ok(!result.stderr.includes(API_KEY));
  assert.equal(server.requests[0].authorization, `Bearer ${API_KEY}`);
  assertOriginalOnly(files);
});

test('argument parsing rejects unknown and repeated singleton options', () => {
  assert.throws(() => parseArgs(['--not-a-real-option']), /unknown|unexpected|unsupported/i);
  assert.throws(() => parseArgs(['--catalog', 'one.json', '--catalog', 'two.json']), /duplicate|repeated|once/i);
});

test('help can be requested without required operational options', () => {
  assert.equal(parseArgs(['--help']).help, true);
});
