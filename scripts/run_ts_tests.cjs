#!/usr/bin/env node

const { readdirSync, statSync } = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

function collectTests(dir) {
  if (statSync(dir).isFile()) {
    if (!dir.endsWith(".test.ts")) {
      throw new Error(`Expected a .test.ts file or test directory: ${dir}`);
    }
    return [dir];
  }
  const files = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...collectTests(fullPath));
    } else if (entry.isFile() && entry.name.endsWith(".test.ts")) {
      files.push(fullPath);
    }
  }
  return files;
}

// Explicit paths select related tests. With no arguments, retain the full
// suite used by CI and existing callers.
const paths = process.argv.slice(2);
const roots = paths.length > 0 ? paths : ["src", "tests"];
let files;
try {
  files = [...new Set(roots.flatMap((root) => collectTests(path.resolve(root))))].sort();
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
if (files.length === 0) {
  console.error("No TypeScript test files found.");
  process.exit(1);
}

console.log(`Running ${files.length} TypeScript test files.`);
const tsxCli = path.join(__dirname, "..", "node_modules", "tsx", "dist", "cli.mjs");
const result = spawnSync(process.execPath, [tsxCli, "--test", ...files], {
  stdio: "inherit",
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 1);
