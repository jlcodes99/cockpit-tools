#!/usr/bin/env node
/**
 * 弹框尺寸冲突检查（避免依赖 CSS 加载顺序）
 *
 * 背景：全局 `.modal { max-width: 560px }` 与组件 `.xxx-modal { max-width: … }`，
 * 以及组件之间（如 `.codex-add-modal` 480px 与 `.codex-oauth-binding-modal` 700px）
 * 优先级相同（都是单个类），谁在打包后的 CSS 里更靠后就谁生效，新增/删除 CSS 导入
 * 就会让尺寸“自己变了”。
 *
 * 规则：
 * 1. 全局基类已用 `:where(.modal)` 降到零优先级，组件单类规则永远生效；
 * 2. 同一个弹框根元素上，**最多只能有一个未加 `.modal.` 前缀的尺寸声明类**；
 *    需要压过共享类（例如 `.codex-add-modal`）时，把该弹框自己的尺寸规则写成
 *    `.modal.<自己的类>`（优先级 0,2,0，与加载顺序无关）。
 * 3. 弹框正文禁止使用 `overflow: visible` 或 `max-height: none`，避免正文溢出
 *    弹框裁剪区后把标题、关闭按钮或底部操作按钮推出视口。
 *
 * 解析说明：按花括号配对解析 CSS 块，递归进入 `@media` 等 at-rule，
 * 并逐个检查选择器列表（`.a, .b { … }`）里的每个类，避免漏检。
 *
 * 用法：
 *   node scripts/check_modal_sizing.cjs           # 打印潜在冲突（退出码 0）
 *   node scripts/check_modal_sizing.cjs --strict  # 存在冲突时退出码 1（用于 CI）
 */

const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..");
const sizeProps = ["width", "max-width", "min-width", "height", "max-height", "min-height"];
const structuralClasses = new Set([
  "modal",
  "modal-content",
  "modal-overlay",
  "modal-body",
  "modal-header",
  "modal-footer",
  "modal-close",
  "modal-tab",
  "modal-tabs",
  "modal-title",
]);

function walk(dir, extensions, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === "node_modules" || entry.name === "dist" || entry.name.startsWith(".")) {
      continue;
    }
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(full, extensions, out);
    } else if (extensions.some((ext) => entry.name.endsWith(ext))) {
      out.push(full);
    }
  }
  return out;
}

function hasSizeDeclaration(body) {
  return sizeProps.some((prop) =>
    new RegExp(`(^|[;{\\s])${prop}\\s*:`).test(body),
  );
}

