const fs = require('node:fs');
const path = require('node:path');
const { app, BrowserWindow, session, shell } = require('electron');

function parseArgs(argv) {
  const result = {};
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (!arg.startsWith('--')) continue;
    const key = arg.slice(2);
    const next = argv[i + 1];
    if (next && !next.startsWith('--')) {
      result[key] = next;
      i += 1;
    } else {
      result[key] = 'true';
    }
  }
  return result;
}

const args = parseArgs(process.argv.slice(2));
const userDataDir = args['user-data-dir'];
const statusFile = args['status-file'];
const exportFile = args['export-file'];
const cookieFile = args['cookie-file'];
const startUrl = args.url || 'https://claude.ai/';
const mode = args.mode || 'auth';
const probeTimeoutMs = Number.parseInt(args['probe-timeout-ms'] || '15000', 10);

if (!userDataDir || !statusFile || !exportFile) {
  console.error('Missing required args: --user-data-dir, --status-file, --export-file');
  process.exit(2);
}

fs.mkdirSync(userDataDir, { recursive: true });
fs.mkdirSync(path.dirname(statusFile), { recursive: true });
fs.mkdirSync(path.dirname(exportFile), { recursive: true });

app.setName('Cockpit Claude Auth');
app.setPath('userData', userDataDir);
app.setPath('logs', path.join(userDataDir, 'Logs'));
app.commandLine.appendSwitch('disable-features', 'CalculateNativeWinOcclusion');
if (process.platform === 'darwin' && (mode === 'probe' || mode === 'cookie_probe') && app.dock) {
  app.dock.hide();
}

function writeJsonAtomic(file, data) {
  const tmp = `${file}.tmp`;
  fs.writeFileSync(tmp, `${JSON.stringify(data, null, 2)}\n`, 'utf8');
  fs.renameSync(tmp, file);
}

function readJsonFile(file) {
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

function publicCookie(cookie) {
  return {
    name: cookie.name,
    value: cookie.value,
    domain: cookie.domain,
    hostOnly: cookie.hostOnly,
    path: cookie.path || '/',
    secure: Boolean(cookie.secure),
    httpOnly: Boolean(cookie.httpOnly),
    session: Boolean(cookie.session),
    expirationDate: cookie.expirationDate || null,
    sameSite: cookie.sameSite || 'unspecified',
  };
}

function isClaudeCookie(cookie) {
  const domain = String(cookie.domain || '').toLowerCase();
  return domain === 'claude.ai' ||
    domain === '.claude.ai' ||
    domain.endsWith('.claude.ai') ||
    domain === 'claude.com' ||
    domain === '.claude.com' ||
    domain.endsWith('.claude.com');
}

function isUuid(value) {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value);
}

async function readClaudeCookies() {
  const cookies = await session.defaultSession.cookies.get({});
  return cookies.filter(isClaudeCookie).map(publicCookie);
}

function findCookie(cookies, name) {
  return cookies.find((cookie) => cookie.name === name && cookie.value);
}

