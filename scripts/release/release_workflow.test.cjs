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