function hasUnboundedMaxHeight(body) {
  return /(^|[;{\s])max-height\s*:\s*none/.test(body);
}

function hasVisibleOverflow(body) {
  return /(^|[;{\s])overflow(?:-y)?\s*:\s*visible/.test(body);
}

function isDialogBodySelector(selector) {
  return /(?:^|[\s>+~,.])\.[\w-]*(?:modal|dialog)(?:-body|__body)\b/.test(selector)
    || /(?:^|[\s>+~,.])\.modal-body\b/.test(selector);
}

/** 读取与 `{` 配对的整段声明块内容。 */
function readBlock(text, start) {
  let depth = 1;
  let index = start;
  while (index < text.length) {
    if (text[index] === "{") {
      depth += 1;
    } else if (text[index] === "}") {
      depth -= 1;
      if (depth === 0) {
        return { body: text.slice(start, index), nextIndex: index + 1 };
      }
    }
    index += 1;
  }
  return { body: text.slice(start), nextIndex: text.length };
}

/**
 * 解析出所有样式规则，递归进入 at-rule（`@media` / `@supports` …），
 * 规则携带所属 at-rule 条件，供报告说明尺寸声明是否只在媒体查询内生效。
 */
function parseStyleRules(text, atRule = null, rules = []) {
  let buffer = "";
  let index = 0;
  while (index < text.length) {
    const char = text[index];
    if (char === "{") {
      const header = buffer.trim();
      buffer = "";
      const { body, nextIndex } = readBlock(text, index + 1);
      if (header.startsWith("@")) {
        parseStyleRules(body, header, rules);
      } else if (header) {
        rules.push({ header, body, atRule });
      }
      index = nextIndex;
      continue;
    }
    if (char !== "}") {
      buffer += char;
    }
    index += 1;
  }
  return rules;
}

function collectSizedClasses() {
  const map = new Map();
  for (const file of walk(path.join(repoRoot, "src"), [".css"])) {
    const raw = fs
      .readFileSync(file, "utf8")
      .replace(/\/\*[\s\S]*?\*\//g, "");
    const relativeFile = path.relative(repoRoot, file);
    for (const rule of parseStyleRules(raw)) {
      const body = rule.body;
      if (!hasSizeDeclaration(body)) continue;
      // 选择器列表逐个检查，避免只取最后一行漏掉前面的类。
      for (const part of rule.header.split(",")) {
        const selector = part.trim();
        const simple = /^\.(\w[\w-]*)$/.exec(selector);
        const prefixed = /^\.modal(?:-content)?\.(\w[\w-]*)$/.exec(selector);
        const className = simple ? simple[1] : prefixed ? prefixed[1] : null;
        if (!className) continue;
        const previous = map.get(className) ?? {
          sized: false,
          prefixed: false,
          file: relativeFile,
          mediaOnly: true,
        };
        map.set(className, {
          sized: true,
          prefixed: previous.prefixed || Boolean(prefixed),
          file: relativeFile,
          files: [...new Set([...(previous.files ?? []), relativeFile])],
          unbounded: previous.unbounded || hasUnboundedMaxHeight(body),
          mediaOnly: previous.mediaOnly && Boolean(rule.atRule),
        });
      }
    }
  }
  return map;
}

function collectDialogRoots(sizedClasses) {
  const roots = [];
  for (const file of walk(path.join(repoRoot, "src"), [".tsx"])) {
    const raw = fs.readFileSync(file, "utf8");
    for (const match of raw.matchAll(/className=(?:"([^"]+)"|\{`([^`]+)`\})/g)) {
      const text = match[1] ?? match[2] ?? "";
      if (!/(^|\s|\{)(modal|modal-content)(\s|$|\})/.test(text)) continue;
      const tokens = text.match(/[\w-]+/g) ?? [];
      const sized = [];
      for (const token of tokens) {
        if (structuralClasses.has(token)) continue;
        if (!/(modal|dialog|picker)/.test(token)) continue;
        const entry = sizedClasses.get(token);
        if (!entry) continue;
        sized.push({
          token,
          prefixed: entry.prefixed,
          files: entry.files ?? [],
          unbounded: Boolean(entry.unbounded),
          mediaOnly: Boolean(entry.mediaOnly),
        });
      }
      if (sized.length >= 2) {
        const unprefixed = sized.filter((item) => !item.prefixed);
        const prefixed = sized.filter((item) => item.prefixed);
        // 优先级更高的一档决定最终生效者；同一档内若只剩一个类，或这些类都在同一个
        // CSS 文件里（文件内顺序稳定），就不算冲突。
        const winningTier = prefixed.length > 0 ? prefixed : unprefixed;
        const winningFiles = new Set(winningTier.flatMap((item) => item.files));
        if (winningTier.length >= 2 && winningFiles.size >= 2) {
          roots.push({
            file: path.relative(repoRoot, file),
            classes: sized.map(
              (item) =>
                `${item.token}${item.prefixed ? " (已加前缀)" : ""}${
                  item.mediaOnly ? "（仅媒体查询内声明）" : ""
                }`,
            ),
            reason: "同优先级的尺寸声明跨多个 CSS 文件",
          });
        }
      }
      const unbounded = sized.filter((item) => item.unbounded);
      if (unbounded.length > 0) {
        roots.push({
          file: path.relative(repoRoot, file),
          classes: unbounded.map((item) => `${item.token}（max-height: none）`),
          reason: "高度没有视口约束，弹框可能顶到窗口外",
        });
      }
    }
  }
  const seen = new Set();
  return roots.filter((root) => {
    const key = `${root.file}:${root.reason}:${root.classes.join(",")}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

function collectUnsafeDialogBodies() {
  const risks = [];
  for (const file of walk(path.join(repoRoot, "src"), [".css"])) {
    const raw = fs
      .readFileSync(file, "utf8")
      .replace(/\/\*[\s\S]*?\*\//g, "");
    const relativeFile = path.relative(repoRoot, file);
    for (const rule of parseStyleRules(raw)) {
      const unsafeDeclarations = [];
      if (hasVisibleOverflow(rule.body)) {
        unsafeDeclarations.push("overflow: visible");
      }
      if (hasUnboundedMaxHeight(rule.body)) {
        unsafeDeclarations.push("max-height: none");
      }
      if (unsafeDeclarations.length === 0) continue;

      for (const part of rule.header.split(",")) {
        const selector = part.trim();
        if (!isDialogBodySelector(selector)) continue;
        risks.push({
          file: relativeFile,
          classes: [selector],
          reason: `弹框正文使用 ${unsafeDeclarations.join(" / ")}`,
        });
      }
    }
  }
  return risks;
}

const sizedClasses = collectSizedClasses();
const conflicts = [
  ...collectDialogRoots(sizedClasses),
  ...collectUnsafeDialogBodies(),
];

if (conflicts.length === 0) {
  console.log("✅ 未发现弹框尺寸或正文滚动风险。");
  process.exit(0);
}

console.log(`⚠️  发现 ${conflicts.length} 处弹框尺寸或正文滚动风险：`);
for (const conflict of conflicts) {
  console.log(`   - ${conflict.file}: ${conflict.classes.join(" + ")}（${conflict.reason}）`);
}
console.log(
  "\n修复方式：1) 弹框尺寸规则写成语义更强的 `.modal.<dialog-class>`（或给共享类加 `:not(...)` 排除），" +
    "确保同一个弹框根元素上只有一个未加前缀的尺寸声明类；" +
    "2) 高度用 `calc(100vh - …)` 之类的视口约束替代 `max-height: none`，让内容区滚动、标题与底部按钮始终可达。",
);

if (process.argv.includes("--strict")) {
  process.exit(1);
}
