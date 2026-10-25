/*
 * Cockpit Tools native service-tier integration.
 * Independently implemented against the desktop client's local module structure.
 * The account, requirements and model serviceTiers data are never fabricated.
 */
(() => {
  "use strict";
  const root = typeof window === "object" ? window : globalThis;
  const LIMIT = 24 * 1024 * 1024;
  const NAME = "[A-Za-z_$][\\w$]*";
  const allowExpression = (host, auth) =>
    `window.__cockpitNativeFast?.allow(${host},${auth})===true`;
  const enabled = () => root.__cockpitEnhancements?.nativeFast === true;
  const quotaModuleEnabled = () => root.__cockpitEnhancements?.quotaCompat === true
    || root.__cockpitEnhancements?.apiService === true;
  const allow = (host, auth) => enabled() && host === "local"
    && (auth === "apiKey" || auth === "apikey"
      // The current renderer can expose no auth method for a functioning API
      // profile. Only the host's verified API-only credentials may fill this
      // missing eligibility signal; never reinterpret another auth method.
      || auth == null && root.__cockpitEnhancements?.apiKeyAuthVerified === true);
  const failure = (source, reason) => ({ ok: false, patchedSource: source, diagnostics: { reason } });

  // Each gate is checked together with its surrounding call contract. No
  // replacement is applied unless all four known contracts occur exactly once.
  function transform(source) {
    if (!enabled()) return failure(source, "disabled");
    if (typeof source !== "string" || source.length > LIMIT) return failure(source, "source-limit");
    const edits = [];
    const count = { ui: 0, request: 0, queued: 0, explicit: 0 };
    let queuedPolicy = null;
    const matches = (pattern) => source.matchAll(new RegExp(pattern, "g"));
    const enclosing = (at) => {
      const start = source.lastIndexOf("function ", at);
      return start >= 0 && at - start < 6000 ? source.slice(start, at) : "";
    };
    const add = (kind, start, before, after) => {
      count[kind]++;
      edits.push({ start, end: start + before.length, text: after });
    };

    for (const match of matches(`(?<identity>${NAME})\\?\\.authMethod===\`chatgpt\`\\|\\|\\k<identity>\\?\\.authMethod===\`personalAccessToken\`(?:\\|\\|\\k<identity>\\?\\.authMethod===\`amazonBedrock\`)?`)) {
      const before = enclosing(match.index);
      const after = source.slice(match.index + match[0].length, match.index + match[0].length + 1100);
      const host = before.match(new RegExp(`(?<host>${NAME})=(?<argument>${NAME})\\?\\.hostId\\?\\?${NAME},${match.groups.identity}=${NAME}\\(\\k<host>\\),${NAME}=$`));
      if (!host || !after.startsWith(",") || !/isPending:/.test(after) || !/serviceTierAccess:/.test(after)
        || !after.includes(`!!${match.groups.identity}?.isLoading`)) continue;
      add("ui", match.index, match[0], `(${match[0]}||${allowExpression(host.groups.host, `${match.groups.identity}?.authMethod`)})`);
    }

    for (const match of matches(`(?<auth>${NAME})!==\`chatgpt\`&&\\k<auth>!==\`personalAccessToken\`(?:&&\\k<auth>!==\`amazonBedrock\`)?`)) {
      const before = enclosing(match.index);
      const signature = before.match(new RegExp(`^function ${NAME}\\((?<scope>${NAME}),(?<host>${NAME})\\)\\{let ${match.groups.auth}=await ${NAME}\\(\\k<scope>,\\k<host>\\);if\\($`));
      const after = source.slice(match.index + match[0].length, match.index + match[0].length + 500);
      if (!signature || !after.startsWith(")return null;let ")
        || !after.includes(`${signature.groups.scope}.query.setData(`)
        || !after.includes(`{authMethod:${match.groups.auth},hostId:${signature.groups.host}}`)) continue;
      add("request", match.index, match[0], `(${match[0]})&&!(${allowExpression(signature.groups.host, match.groups.auth)})`);
    }

    for (const match of matches(`\\(await (?<manager>${NAME})\\.getAccount\\(\\{priority:\`critical\`\\}\\)\\)\\.account\\?\\.type===\`chatgpt\``)) {
      const before = enclosing(match.index);
      const signature = before.match(new RegExp(`^function ${NAME}\\(${match.groups.manager},\\{runtime:(?<runtime>${NAME}),storage:(?<storage>${NAME})\\},${NAME},${NAME}\\)\\{`));
      const after = source.slice(match.index + match[0].length, match.index + match[0].length + 50);
      if (!signature || !before.includes("async readServiceTier(") || !after.startsWith(")},async writeServiceTier(")) continue;
      const { manager } = match.groups;
      const { runtime, storage } = signature.groups;
      if (!before.endsWith(`${manager}.getHostId()===\`local\`&&await ${storage}.readGlobalState(\`use-copilot-auth-if-available\`)===!0&&await ${runtime}.isCopilotApiAvailable()?!1:`)) continue;
      const account = match[0].slice(0, -"===`chatgpt`".length);
      add("queued", match.index, match[0], `((__cockpitType)=>__cockpitType===\`chatgpt\`||${allowExpression(`${manager}.getHostId()`, "__cockpitType")})(${account})`);
      queuedPolicy = "legacy";
    }

    // 26.1021 keeps Copilot selection inside the runtime and reads the account
    // into a local variable. Match the complete read contract, not arbitrary ORs.
    for (const match of matches(`(?<account>${NAME})\\.account\\?\\.type===\`chatgpt\`\\|\\|\\k<account>\\.account\\?\\.type===\`amazonBedrock\``)) {
      const before = enclosing(match.index);
      const signature = before.match(new RegExp(
        `^function ${NAME}\\((?<manager>${NAME}),\\{runtime:(?<runtime>${NAME})\\},(?<beforeRead>${NAME}),${NAME}\\)\\{let (?<pending>${NAME})=null;`
        + `return\\{async readServiceTier\\((?<model>${NAME})\\)\\{for\\(;\\k<pending>!=null;\\)await \\k<pending>\\.catch\\(\\(\\)=>void 0\\);`
        + `return \\k<beforeRead>\\(\\),${NAME}\\(\\k<manager>\\.requestClient,\\k<manager>\\.logger,\\k<model>,async\\(\\)=>\\{`
        + `if\\(\\k<manager>\\.getHostId\\(\\)===\`local\`&&await \\k<runtime>\\.isUsingCopilotInference\\(\\)\\)return!1;`
        + `let ${match.groups.account}=await \\k<manager>\\.getAccount\\(\\{priority:\`critical\`\\}\\);return $`,
      ));
      const after = source.slice(match.index + match[0].length, match.index + match[0].length + 50);
      if (!signature || !after.startsWith("})},async writeServiceTier(")) continue;
      const { manager, runtime } = signature.groups;
      if (new Set([manager, runtime, match.groups.account]).size !== 3) continue;
      add("queued", match.index, match[0], `(${match[0]}||${allowExpression(`${manager}.getHostId()`, `${match.groups.account}.account?.type`)})`);
      queuedPolicy = "runtime";
    }

    for (const match of matches(`(?<account>${NAME})\\?\\.type===\`chatgpt\`(?:\\|\\|\\k<account>\\?\\.type===\`amazonBedrock\`)?(?=\\?(?<options>${NAME})\\.serviceTier:null)`)) {
      const before = enclosing(match.index);
      const signature = before.match(new RegExp(`^function ${NAME}\\((?<manager>${NAME}),\\{settings:(?<settings>${NAME}),storage:(?<storage>${NAME}),filesystem:${NAME},runtime:(?<runtime>${NAME})\\},${NAME},${NAME},${NAME}\\)\\{`));
      if (!signature || !before.includes("submissionOptions") || !before.includes("Promise.all([")) continue;
      const { manager, runtime, storage } = signature.groups;
      const hostMatch = before.match(new RegExp(`(?<host>${NAME})=${match.groups.options}\\?\\.executionHostId\\?\\?${manager}\\.getHostId\\(\\);`));
      if (!hostMatch || !before.endsWith(`let{account:${match.groups.account}}=await ${manager}.getAccount({priority:\`critical\`});return `)) continue;
      if (new Set([manager, runtime, storage, hostMatch.groups.host, match.groups.account]).size !== 5) continue;
      const copilot = queuedPolicy === "runtime"
        ? `await ${runtime}.isUsingCopilotInference()`
        : `await ${storage}.readGlobalState(\`use-copilot-auth-if-available\`)===!0&&await ${runtime}.isCopilotApiAvailable()`;
      add("explicit", match.index, match[0], `(${match[0]}||${hostMatch.groups.host}===\`local\`&&${allowExpression(`${manager}.getHostId()`, `${match.groups.account}?.type`)}&&!(${copilot}))`);
    }
    if (Object.values(count).some(value => value !== 1)) return failure(source, "unsupported-gates");
    edits.sort((a, b) => a.start - b.start);
    if (edits.some((edit, i) => i > 0 && edit.start < edits[i - 1].end)) return failure(source, "overlapping-gates");
    let patched = source;
    for (const edit of edits.reverse()) patched = patched.slice(0, edit.start) + edit.text + patched.slice(edit.end);
    return { ok: true, patchedSource: patched, diagnostics: { reason: "prepared", gates: count } };
  }

  function localAsset(value, entry = false) {
    try {
      const url = new URL(value);
      if (!((url.protocol === "app:" && url.hostname === "-")
        || (url.protocol === "file:" && !url.hostname))) return false;
      if (url.search || url.hash || url.username || url.password) return false;
      return new RegExp(`/assets/${entry ? "index" : "app-initial"}-[A-Za-z0-9_-]+\\.js$`).test(url.pathname);
    } catch { return false; }
  }

  function localCompanion(value, initialUrl, adapter = false) {
    try {
      const url = new URL(value), initial = new URL(initialUrl);
      return localAsset(initialUrl) && url.protocol === initial.protocol && url.host === initial.host
        && !url.search && !url.hash && !url.username && !url.password
        && url.pathname.slice(0, url.pathname.lastIndexOf('/')) === initial.pathname.slice(0, initial.pathname.lastIndexOf('/'))
        && (adapter ? /\/assets\/codex-composer-adapter-[A-Za-z0-9_-]+\.js$/ : /\/assets\/app-primary-[A-Za-z0-9_-]+\.js$/).test(url.pathname);
    } catch { return false; }
  }

  const localPrimary = (value, initialUrl) => localCompanion(value, initialUrl);
  const localAdapter = (value, initialUrl) => localCompanion(value, initialUrl, true);
  const completionKinds = modules => modules.length === 2 ? ['quota-primary', 'quota']
    : modules.length === 1 ? ['quota'] : [];

  const sourceLength = value => value.source.length + (value.modules || []).reduce((sum, module) => sum + module.source.length, 0);
  const validQuotaReason = reason => typeof reason === 'string' && /^[a-z][a-z0-9_-]{0,63}$/.test(reason);
  function validModules(source, url, modules, mode = 'full', quotaReason = null) {
    if (!['full', 'fast-only'].includes(mode) || !Array.isArray(modules)) return false;
    // Do not reinstall a staged module produced by the retired dictation bridge
    // or capability suppression. The original client module remains available.
    if (typeof source !== 'string' || source.includes('__cockpitNativeDictation')
      || source.includes('__cockpitApiDictation')) return false;
    if (mode === 'fast-only') {
      return enabled() && quotaModuleEnabled() && validQuotaReason(quotaReason)
        && modules.length === 0 && source.length <= LIMIT && !source.includes('__cockpitQuotaCompat')
        && (source.match(/window\.__cockpitNativeFast\?\.allow\(/g) || []).length === 4;
    }
    // The validated initial module fixes the graph shape; a truncated current
    // graph must never be reinterpreted as the older two-module layout.
    const currentLayout = (source.match(/globalThis\.__cockpitQuotaCompat\.initialLayout="adapter-v1";/g) || []).length;
    if (currentLayout > 1 || modules.length !== (quotaModuleEnabled() ? currentLayout === 1 ? 2 : 1 : 0)) return false;
    const kinds = completionKinds(modules);
    return modules.every((module, index) => module
      && (index === 0 ? localPrimary(module.url, url) : localAdapter(module.url, url))
      && typeof module.source === 'string'
      && module.source.includes(`window.__cockpitNativeFastComplete?.(${JSON.stringify(kinds[index])})`))
      && sourceLength({ source, modules }) <= LIMIT;
  }

  // Streaming lexical scan: no eval, no full AST and no retained token array.
  // Strings, comments, regex bodies and template text cannot become imports.
  function* tokens(source) {
    let position = 0;
    let expression = true;
    let last = "";
    const controls = [];
    const blocks = [];
    const emit = (kind, start, end, text = source.slice(start, end)) => {
      if (kind === "name") expression = /^(return|throw|case|delete|void|typeof|new|in|of|instanceof|yield|await|else|do)$/.test(text);
      else if (kind === "literal") expression = false;
      else if (text === "(") { controls.push(/^(if|while|for|with|switch|catch)$/.test(last)); expression = true; }
      else if (text === ")") expression = controls.pop() === true;
      else if (text === "{") { blocks.push(!["=", "(", "[", ",", ":", "?", "return", "yield"].includes(last)); expression = true; }
      else if (text === "}") expression = blocks.pop() !== false;
      else expression = ![".", "?.", "]", "++", "--"].includes(text);
      last = text;
      return { kind, start, end, text };
    };
    function* code(template = false) {
      let depth = 0;
      while (position < source.length) {
        let start = position;
        let ch = source[position];
        if (/\s/.test(ch)) { position++; continue; }
        if (source.startsWith("//", position)) {
          const end = source.indexOf("\n", position + 2); position = end < 0 ? source.length : end + 1; continue;
        }
        if (source.startsWith("/*", position)) {
          const end = source.indexOf("*/", position + 2);
          if (end < 0) throw new Error("unclosed-comment");
          position = end + 2; continue;
        }
        if (ch === "'" || ch === '"') {
          position++;
          while (position < source.length && source[position] !== ch) {
            if (source[position] === "\\") position++;
            else if (/[\r\n]/.test(source[position])) throw new Error("unclosed-string");
            position++;
          }
          if (position >= source.length) throw new Error("unclosed-string");
          yield emit("literal", start, ++position); continue;
        }
        if (ch === "`") {
          position++;
          let interpolated = false;
          for (;;) {
            if (position >= source.length) throw new Error("unclosed-template");
            if (source[position] === "\\") { position += 2; continue; }
            if (source[position] === "`") {
              position++;
              yield emit(interpolated ? "template" : "literal", start, position);
              expression = false; break;
            }
            if (source.startsWith("${", position)) {
              interpolated = true; position += 2; expression = true;
              yield* code(true);
            } else position++;
          }
          continue;
        }
        if (ch === "/" && expression) {
          position++;
          let bracket = false;
          while (position < source.length) {
            const c = source[position++];
            if (c === "\\") { position++; continue; }
            if (/[\r\n]/.test(c)) throw new Error("unclosed-regex");
            if (c === "[") bracket = true;
            if (c === "]") bracket = false;
            if (c === "/" && !bracket) break;
          }
          while (/[a-z]/i.test(source[position] || "")) position++;
          yield emit("literal", start, position); continue;
        }
        if (/[$A-Za-z_\u0080-\uFFFF]/.test(ch)) {
          while (/[$\w\u0080-\uFFFF]/.test(source[++position] || "")) { /* identifier */ }
          yield emit("name", start, position); continue;
        }
        if (/\d/.test(ch)) {
          while (/[\w.]/.test(source[++position] || "")) { /* number */ }
          yield emit("literal", start, position); continue;
        }
        if (template && ch === "}" && depth === 0) { position++; return; }
        if (ch === "{") depth++;
        if (ch === "}") depth--;
        const pair = source.slice(position, position + 2);
        position += ["?.", "++", "--", "=>", "&&", "||", "??", "==", "!=", "<=", ">=", "**"].includes(pair) ? 2 : 1;
        yield emit("punctuation", start, position);
      }
      if (template) throw new Error("unclosed-template-expression");
    }
    yield* code();
  }

  // Locate a complete named component, including nested function declarations.
  // Reuse the import scanner so braces in strings/regex/templates are ignored.
  function functionRange(source, start) {
    if (!Number.isInteger(start) || start < 0
      || !new RegExp(`^function ${NAME}\\(${NAME}\\)\\{`).test(source.slice(start, start + 200))) return null;
    let depth = 0, opened = false, returnAt = -1;
    try {
      for (const token of tokens(source.slice(start, start + 256 * 1024))) {
        if (token.kind === 'punctuation' && token.text === '{') { depth++; opened = true; }
        else if (token.kind === 'punctuation' && token.text === '}' && --depth === 0 && opened) {
          return returnAt < 0 ? null : { start, end: start + token.end, returnAt: start + returnAt };
        } else if (depth === 1 && token.kind === 'name' && token.text === 'return') returnAt = token.start;
      }
    } catch { /* Unsupported lexical syntax must leave the native source intact. */ }
    return null;
  }

  async function rewriteModule(source, url) {
    const iterator = tokens(source);
    const buffered = [];
    const next = () => buffered.length ? buffered.shift() : iterator.next().value;
    const peek = () => { if (!buffered.length) { const token = iterator.next().value; if (token) buffered.push(token); } return buffered[0]; };
    const edits = [];
    const rewrite = token => {
      if (token?.kind !== "literal" || !/^["'`]/.test(token.text)) throw new Error("computed-import");
      const path = token.text.slice(1, -1);
      if (/\\|[\r\n]/.test(path)) throw new Error("escaped-import");
      if (!path.startsWith(".")) return;
      const resolved = new URL(path, url);
      if (!(resolved.protocol === "app:" && resolved.host === "-" || resolved.protocol === "file:" && !resolved.host)) throw new Error("external-module");
      edits.push({ start: token.start, end: token.end, text: JSON.stringify(resolved.href) });
    };
    let token, previous = null, checkpoint = 0;
    while ((token = next())) {
      if (token.end - checkpoint > 256 * 1024) {
        checkpoint = token.end;
        await new Promise(resolve => (root.setTimeout || setTimeout)(resolve, 0));
      }
      if (token.kind !== "name" || [".", "?."].includes(previous?.text)) { previous = token; continue; }
      if (peek()?.text === ":") { previous = token; continue; }
      if (token.text === "import") {
        if (peek()?.text === ".") {
          next();
          const meta = next(), dot = next(), property = next();
          if (meta?.text !== "meta" || dot?.text !== "." || property?.text !== "url") throw new Error("unknown-import-meta");
          // import.meta.url is the original module URL, never its temporary blob.
          edits.push({ start: token.start, end: property.end, text: JSON.stringify(url) });
        } else if (peek()?.text === "(") {
          next();
          const argument = next();
          // A class/object method called import is not a dynamic import.
          if (argument?.kind !== "literal") throw new Error("computed-import");
          if (![",", ")"].includes(peek()?.text)) throw new Error("computed-import");
          rewrite(argument);
        } else {
          let item = next();
          if (item?.kind !== "literal") {
            while (item && !["from", ";"].includes(item.text)) item = next();
            item = item?.text === "from" ? next() : null;
          }
          rewrite(item);
        }
      } else if (token.text === "export" && ["*", "{"].includes(peek()?.text)) {
        let item = next();
        if (item.text === "{") {
          while (item && item.text !== "}") item = next();
          if (peek()?.text !== "from") { previous = token; continue; }
          next();
        } else {
          while (item && !["from", ";"].includes(item.text)) item = next();
          if (item?.text !== "from") throw new Error("unknown-export");
        }
        rewrite(next());
      }
      previous = token;
    }
    let end = 0;
    const parts = [];
    for (const edit of edits) {
      if (edit.start < end) throw new Error("overlapping-imports");
      parts.push(source.slice(end, edit.start), edit.text); end = edit.end;
    }
    parts.push(source.slice(end));
    return parts.join("");
  }

  // The startup preload runs as a classic document-start script. It must finish
  // before the parser reaches the app entry, so it cannot yield to the event
  // loop like the background preparation path does.
  function rewriteModuleSync(source, url) {
    const iterator = tokens(source);
    const buffered = [];
    const next = () => buffered.length ? buffered.shift() : iterator.next().value;
    const peek = () => { if (!buffered.length) { const token = iterator.next().value; if (token) buffered.push(token); } return buffered[0]; };
    const edits = [];
    const rewrite = token => {
      if (token?.kind !== "literal" || !/^["'`]/.test(token.text)) throw new Error("computed-import");
      const target = token.text.slice(1, -1);
      if (/\\|[\r\n]/.test(target)) throw new Error("escaped-import");
      if (!target.startsWith(".")) return;
      const resolved = new URL(target, url);
      if (!(resolved.protocol === "app:" && resolved.host === "-" || resolved.protocol === "file:" && !resolved.host)) throw new Error("external-module");
      edits.push({ start: token.start, end: token.end, text: JSON.stringify(resolved.href) });
    };
    let token, previous = null;
    while ((token = next())) {
      if (token.kind !== "name" || [".", "?."].includes(previous?.text)) { previous = token; continue; }
      if (peek()?.text === ":") { previous = token; continue; }
      if (token.text === "import") {
        if (peek()?.text === ".") {
          next();
          const meta = next(), dot = next(), property = next();
          if (meta?.text !== "meta" || dot?.text !== "." || property?.text !== "url") throw new Error("unknown-import-meta");
          edits.push({ start: token.start, end: property.end, text: JSON.stringify(url) });
        } else if (peek()?.text === "(") {
          next();
          const argument = next();
          if (argument?.kind !== "literal") throw new Error("computed-import");
          if (![",", ")"].includes(peek()?.text)) throw new Error("computed-import");
          rewrite(argument);
        } else {
          let item = next();
          if (item?.kind !== "literal") {
            while (item && !["from", ";"].includes(item.text)) item = next();
            item = item?.text === "from" ? next() : null;
          }
          rewrite(item);
        }
      } else if (token.text === "export" && ["*", "{"].includes(peek()?.text)) {
        let item = next();
        if (item.text === "{") {
          while (item && item.text !== "}") item = next();
          if (peek()?.text !== "from") { previous = token; continue; }
          next();
        } else {
          while (item && !["from", ";"].includes(item.text)) item = next();
          if (item?.text !== "from") throw new Error("unknown-export");
        }
        rewrite(next());
      }
      previous = token;
    }
    let end = 0;
    const parts = [];
    for (const edit of edits) {
      if (edit.start < end) throw new Error("overlapping-imports");
      parts.push(source.slice(end, edit.start), edit.text);
      end = edit.end;
    }
    parts.push(source.slice(end));
    return parts.join("");
  }

  // Optional quota compatibility must not disable a fully validated speed
  // integration. Restart from the original module, never a partial quota edit.
  function prepareFastOnly(source, url, quotaReason) {
    if (!enabled() || !quotaModuleEnabled() || !localAsset(url)
      || typeof source !== 'string' || source.length > LIMIT
      || source.includes('__cockpitQuotaCompat') || !validQuotaReason(quotaReason))
      return failure(source, 'fast-only-unavailable');
    const installed = (source.match(/window\.__cockpitNativeFast\?\.allow\(/g) || []).length === 4;
    const fast = installed ? { ok: true, patchedSource: source } : transform(source);
    if (!fast.ok) return fast;
    return { ...fast, url, mode: 'fast-only', quotaReason };
  }

  function prepareLive(source, url) {
    if (!localAsset(url) || typeof source !== "string" || source.length > LIMIT) return failure(source, "unsupported-module");
    let result = { ok: true, patchedSource: source };
    const fastInstalled = (source.match(/window\.__cockpitNativeFast\?\.allow\(/g) || []).length === 4;
    const quotaInstalled = (source.match(/globalThis\.__cockpitQuotaCompat\.initialReady=true/g) || []).length === 1;
    if (enabled() && !fastInstalled) result = transform(result.patchedSource);
    if (result.ok && quotaModuleEnabled() && !quotaInstalled) {
      result = root.__cockpitQuotaCompat?.transform(result.patchedSource, url) || failure(source, "quota-unavailable");
      if (!result.ok && enabled()) return prepareFastOnly(source, url, result.diagnostics?.reason || 'quota-unavailable');
    }
    if (!result.ok) return failure(source, result.diagnostics?.reason || "unsupported-gates");
    if (!enabled() && !quotaModuleEnabled()) return failure(source, "disabled");
    return { ok: true, patchedSource: result.patchedSource, url, mode: 'full' };
  }

  async function prepareModule(source, url) {
    const result = prepareLive(source, url);
    if (!result.ok) return result;
    try {
      const patchedSource = await rewriteModule(result.patchedSource, url);
      return { ...result, patchedSource: `${patchedSource}\n;window.__cockpitNativeFastComplete?.();\n//# sourceURL=${url}\n`, url };
    } catch { return failure(source, "unsupported-module-syntax"); }
  }

  async function prepareDocument() {
    const document = root.document;
    const candidates = new Set();
    const entries = [...document.scripts].map(script => script.src).filter(url => localAsset(url, true));
    if (!entries.length || entries.length > 4) return { ok: false, reason: "unsupported-entry" };
    const read = async url => {
      const response = await root.fetch(url, { signal: AbortSignal.timeout(6000) });
      if (!response.ok) throw new Error("asset-unavailable");
      const source = await response.text();
      if (source.length > LIMIT) throw new Error("source-limit");
      return source;
    };
    try {
      for (const entry of entries) {
        for (const match of (await read(entry)).matchAll(/["'`]\.\/app-initial-[A-Za-z0-9_-]+\.js["'`]/g)) {
          const url = new URL(match[0].slice(1, -1), entry).href;
          if (localAsset(url)) candidates.add(url);
        }
      }
      if (candidates.size !== 1) return { ok: false, reason: "ambiguous-module" };
      const url = [...candidates][0];
      const source = await read(url);
      let result = await prepareModule(source, url);
      if (!result.ok) return { ok: false, reason: result.diagnostics?.reason || "unsupported-module" };
      const modules = [];
      if (quotaModuleEnabled() && result.mode === 'full') {
        try {
          const primaryUrls = new Set([...source.matchAll(/["'`]\.\/app-primary-[A-Za-z0-9_-]+\.js["'`]/g)]
            .map(match => new URL(match[0].slice(1, -1), url).href).filter(value => localPrimary(value, url)));
          if (primaryUrls.size !== 1) throw Error('quota-primary-not-unique');
          const primaryUrl = [...primaryUrls][0];
          const contract = root.__cockpitQuotaCompat.transform(source, url);
          const primary = root.__cockpitQuotaCompat.transformComposer(await read(primaryUrl), primaryUrl, url, contract.diagnostics?.bindings);
          if (!contract.ok || !primary.ok) throw Error(primary.diagnostics?.reason || 'quota-contract-mismatch');
          const currentLayout = contract.diagnostics?.bindings?.layout === 'adapter-v1';
          const prepared = await rewriteModule(primary.patchedSource, primaryUrl);
          const primaryKind = currentLayout ? 'quota-primary' : 'quota';
          modules.push({ url: primaryUrl, source: `${prepared}\n;window.__cockpitNativeFastComplete?.(${JSON.stringify(primaryKind)});\n//# sourceURL=${primaryUrl}\n` });
          if (currentLayout) {
            const adapterUrl = new URL(contract.diagnostics.bindings.adapterPath, url).href;
            if (!localAdapter(adapterUrl, url)) throw Error('quota-adapter-url-invalid');
            const adapter = root.__cockpitQuotaCompat.transformAdapter(
              await read(adapterUrl), adapterUrl, url, primaryUrl, primary.diagnostics?.bindings,
            );
            if (!adapter.ok) throw Error(adapter.diagnostics?.reason || 'quota-adapter-contract-mismatch');
            const preparedAdapter = await rewriteModule(adapter.patchedSource, adapterUrl);
            modules.push({ url: adapterUrl, source: `${preparedAdapter}\n;window.__cockpitNativeFastComplete?.("quota");\n//# sourceURL=${adapterUrl}\n` });
          }
        } catch (error) {
          const quotaReason = validQuotaReason(error.message) ? error.message : 'quota-primary-unavailable';
          const fastOnly = prepareFastOnly(source, url, quotaReason);
          if (!fastOnly.ok) return failure(source, quotaReason);
          const prepared = await rewriteModule(fastOnly.patchedSource, url);
          result = { ...fastOnly, patchedSource: `${prepared}\n;window.__cockpitNativeFastComplete?.();\n//# sourceURL=${url}\n` };
          modules.length = 0;
        }
      }
      if (!validModules(result.patchedSource, url, modules, result.mode, result.quotaReason)) return { ok: false, reason: 'module-source-limit' };
      return { ...result, modules };
    } catch { return { ok: false, reason: "asset-unavailable" }; }
  }

  const HANDOFF_PREFIX = "cockpit-native-handoff-v1:";
  let handoff = null, unloadHandler = null, restoreTimer = null, staged = null, preparationEpoch = 0;
  const configurationMask = config => [config?.nativeFast, config?.quotaCompat, config?.apiKeyAuthVerified, config?.apiService]
    .map(flag => flag === true ? "1" : "0").join("");
  const featureMask = (config, mode = 'full') => mode === 'full' ? configurationMask(config)
    : mode === 'fast-only' && config?.nativeFast === true
      ? `10${config.apiKeyAuthVerified === true ? '1' : '0'}0` : 'invalid';
  const validNonce = nonce => typeof nonce === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(nonce);
  const localPage = () => root.location?.protocol === "app:" && root.location?.hostname === "-";
  const setState = (value, reason) => {
    const config = root.__cockpitEnhancements;
    if (config) { config.nativePatchState = value; config.nativePatchReason = reason; }
  };

  function restoreStagedName() {
    if (restoreTimer != null) root.clearTimeout(restoreTimer);
    restoreTimer = null;
    if (staged && root.name === staged.encoded) root.name = staged.previousName;
    staged = null;
  }

  // A host-initiated reload may snapshot the name before beforeunload, then
  // deliver the old document's late write to the new context. That context has
  // no local staged record; cleanup must still restore only our own handoff.
  function restoreInheritedName(nonce, expectedOwner = root.__cockpitEnhancements?.owner) {
    const raw = String(root.name ?? "");
    if (nonce !== undefined && !validNonce(nonce) || !localPage() || !raw.startsWith(HANDOFF_PREFIX)
      || raw.length > LIMIT * 2 + 131072) return;
    try {
      const value = JSON.parse(raw.slice(HANDOFF_PREFIX.length));
      if (validNonce(value.nonce) && (nonce === undefined || value.nonce === nonce)
        && value.owner === expectedOwner
        && typeof value.previousName === "string" && value.previousName.length <= 65536
        && !value.previousName.startsWith(HANDOFF_PREFIX)) root.name = value.previousName;
    } catch { /* Preserve names that cannot be attributed to this handoff. */ }
  }

  function stageHandoff(nonce) {
    const config = root.__cockpitEnhancements;
    if (!handoff || handoff.nonce !== nonce || !localPage()
      || handoff.owner !== config?.owner || handoff.features !== featureMask(config, handoff.mode)
      || handoff.requestedFeatures !== configurationMask(config))
      return { ok: false, reason: "handoff-unavailable" };
    restoreStagedName();
    restoreInheritedName(nonce);
    const previousName = String(root.name ?? "");
    if (previousName.length > 65536 || previousName.startsWith(HANDOFF_PREFIX))
      return { ok: false, reason: "handoff-name-in-use" };
    const encoded = HANDOFF_PREFIX + JSON.stringify({ ...handoff, previousName, createdAt: Date.now() });
    try {
      root.name = encoded;
      if (root.name !== encoded) { root.name = previousName; return { ok: false, reason: "handoff-storage-failed" }; }
      staged = { encoded, previousName };
      // A canceled navigation must not leave a large payload as the window name.
      restoreTimer = root.setTimeout(restoreStagedName, 2000);
      return { ok: true };
    } catch { return { ok: false, reason: "handoff-storage-failed" }; }
  }

  function cancelHandoff(nonce) {
    if (handoff && nonce !== handoff.nonce) return false;
    preparationEpoch++;
    if (unloadHandler) root.removeEventListener?.("beforeunload", unloadHandler);
    unloadHandler = null;
    restoreStagedName();
    restoreInheritedName(nonce);
    handoff = null;
    return true;
  }

  function retainHandoff(value) {
    cancelHandoff(handoff?.nonce);
    handoff = value;
    unloadHandler = () => { stageHandoff(value.nonce); };
    root.addEventListener?.("beforeunload", unloadHandler);
  }

  async function prepareHandoff(nonce) {
    if (!validNonce(nonce)) return { ok: false, reason: "invalid-handoff-nonce" };
    restoreInheritedName();
    const config = root.__cockpitEnhancements;
    if (handoff?.nonce === nonce && handoff.owner === config?.owner
      && handoff.features === featureMask(config, handoff.mode)
      && handoff.requestedFeatures === configurationMask(config))
      return { ok: true, url: handoff.url, sourceLength: sourceLength(handoff), mode: handoff.mode };
    const epoch = preparationEpoch;
    const result = await prepareDocument();
    if (epoch !== preparationEpoch || config?.owner !== root.__cockpitEnhancements?.owner
      || configurationMask(config) !== configurationMask(root.__cockpitEnhancements))
      return { ok: false, reason: "handoff-canceled" };
    if (!result.ok) return result;
    retainHandoff({ nonce, owner: config?.owner, requestedFeatures: configurationMask(config),
      features: featureMask(config, result.mode), mode: result.mode, quotaReason: result.quotaReason,
      url: result.url, source: result.patchedSource, modules: result.modules });
    return { ok: true, url: result.url, sourceLength: sourceLength(handoff), mode: handoff.mode };
  }

  function installModule(source, url, modules = [], mode = 'full', quotaReason = null) {
    const doc = root.document;
    if (root.__cockpitNativeInstallStarted) return { ok: false, reason: 'installation-already-started' };
    if (!localAsset(url) || !doc || doc.documentElement || !validModules(source, url, modules, mode, quotaReason)) {
      setState("unsupported", "document-start-required");
      return { ok: false, reason: "document-start-required" };
    }
    root.__cockpitNativeInstallStarted = true;
    let container;
    const blobs = [], preloads = [];
    try {
      if (root.HTMLScriptElement?.supports?.("importmap") !== true) throw Error("unsupported-importmap");
      const imports = {};
      for (const module of [{ source, url }, ...modules]) {
        const preload = doc.createElement("link");
        if (preload.relList?.supports?.("modulepreload") !== true) throw Error("unsupported-modulepreload");
        const blob = root.URL.createObjectURL(new root.Blob([module.source], { type: "text/javascript" }));
        blobs.push(blob); imports[module.url] = blob;
        preload.rel = "modulepreload"; preload.href = blob; preloads.push(preload);
      }
      const map = doc.createElement("script");
      map.type = "importmap"; map.textContent = JSON.stringify({ imports });
      const owner = root.__cockpitEnhancements?.owner, features = featureMask(root.__cockpitEnhancements, mode);
      const requestedFeatures = configurationMask(root.__cockpitEnhancements);
      const evaluated = new Set();
      let completed = false, timer;
      const detach = () => {
        if (timer) root.clearTimeout(timer);
        doc.removeEventListener("DOMContentLoaded", watch);
        preloads.forEach(preload => { preload.onerror = null; });
        blobs.forEach(blob => root.URL.revokeObjectURL(blob));
      };
      const fail = () => {
        if (completed) return;
        completed = true; detach(); cancelHandoff(handoff?.nonce);
        setState("unsupported", "module-preload-failed");
      };
      const watch = () => { if (!completed) timer = root.setTimeout(fail, 30000); };
      root.__cockpitNativeFastComplete = (kind = 'initial') => {
        const config = root.__cockpitEnhancements;
        if (completed || !config || config.owner !== owner || featureMask(config, mode) !== features || configurationMask(config) !== requestedFeatures) return;
        if (kind !== 'initial' && !completionKinds(modules).includes(kind)) return;
        evaluated.add(kind);
        if (evaluated.size !== 1 + modules.length) return;
        completed = true; detach();
        config.nativePatchMode = mode;
        config.nativePatchFeatures = features;
        config.nativePatchRequestedFeatures = requestedFeatures;
        config.nativeQuotaPatchState = mode === 'fast-only' ? 'unsupported' : quotaModuleEnabled() ? 'active' : 'disabled';
        config.nativeQuotaPatchReason = mode === 'fast-only' ? quotaReason : undefined;
        setState('active', mode === 'fast-only' ? 'fast-only-quota-unsupported' : 'module-evaluated');
      };
      preloads.forEach(preload => { preload.onerror = fail; });
      container = doc.createElement("html"); doc.appendChild(container);
      container.appendChild(map); preloads.forEach(preload => container.appendChild(preload));
      setState("preloading", "module-preloaded");
      doc.addEventListener("DOMContentLoaded", watch, { once: true });
      root.__cockpitNativeFastDispose = () => {
        completed = true; detach(); cancelHandoff(handoff?.nonce);
        delete root.__cockpitNativeFastComplete;
      };
      return { ok: true };
    } catch {
      blobs.forEach(blob => root.URL.revokeObjectURL(blob));
      cancelHandoff(handoff?.nonce); setState("unsupported", "module-preload-failed");
      return { ok: false, reason: "module-preload-failed" };
    } finally {
      if (container?.parentNode === doc) doc.removeChild(container);
    }
  }

  function consumeHandoff(nonce) {
    const fail = reason => { setState("unsupported", reason); return { ok: false, reason }; };
    if (!validNonce(nonce) || !localPage()) return fail("invalid-handoff-nonce");
    const raw = String(root.name ?? "");
    if (!raw.startsWith(HANDOFF_PREFIX) || raw.length > LIMIT * 2 + 131072)
      return fail("handoff-unavailable");
    let value;
    try { value = JSON.parse(raw.slice(HANDOFF_PREFIX.length)); }
    catch { return fail("handoff-invalid"); }
    if (value?.nonce !== nonce) return fail("handoff-unavailable");
    if (typeof value.previousName !== "string" || value.previousName.length > 65536)
      return fail("handoff-invalid");
    root.name = value.previousName;
    const config = root.__cockpitEnhancements, age = Date.now() - value.createdAt;
    if (value.owner !== config?.owner || value.features !== featureMask(config, value.mode)
      || value.requestedFeatures !== configurationMask(config)
      || !Number.isFinite(age) || age < 0 || age > 30000 || !localAsset(value.url)
      || !validModules(value.source, value.url, value.modules || [], value.mode, value.quotaReason)
      || !value.source.includes("window.__cockpitNativeFastComplete?.()")) return fail("handoff-invalid");
    retainHandoff({ nonce, owner: value.owner, features: value.features, requestedFeatures: value.requestedFeatures,
      mode: value.mode, quotaReason: value.quotaReason, url: value.url, source: value.source, modules: value.modules });
    return installModule(value.source, value.url, value.modules || [], value.mode, value.quotaReason);
  }

  function reloadPreparedDocument(nonce) {
    const guard = root.__cockpitStartupGuard;
    if (!guard) return { ok: false, reason: "startup-guard-unavailable" };
    const reason = guard.inspect();
    if (reason) return { ok: false, reason };
    const stage = stageHandoff(nonce);
    if (!stage.ok) return stage;
    const result = guard.reload();
    if (!result.ok) restoreStagedName();
    return result;
  }

  const api = { discardHandoff: restoreInheritedName, allow, transform, localAsset, localAdapter, functionRange, rewriteModule, rewriteModuleSync, prepareFastOnly, prepareLive, prepareModule, prepareDocument, installModule, prepareHandoff, consumeHandoff, cancelHandoff, reloadPreparedDocument,
    inspectStartupReload: () => {
      const guard = root.__cockpitStartupGuard;
      if (!guard) return { ok: false, reason: "startup-guard-unavailable" };
      const reason = guard.inspect();
      return reason ? { ok: false, reason, ...(reason === 'draft-present' ? { draft: guard.snapshotDraft?.() } : {}) }
        : { ok: true, reason: "clean-startup" };
    } };
  root.__cockpitNativeFast = api;
  if (typeof module === "object" && module.exports) module.exports = api;
})();
