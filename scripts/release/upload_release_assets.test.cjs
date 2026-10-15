const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");
const { uploadReleaseAssets, isRetryableFailure } = require("./upload_release_assets.cjs");

const success = (stdout = "") => ({ status: 0, stdout, stderr: "" });
const failure = (stderr) => ({ status: 1, stdout: "", stderr });

function assets(t, names = ["first.zip", "second.deb", "third.rpm"]) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "cockpit-release-upload-"));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return names.map((name) => {
    const file = path.join(directory, name);
    fs.writeFileSync(file, `complete bytes for ${name}`);
    return file;
  });
}

test("partial success retries only the failed asset and replaces its partial upload", async (t) => {
  const files = assets(t);
  const remote = new Map();
  const uploaded = [];
  const waits = [];
  let draftChecks = 0;
  let failedOnce = false;
  await uploadReleaseAssets("v1.3.66", files, {
    run(args) {
      if (args[1] === "view") {
        draftChecks += 1;
        return success("true\n");
      }
      assert.deepEqual(args.slice(0, 5), ["release", "upload", "v1.3.66", "--clobber", "--"]);
      const asset = args[5];
      uploaded.push(path.basename(asset));
      if (asset === files[1] && !failedOnce) {
        failedOnce = true;
        remote.set(path.basename(asset), "partial bytes");
        return failure("HTTP 500 (https://uploads.github.com/example)");
      }
      remote.set(path.basename(asset), fs.readFileSync(asset, "utf8"));
      return success();
    },
    wait: async (delay) => waits.push(delay),
    log() {},
  });
  assert.deepEqual(uploaded, ["first.zip", "second.deb", "second.deb", "third.rpm"]);
  assert.deepEqual(waits, [2000]);
  assert.equal(draftChecks, 4);
  for (const file of files) assert.equal(remote.get(path.basename(file)), fs.readFileSync(file, "utf8"));
});

test("retry exhaustion remains a failure with bounded exponential backoff", async (t) => {
  const files = assets(t, ["failed.zip"]);
  let uploads = 0;
  const waits = [];
  await assert.rejects(uploadReleaseAssets("v1.3.66", files, {
    run(args) {
      if (args[1] === "view") return success("true");
      uploads += 1;
      return failure("HTTP 503 (https://uploads.github.com/example)");
    },
    wait: async (delay) => waits.push(delay),
    log() {},
  }), /asset upload failed.*\(5\/5\).*HTTP 503/);
  assert.equal(uploads, 5);
  assert.deepEqual(waits, [2000, 4000, 8000, 16000]);
});

test("authentication, permission, missing release and invalid assets fail without retry", async (t) => {
  const files = assets(t, ["asset.zip"]);
  for (const status of [401, 403, 404, 422]) {
    let uploads = 0;
    let waits = 0;
    await assert.rejects(uploadReleaseAssets("v1.3.66", files, {
      run(args) {
        if (args[1] === "view") return success("true");
        uploads += 1;
        return failure(`HTTP ${status}: request rejected (not a transient EOF)`);
      },
      wait: async () => { waits += 1; },
      log() {},
    }), new RegExp(`HTTP ${status}`));
    assert.equal(uploads, 1);
    assert.equal(waits, 0);
  }
});

test("published release is protected before initial uploads and after a failed attempt", async (t) => {
  const files = assets(t, ["asset.zip"]);
  for (const initiallyPublished of [true, false]) {
    let checks = 0;
    let uploads = 0;
    await assert.rejects(uploadReleaseAssets("v1.3.66", files, {
      run(args) {
        if (args[1] === "view") {
          checks += 1;
          return success(initiallyPublished || checks > 1 ? "false" : "true");
        }
        uploads += 1;
        return failure("HTTP 500");
      },
      wait: async () => {},
      log() {},
    }), /refusing to mutate a published release/);
    assert.equal(uploads, initiallyPublished ? 0 : 1);
  }
});

test("transient release lookup is retried before any upload", async (t) => {
  const files = assets(t, ["asset.zip"]);
  const calls = [];
  await uploadReleaseAssets("v1.3.66", files, {
    run(args) {
      calls.push(args[1]);
      if (calls.length === 1) return failure("HTTP 502");
      return success(args[1] === "view" ? "true" : "");
    },
    wait: async () => {},
    log() {},
  });
  assert.deepEqual(calls, ["view", "view", "upload"]);
});

test("unknown draft state and invalid local files cannot reach mutation", async (t) => {
  const files = assets(t, ["asset.zip"]);
  await assert.rejects(uploadReleaseAssets("v1.3.66", files, {
    run(args) {
      assert.equal(args[1], "view");
      return success("null");
    },
    log() {},
  }), /not a draft/);
  await assert.rejects(uploadReleaseAssets("v1.3.66", [files[0] + ".missing"], {
    run() { assert.fail("missing local file must not invoke gh"); },
  }), /ENOENT/);
});

test("CLI failures exit nonzero before any remote operation", () => {
  const result = spawnSync(process.execPath, [path.join(__dirname, "upload_release_assets.cjs")], { encoding: "utf8" });
  assert.ifError(result.error);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Usage:/);
});

test("CLI reports exhausted upload retries with a failing exit code", (t) => {
  const [file] = assets(t, ["asset.zip"]);
  const preload = path.join(path.dirname(file), "fake-gh.cjs");
  fs.writeFileSync(preload, `
    require("node:child_process").spawnSync = (_command, args) =>
      args[1] === "view"
        ? { status: 0, stdout: "true", stderr: "" }
        : { status: 1, stdout: "", stderr: "HTTP 500 (fake upload)" };
    require("node:timers/promises").setTimeout = async () => {};
  `);
  const result = spawnSync(process.execPath, ["--require", preload, path.join(__dirname, "upload_release_assets.cjs"), "v1.3.66", file], { encoding: "utf8" });
  assert.ifError(result.error);
  assert.equal(result.status, 1);
  assert.match(result.stderr, /asset upload failed.*\(5\/5\).*HTTP 500/);
  assert.equal((result.stdout.match(/Retrying in/g) || []).length, 4);
});

test("retry classification includes transport disconnects and timeouts", () => {
  for (const message of ["Post https://uploads.github.com: EOF", "unexpected EOF", "TLS handshake timeout", "connection reset by peer", "HTTP 429", "HTTP 500"]) {
    assert.equal(isRetryableFailure(failure(message)), true, message);
  }
  assert.equal(isRetryableFailure({ status: null, error: { code: "ETIMEDOUT", message: "timed out" } }), true);
  assert.equal(isRetryableFailure({ status: null, error: { code: "ENOENT", message: "gh not found" } }), false);
});
