import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { assertReactSingleton } from "../scripts/react-singleton";

function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "cockpit-react-singleton-"));
  const runtime = (name: string) => {
    const file = path.join(root, name, "node_modules/react/cjs/react.production.js");
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, "module.exports = {};\n");
    return file;
  };
  return { root, runtime, dispose: () => fs.rmSync(root, { recursive: true, force: true }) };
}

function check(ids: string[], excluded: string[] = []) {
  const hook = assertReactSingleton().generateBundle;
  assert.equal(typeof hook, "function");
  (hook as Function).call({
    getModuleIds: () => ids.values(),
    getModuleInfo: (id: string) => ({ isIncluded: !excluded.includes(id) }),
    error: (message: string) => { throw new Error(message); },
  });
}

test("one React core plus CommonJS wrappers is a singleton", () => {
  const files = fixture();
  try {
    const runtime = files.runtime("app");
    assert.doesNotThrow(() => check([
      runtime, runtime, `\0${runtime}?commonjs-proxy`, `\0${runtime}?commonjs-module`,
      path.join(files.root, "missing.js"),
    ]));
  } finally { files.dispose(); }
});

test("two physical React cores fail the build", () => {
  const files = fixture();
  try {
    assert.throws(() => check([files.runtime("app"), files.runtime("dependency")]), /Multiple React runtime/);
  } finally { files.dispose(); }
});

test("distinct module IDs for the same physical React core fail the build", () => {
  const files = fixture();
  try {
    const runtime = files.runtime("app");
    const alias = path.join(files.root, "alias");
    fs.symlinkSync(path.dirname(runtime), alias, process.platform === "win32" ? "junction" : "dir");
    const aliasedRuntime = path.join(alias, path.basename(runtime));
    assert.equal(fs.realpathSync.native(runtime), fs.realpathSync.native(aliasedRuntime));
    assert.throws(() => check([runtime, aliasedRuntime]), /Multiple React runtime/);
  } finally { files.dispose(); }
});

test("unused React copies do not fail the build", () => {
  const files = fixture();
  try {
    const runtime = files.runtime("app");
    const unused = files.runtime("unused");
    assert.doesNotThrow(() => check([runtime, unused], [unused]));
  } finally { files.dispose(); }
});
