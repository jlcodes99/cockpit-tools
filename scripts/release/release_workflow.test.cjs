const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const workflowPath = path.join(__dirname, "..", "..", ".github", "workflows", "release.yml");
const workflow = fs.readFileSync(workflowPath, "utf8");

function jobBody(name, nextName) {
  const startMarker = `  ${name}:`;
  const start = workflow.indexOf(startMarker);
  assert.notEqual(start, -1, `missing ${name} job`);
  const end = nextName ? workflow.indexOf(`  ${nextName}:`, start + startMarker.length) : workflow.length;
  assert.notEqual(end, -1, `missing ${nextName} job`);
  return workflow.slice(start, end);
}

test("release remains draft until all assets, manifests, and checksums are ready", () => {
  const prepare = jobBody("prepare-release", "build-windows");
  const finalize = jobBody("finalize-legacy-latest", "upload-checksums");
  const checksums = jobBody("upload-checksums", "publish-release");
  const publish = jobBody("publish-release", "update-homebrew-cask");

  assert.match(prepare, /gh release create "\$\{TAG\}" --draft/);
  assert.doesNotMatch(prepare, /--draft=false/);
  assert.doesNotMatch(finalize, /--draft=false/);
  assert.doesNotMatch(checksums, /--draft=false/);
  assert.match(publish, /needs:[\s\S]*finalize-legacy-latest[\s\S]*upload-checksums/);
  assert.match(publish, /gh release edit "\$\{TAG\}" --draft=false --prerelease=false --latest/);
});

test("public updater verification and Homebrew run only after publication", () => {
  const publish = jobBody("publish-release", "update-homebrew-cask");
  const homebrew = jobBody("update-homebrew-cask");

  assert.match(publish, /Verify complete published updater state/);
  assert.match(publish, /verify_published_updater_manifests\.cjs/);
  assert.match(homebrew, /needs:[\s\S]*publish-release/);
});

test("every release upload uses the draft-safe retry helper", () => {
  assert.doesNotMatch(workflow, /gh release upload/);
  assert.equal((workflow.match(/node scripts\/release\/upload_release_assets\.cjs /g) || []).length, 11);
  for (const [job, next] of [
    ["build-windows", "build-macos-aarch64"],
    ["build-macos-aarch64", "build-macos-x86_64"],
    ["build-macos-x86_64", "build-macos-universal"],
    ["build-macos-universal", "build-linux"],
    ["build-linux", "finalize-legacy-latest"],
    ["finalize-legacy-latest", "upload-checksums"],
    ["upload-checksums", "publish-release"],
  ]) {
    const body = jobBody(job, next);
    assert.match(body, /actions\/checkout@v4/);
    assert.match(body, /actions\/setup-node@v4/);
    assert.match(body, /node scripts\/release\/upload_release_assets\.cjs/);
  }
});

test("macOS release builds use the wrapper that cleans temporary DMG mounts", () => {
  for (const [job, next, target] of [
    ["build-macos-aarch64", "build-macos-x86_64", "aarch64-apple-darwin"],
    ["build-macos-x86_64", "build-macos-universal", "x86_64-apple-darwin"],
  ]) {
    assert.ok(jobBody(job, next).includes(`node scripts/tauri.cjs build --ci --target ${target}`));
  }
  assert.match(jobBody("build-macos-universal", "build-linux"), /node scripts\/tauri.cjs bundle --ci --target universal-apple-darwin/);
});

test("Universal consumes this run's verified thin inputs and does not recompile", () => {
  const universal = jobBody("build-macos-universal", "build-linux");
  assert.match(universal, /needs:\s+- prepare-release\s+- build-macos-aarch64\s+- build-macos-x86_64/);
  assert.match(universal, /macos-universal.cjs --mode merge/);
  assert.doesNotMatch(universal, /tauri\.cjs build|npx tauri build|setup-go|host-build-cache/);
  for (const [job, next, name, target] of [
    ["build-macos-aarch64", "build-macos-x86_64", "aarch64", "aarch64-apple-darwin"],
    ["build-macos-x86_64", "build-macos-universal", "x86_64", "x86_64-apple-darwin"],
  ]) {
    const producer = jobBody(job, next);
    assert.match(producer, /macos-universal.cjs --mode stage/);
    assert.ok(producer.includes(`name: universal-inputs-${name}`));
    assert.ok(universal.includes(`name: universal-inputs-${name}`));
    assert.ok(universal.includes(`path: universal-inputs/${target}`));
    assert.match(producer, /if-no-files-found: error/);
  }
});