function isClaudeAppUrl(url) {
  return /^https:\/\/([^/]+\.)?claude\.ai\b/i.test(String(url || ''));
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function loadUrlWithTimeout(win, url, timeoutMs) {
  return Promise.race([
    win.loadURL(url),
    new Promise((resolve) => setTimeout(resolve, Math.max(1000, timeoutMs))),
  ]);
}

let cachedWebProfile = null;
let cachedWebProfileKey = '';

function cookieHeader(cookies) {
  return cookies
    .filter((cookie) => cookie.name && cookie.value && isClaudeCookie(cookie))
    .map((cookie) => `${cookie.name}=${cookie.value}`)
    .join('; ');
}

async function fetchJsonWithCookies(url, cookies, extraHeaders = {}) {
  const cookie = cookieHeader(cookies);
  if (!cookie) {
    throw new Error('missing Claude cookies');
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 10000);
  try {
    const response = await fetch(url, {
      method: 'GET',
      redirect: 'manual',
      headers: {
        accept: 'application/json',
        'user-agent': 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126 Safari/537.36',
        origin: 'https://claude.ai',
        referer: 'https://claude.ai/',
        'sec-fetch-site': 'same-origin',
        'sec-fetch-mode': 'cors',
        'sec-fetch-dest': 'empty',
        cookie,
        ...extraHeaders,
      },
      signal: controller.signal,
    });
    const text = await response.text();
    if (!response.ok) {
      throw new Error(`HTTP ${response.status} ${text.slice(0, 500)}`);
    }
    return text ? JSON.parse(text) : null;
  } finally {
    clearTimeout(timer);
  }
}

async function fetchClaudeWebProfileInPage(webContents, lastActiveOrg) {
  if (!webContents || webContents.isDestroyed()) {
    throw new Error('missing Claude page context');
  }
  if (!isClaudeAppUrl(webContents.getURL())) {
    throw new Error(`Claude page context is not active: ${webContents.getURL() || 'blank'}`);
  }
  const script = `
    (async () => {
      const lastActiveOrg = ${JSON.stringify(lastActiveOrg || '')};
      const orgHeaders = lastActiveOrg ? { 'x-organization-uuid': lastActiveOrg } : {};
      const result = {
        version: 1,
        fetchContext: 'page',
        lastActiveOrg,
        fetchedAt: new Date().toISOString(),
        endpoints: {},
      };
      const errors = {};

      async function fetchEndpoint(key, url, headers = {}, options = {}) {
        try {
          const response = await fetch(url, {
            method: 'GET',
            credentials: 'include',
            redirect: 'follow',
            referrer: options.referrer || 'https://claude.ai/',
            headers: {
              accept: 'application/json',
              ...headers,
            },
          });
          const text = await response.text();
          if (!response.ok) {
            throw new Error(\`HTTP \${response.status} \${text.slice(0, 500)}\`);
          }
          result.endpoints[key] = text ? JSON.parse(text) : null;
        } catch (error) {
          errors[key] = String(error && error.message ? error.message : error);
        }
      }

      await fetchEndpoint('accountProfile', 'https://claude.ai/api/account_profile', orgHeaders);
      await fetchEndpoint('account', 'https://claude.ai/api/account', orgHeaders);

      if (lastActiveOrg) {
        const bootstrapUrl =
          \`https://claude.ai/api/bootstrap/\${encodeURIComponent(lastActiveOrg)}\` +
          '/app_start?statsig_hashing_algorithm=djb2&growthbook_format=sdk&include_system_prompts=false';
        await fetchEndpoint('bootstrapAppStart', bootstrapUrl, orgHeaders);
        const orgBase = \`https://claude.ai/api/organizations/\${encodeURIComponent(lastActiveOrg)}\`;
        const usageReferrer = 'https://claude.ai/settings/usage';
        await fetchEndpoint('organizationUsage', \`\${orgBase}/usage\`, orgHeaders, {
          referrer: usageReferrer,
        });
        await fetchEndpoint('subscriptionDetails', \`\${orgBase}/subscription_details\`, orgHeaders, {
          referrer: usageReferrer,
        });
        await fetchEndpoint('overageSpendLimit', \`\${orgBase}/overage_spend_limit\`, orgHeaders, {
          referrer: usageReferrer,
        });
      } else {
        errors.bootstrapAppStart = 'missing lastActiveOrg';
        errors.organizationUsage = 'missing lastActiveOrg';
        errors.subscriptionDetails = 'missing lastActiveOrg';
        errors.overageSpendLimit = 'missing lastActiveOrg';
      }

      if (Object.keys(errors).length > 0) {
        result.errors = errors;
      }
      return result;
    })()
  `;
  return webContents.executeJavaScript(script, true);
}

function cookieUrl(cookie) {
  const domain = String(cookie.domain || 'claude.ai').trim().replace(/^\./, '') || 'claude.ai';
  const cookiePath = String(cookie.path || '/').trim() || '/';
  return `https://${domain}${cookiePath.startsWith('/') ? cookiePath : `/${cookiePath}`}`;
}

async function importCookiesFromFile(file) {
  if (!file) {
    throw new Error('missing --cookie-file');
  }
  const payload = readJsonFile(file);
  const cookies = Array.isArray(payload.cookies) ? payload.cookies : [];
  for (const cookie of cookies) {
    if (!cookie || !cookie.name || !cookie.value || !isClaudeCookie(cookie)) {
      continue;
    }
    const imported = {
      url: cookieUrl(cookie),
      name: String(cookie.name),
      value: String(cookie.value),
      domain: cookie.domain || undefined,
      path: cookie.path || '/',
      secure: cookie.secure !== false,
      httpOnly: Boolean(cookie.httpOnly),
    };
    if (cookie.expirationDate) {
      imported.expirationDate = Number(cookie.expirationDate);
    }
    if (cookie.sameSite && cookie.sameSite !== 'unspecified') {
      imported.sameSite = cookie.sameSite;
    }
    await session.defaultSession.cookies.set(imported);
  }
}

async function fetchClaudeWebProfileWithCookies(cookies, pageContextError) {
  const lastActiveOrg = findCookie(cookies, 'lastActiveOrg')?.value;
  const orgHeaders = lastActiveOrg ? { 'x-organization-uuid': lastActiveOrg } : {};
  const result = {
    version: 1,
    fetchContext: 'cookie',
    lastActiveOrg,
    fetchedAt: new Date().toISOString(),
    endpoints: {},
  };
  const errors = {};
  if (pageContextError) {
    errors.pageContext = pageContextError;
  }

  async function fetchEndpoint(key, url, headers = {}) {
    try {
      result.endpoints[key] = await fetchJsonWithCookies(url, cookies, headers);
    } catch (error) {
      errors[key] = String(error && error.message ? error.message : error);
    }
  }

  await fetchEndpoint('accountProfile', 'https://claude.ai/api/account_profile', orgHeaders);
  await fetchEndpoint('account', 'https://claude.ai/api/account', orgHeaders);

  if (lastActiveOrg) {
    const bootstrapUrl =
      `https://claude.ai/api/bootstrap/${encodeURIComponent(lastActiveOrg)}` +
      '/app_start?statsig_hashing_algorithm=djb2&growthbook_format=sdk&include_system_prompts=false';
    await fetchEndpoint('bootstrapAppStart', bootstrapUrl, orgHeaders);
    const orgBase = `https://claude.ai/api/organizations/${encodeURIComponent(lastActiveOrg)}`;
    const usageHeaders = {
      ...orgHeaders,
      origin: 'https://claude.ai',
      referer: 'https://claude.ai/settings/usage',
    };
    await fetchEndpoint('organizationUsage', `${orgBase}/usage`, usageHeaders);
    await fetchEndpoint('subscriptionDetails', `${orgBase}/subscription_details`, usageHeaders);
    await fetchEndpoint('overageSpendLimit', `${orgBase}/overage_spend_limit`, usageHeaders);
  } else {
    errors.bootstrapAppStart = 'missing lastActiveOrg';
    errors.organizationUsage = 'missing lastActiveOrg';
    errors.subscriptionDetails = 'missing lastActiveOrg';
    errors.overageSpendLimit = 'missing lastActiveOrg';
  }

  if (Object.keys(errors).length > 0) {
    result.errors = errors;
  }
  return result;
}

async function fetchClaudeWebProfile(cookies, webContents) {
  const lastActiveOrg = findCookie(cookies, 'lastActiveOrg')?.value;
  if (webContents && !webContents.isDestroyed() && isClaudeAppUrl(webContents.getURL())) {
    try {
      return await fetchClaudeWebProfileInPage(webContents, lastActiveOrg);
    } catch (error) {
      return fetchClaudeWebProfileWithCookies(
        cookies,
        String(error && error.message ? error.message : error),
      );
    }
  }
  return fetchClaudeWebProfileWithCookies(cookies, null);
}

async function writeStatus(status, extra = {}, webContents = null, options = {}) {
  const cookies = await readClaudeCookies().catch(() => []);
  const sessionKey = ['sessionKey', 'sessionKeyV3', 'sessionKeyV2']
    .map((name) => findCookie(cookies, name)).find(Boolean);
  const lastActiveOrg = findCookie(cookies, 'lastActiveOrg');
  const authenticated = Boolean(sessionKey && lastActiveOrg && isUuid(lastActiveOrg.value));
  const profileKey = authenticated ? `${sessionKey.value}:${lastActiveOrg.value}` : '';
  if (authenticated && !options.skipWebProfile && profileKey !== cachedWebProfileKey) {
    cachedWebProfileKey = profileKey;
    cachedWebProfile = await fetchClaudeWebProfile(cookies, webContents).catch((error) => ({
      version: 1,
      fetchedAt: new Date().toISOString(),
      errors: {
        profile: String(error && error.message ? error.message : error),
      },
    }));
  }
  if (!authenticated) {
    cachedWebProfileKey = '';
    cachedWebProfile = null;
  }
  const webProfile = authenticated ? cachedWebProfile : null;
  const payload = {
    version: 1,
    status,
    authenticated,
    exportedAt: new Date().toISOString(),
    userDataDir,
    cookieNames: cookies.map((cookie) => cookie.name).sort(),
    hasSessionKey: Boolean(sessionKey),
    hasLastActiveOrg: Boolean(lastActiveOrg),
    ...extra,
  };
  writeJsonAtomic(statusFile, payload);
  if (authenticated) {
    writeJsonAtomic(exportFile, {
      version: 1,
      source: 'cockpit-claude-desktop-auth-helper',
      exportedAt: payload.exportedAt,
      userDataDir,
      cookies,
      webProfile,
    });
  }
  return authenticated;
}

app.whenReady().then(async () => {
  if (mode === 'cookie_probe') {
    await importCookiesFromFile(cookieFile);
    await writeStatus('starting', {}, null, { skipWebProfile: true });

    const probeWin = new BrowserWindow({
      width: 960,
      height: 760,
      show: false,
      autoHideMenuBar: true,
      webPreferences: {
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: true,
        webSecurity: true,
      },
    });
    let lastError = null;
    try {
      await loadUrlWithTimeout(probeWin, startUrl, probeTimeoutMs);
      await writeStatus('probe', {
        url: probeWin.webContents.isDestroyed() ? null : probeWin.webContents.getURL(),
      }, probeWin.webContents);
    } catch (error) {
      lastError = String(error && error.message ? error.message : error);
    }
    if (probeWin && !probeWin.isDestroyed()) {
      probeWin.destroy();
    }
    if (lastError) {
      writeJsonAtomic(statusFile, {
        version: 1,
        status: 'probe_error',
        authenticated: false,
        exportedAt: new Date().toISOString(),
        userDataDir,
        error: lastError,
      });
    }
    app.quit();
    return;
  }

  await writeStatus('starting', {}, null, { skipWebProfile: mode === 'probe' });

  if (mode === 'probe') {
    let probeWin = null;
    const startedAt = Date.now();
    let authenticated = false;
    let lastError = null;
    try {
      probeWin = new BrowserWindow({
        width: 960,
        height: 760,
        show: false,
        autoHideMenuBar: true,
        webPreferences: {
          nodeIntegration: false,
          contextIsolation: true,
          sandbox: true,
          webSecurity: true,
        },
      });
      await probeWin.loadURL(startUrl);
    } catch (error) {
      lastError = String(error && error.message ? error.message : error);
    }
    while (Date.now() - startedAt <= Math.max(1000, probeTimeoutMs)) {
      try {
        authenticated = await writeStatus('probe', {
          url: probeWin && !probeWin.webContents.isDestroyed() ? probeWin.webContents.getURL() : null,
        }, probeWin ? probeWin.webContents : null);
        if (authenticated) break;
      } catch (error) {
        lastError = String(error && error.message ? error.message : error);
      }
      await sleep(500);
    }
    if (probeWin && !probeWin.isDestroyed()) {
      probeWin.destroy();
    }
    if (!authenticated && lastError) {
      writeJsonAtomic(statusFile, {
        version: 1,
        status: 'probe_error',
        authenticated: false,
        exportedAt: new Date().toISOString(),
        userDataDir,
        error: lastError,
      });
    }
    app.quit();
    return;
  }

  const win = new BrowserWindow({
    width: 1200,
    height: 900,
    minWidth: 920,
    minHeight: 720,
    title: 'Claude Login',
    show: true,
    autoHideMenuBar: true,
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      sandbox: true,
      webSecurity: true,
    },
  });

  win.once('ready-to-show', () => {
    win.show();
    win.focus();
  });

  win.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https:\/\/([^/]+\.)?(claude\.ai|claude\.com|google\.com|googleusercontent\.com|apple\.com|anthropic\.com)\b/i.test(url)) {
      return {
        action: 'allow',
        overrideBrowserWindowOptions: {
          width: 960,
          height: 760,
          autoHideMenuBar: true,
          webPreferences: {
            nodeIntegration: false,
            contextIsolation: true,
            sandbox: true,
            webSecurity: true,
          },
        },
      };
    }
    shell.openExternal(url).catch(() => {});
    return { action: 'deny' };
  });

  win.on('closed', () => {
    writeStatus('closed').finally(() => app.quit());
  });

  win.webContents.on('did-navigate', (_, url) => {
    writeStatus('running', { url }, win.webContents).catch(() => {});
  });
  win.webContents.on('did-navigate-in-page', (_, url) => {
    writeStatus('running', { url }, win.webContents).catch(() => {});
  });
  win.webContents.on('did-fail-load', (_, errorCode, errorDescription, validatedUrl) => {
    writeStatus('load_error', { errorCode, errorDescription, url: validatedUrl }, win.webContents).catch(() => {});
  });
  win.webContents.on('did-finish-load', () => {
    writeStatus('loaded', { url: win.webContents.getURL() }, win.webContents).catch(() => {});
  });

  await win.loadURL(startUrl);

  const interval = setInterval(async () => {
    const authenticated = await writeStatus('running', {
      url: win.webContents.isDestroyed() ? null : win.webContents.getURL(),
    }, win.webContents).catch(() => false);
    if (authenticated && !win.isDestroyed()) {
      win.setTitle('Claude Login - authenticated');
    }
  }, 1500);

  app.on('before-quit', () => clearInterval(interval));
});

app.on('window-all-closed', () => app.quit());