test("trusted main CI and release jobs share caches, with correct platform separation", () => {
  const ci = fs.readFileSync(path.join(__dirname, "../../.github/workflows/build-matrix.yml"), "utf8");
  const cache = fs.readFileSync(path.join(__dirname, "../../.github/actions/host-build-cache/action.yml"), "utf8");
  assert.match(ci, /push:\s+branches:[\s\S]*?- "main"/);
  assert.match(ci, /uses: \.\/.github\/actions\/host-build-cache\s+with:\s+target: \$\{\{ matrix.cache_target \}\}/);
  for (const target of ["windows", "macos-aarch64", "macos-x86_64", "linux-x86_64", "linux-aarch64"]) {
    assert.ok(ci.includes(`cache_target: "${target}"`));
  }
  assert.match(cache, /shared-key: host-release-v2-\$\{\{ inputs.target \}\}/);
  assert.match(cache, /workspaces: "\. -> target"/);
  assert.match(cache, /cache-workspace-crates: "false"/);
  assert.match(cache, /save-if: .*github.event_name != 'pull_request'.*refs\/heads\/main/);
  assert.match(cache, /if: runner.os != 'macOS'/);
  assert.doesNotMatch(workflow, /Library\/Caches\/tauri|uses: swatinem\/rust-cache/);
});

// Execute the actual workflow shell steps with fake external commands. This
// catches early exits that textual ordering checks cannot detect.
function runPublishJob(overrides = {}) {
  const publish = jobBody("publish-release", "update-homebrew-cask");
  const scripts = [...publish.matchAll(/        run: \|\r?\n((?:          .*\r?\n|\r?\n)+)/g)]
    .map((match) => match[1].replace(/^          /gm, "").replace(/\r/g, ""));
  assert.equal(scripts.length, 2, "expected publication and verification shell steps");
  const bash = process.platform === "win32"
    ? path.join(process.env.ProgramFiles, "Git", "bin", "bash.exe")
    : "bash";
  const result = spawnSync(bash, ["--noprofile", "--norc", "-e", "-o", "pipefail", "-c", `
    gh() {
      case "$1 $2" in
        "release view") printf '%s\\n' "$TEST_DRAFT"; return "$TEST_VIEW_STATUS" ;;
        "release edit") echo "EDIT $*"; return "$TEST_EDIT_STATUS" ;;
        *) echo "Unexpected gh command: $*" >&2; return 99 ;;
      esac
    }
    node() { echo "VERIFY $*"; return "$TEST_VERIFY_STATUS"; }
    ${scripts.join("\n")}
  `], {
    encoding: "utf8",
    env: {
      ...process.env,
      VERSION: "1.2.3",
      GITHUB_REPOSITORY: "example/cockpit-tools",
      TEST_DRAFT: "true",
      TEST_VIEW_STATUS: "0",
      TEST_EDIT_STATUS: "0",
      TEST_VERIFY_STATUS: "0",
      ...overrides,
    },
  });
  assert.ifError(result.error);
  return result;
}

test("failed public verification can be retried without republishing the release", () => {
  const first = runPublishJob({ TEST_VERIFY_STATUS: "1" });
  assert.equal(first.status, 1, first.stderr);
  assert.match(first.stdout, /EDIT release edit v1\.2\.3 --draft=false --prerelease=false --latest/);
  assert.match(first.stdout, /VERIFY scripts\/release\/verify_published_updater_manifests\.cjs/);

  const retry = runPublishJob({ TEST_DRAFT: "false" });
  assert.equal(retry.status, 0, retry.stderr);
  assert.doesNotMatch(retry.stdout, /EDIT /);
  assert.match(retry.stdout, /VERIFY scripts\/release\/verify_published_updater_manifests\.cjs --version 1\.2\.3 --repo example\/cockpit-tools/);
  assert.match(retry.stdout, /--legacy/);
});

test("verification failure still fails a retry of an already public release", () => {
  const result = runPublishJob({ TEST_DRAFT: "false", TEST_VERIFY_STATUS: "1" });
  assert.equal(result.status, 1, result.stderr);
  assert.doesNotMatch(result.stdout, /EDIT /);
  assert.match(result.stdout, /VERIFY /);
});

test("release lookup and publication failures stop before public verification", () => {
  for (const overrides of [
    { TEST_VIEW_STATUS: "1" },
    { TEST_EDIT_STATUS: "1" },
    { TEST_DRAFT: "null" },
  ]) {
    const result = runPublishJob(overrides);
    assert.equal(result.status, 1, result.stderr);
    assert.doesNotMatch(result.stdout, /VERIFY /);
  }
});
