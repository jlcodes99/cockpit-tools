const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const crypto = require("node:crypto");
const { spawnSync } = require("node:child_process");
const test = require("node:test");

const workflowPath = path.join(__dirname, "..", "..", ".github", "workflows", "release.yml");
const workflow = fs.readFileSync(workflowPath, "utf8");
const repoRoot = path.resolve(__dirname, "../..");
const bash = process.platform === "win32"
  ? path.join(process.env.ProgramFiles, "Git", "bin", "bash.exe")
  : "bash";

function jobBody(name, nextName) {
  const startMarker = `  ${name}:`;
  const start = workflow.indexOf(startMarker);
  assert.notEqual(start, -1, `missing ${name} job`);
  const end = nextName ? workflow.indexOf(`  ${nextName}:`, start + startMarker.length) : workflow.length;
  assert.notEqual(end, -1, `missing ${nextName} job`);
  return workflow.slice(start, end);
}

function shellScripts(job) {
  return [...job.matchAll(/        run: \|\r?\n((?:          .*\r?\n|\r?\n)+)/g)]
    .map((match) => match[1].replace(/^          /gm, "").replace(/\r/g, ""));
}

test("release remains draft until all assets, manifests, and checksums are ready", () => {
  const prepare = jobBody("prepare-release", "build-windows");
  const finalize = jobBody("finalize-legacy-latest", "publish-release");
  const publish = jobBody("publish-release", "update-homebrew-cask");

  assert.match(prepare, /gh release create "\$\{TAG\}" --draft/);
  assert.doesNotMatch(prepare, /--draft=false/);
  assert.doesNotMatch(finalize, /--draft=false/);
  assert.match(publish, /needs:[\s\S]*finalize-legacy-latest/);
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
  for (const [job, next] of [
    ["build-windows", "build-macos-aarch64"],
    ["build-macos-aarch64", "build-macos-x86_64"],
    ["build-macos-x86_64", "build-macos-universal"],
    ["build-macos-universal", "build-linux"],
    ["build-linux", "build-linux-cli"],
    ["build-linux-cli", "finalize-legacy-latest"],
    ["finalize-legacy-latest", "publish-release"],
  ]) {
    const body = jobBody(job, next);
    assert.match(body, /actions\/checkout@v4/);
    assert.match(body, /actions\/setup-node@v4/);
    assert.match(body, /node scripts\/release\/upload_release_assets\.cjs/);
  }
});

test("standalone Linux CLI archives join release checksums and validation", () => {
  const cli = jobBody("build-linux-cli", "finalize-legacy-latest");
  const checksums = jobBody("finalize-legacy-latest", "publish-release");
  const validation = jobBody("validate-release-artifacts");

  assert.match(cli, /arch: x86_64[\s\S]*arch: aarch64/);
  assert.match(cli, /cargo build --locked --release --package cockpit-cli/);
  assert.match(cli, /tar -xzf .*\n.*ldd /);
  assert.match(cli, /env -u DISPLAY -u WAYLAND_DISPLAY/);
  assert.match(cli, /name: linux-cli-/);
  assert.doesNotMatch(cli, /prepare-frontend|tauri build|libwebkit2gtk-dev/);
  assert.match(checksums, /needs:[\s\S]*?- build-linux-cli/);
  assert.match(validation, /needs:[\s\S]*?- build-linux-cli/);
  assert.match(validation, /pattern: linux-cli-\*\n\s+path: validation-workspace\/release-assets/);
  assert.match(validation, /for arch in x86_64 aarch64; do\n\s+test -f .*cockpit-cli_/);
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

test("all macOS release assets pass signing and notarization before staging and upload", () => {
  for (const [job, next] of [
    ["build-macos-aarch64", "build-macos-x86_64"],
    ["build-macos-x86_64", "build-macos-universal"],
    ["build-macos-universal", "build-linux"],
  ]) {
    const body = jobBody(job, next);
    const prepare = body.indexOf('macos-signing.cjs prepare');
    const build = body.search(/node scripts\/tauri.cjs (build|bundle)/);
    const dmg = body.indexOf('inject-dmg-readme.cjs');
    const verify = body.indexOf('macos-signing.cjs finalize');
    const stage = body.indexOf('stage_release_assets.cjs');
    const upload = body.indexOf('upload_release_assets.cjs');
    assert.ok(prepare >= 0 && prepare < build && build < dmg && dmg < verify && verify < stage && stage < upload, job);
    assert.match(body, /if: always\(\)\s+run: node scripts\/release\/macos-signing.cjs cleanup/);
    for (const name of ['APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD', 'APPLE_SIGNING_IDENTITY', 'APPLE_ID', 'APPLE_PASSWORD', 'APPLE_TEAM_ID']) {
      assert.ok(body.includes(`secrets.${name}`), `${job}: missing ${name}`);
    }
  }
  assert.doesNotMatch(jobBody('build-windows', 'build-macos-aarch64'), /APPLE_PASSWORD/);
  assert.doesNotMatch(jobBody('build-linux', 'finalize-legacy-latest'), /APPLE_PASSWORD/);
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
  for (const target of ["windows", "windows-aarch64", "macos-aarch64", "macos-x86_64", "linux-x86_64", "linux-aarch64"]) {
    assert.ok(ci.includes(`cache_target: "${target}"`));
  }
  assert.match(cache, /shared-key: host-release-v2-\$\{\{ inputs.target \}\}/);
  assert.match(cache, /workspaces: "\. -> target"/);
  assert.match(cache, /cache-workspace-crates: "false"/);
  assert.match(cache, /save-if: .*github.event_name != 'pull_request'.*refs\/heads\/main/);
  assert.match(cache, /if: runner.os != 'macOS'/);
  assert.doesNotMatch(workflow, /Library\/Caches\/tauri|uses: swatinem\/rust-cache/);
});

test("finalizes from one download, checksums the freshly generated manifest and stops if a signature is missing", {
  skip: process.platform === "win32",
}, (context) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "cockpit-release-finalize-"));
  context.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  const fixtures = path.join(directory, "draft");
  const metadata = path.join(directory, "release-metadata");
  fs.mkdirSync(fixtures);
  fs.mkdirSync(metadata);
  fs.writeFileSync(path.join(metadata, "release-notes.md"), "Release notes");
  fs.writeFileSync(path.join(metadata, "published-at.txt"), "2026-11-18T02:00:00Z");
  const installers = [
    "Cockpit.Tools_aarch64.app.tar.gz", "Cockpit.Tools_x64.app.tar.gz",
    "Cockpit.Tools_1.2.3_x64_en-US.msi", "Cockpit.Tools_1.2.3_x64-setup.exe",
    "Cockpit.Tools_1.2.3_arm64-setup.exe",
    "Cockpit.Tools_1.2.3_amd64.AppImage", "Cockpit.Tools_1.2.3_aarch64.AppImage",
    "Cockpit.Tools_1.2.3_amd64.deb", "Cockpit.Tools_1.2.3_arm64.deb",
    "Cockpit.Tools-1.2.3-1.x86_64.rpm", "Cockpit.Tools-1.2.3-1.aarch64.rpm",
  ];
  for (const installer of installers) {
    fs.writeFileSync(path.join(fixtures, installer), "installer");
    fs.writeFileSync(path.join(fixtures, installer + ".sig"), "test-signature");
  }
  const cliArchives = ["cockpit-cli_1.2.3_linux_x86_64.tar.gz", "cockpit-cli_1.2.3_linux_aarch64.tar.gz"];
  for (const archive of cliArchives) {
    fs.writeFileSync(path.join(fixtures, archive), archive);
  }
  fs.writeFileSync(path.join(fixtures, "latest.json"), "stale draft manifest");
  fs.writeFileSync(path.join(fixtures, "SHA256SUMS.txt"), "stale checksum");

  const scripts = shellScripts(jobBody("finalize-legacy-latest", "publish-release"));
  const fakeCommands = [
    'gh() { test "$1 $2" = "release download" || return 99; echo "DOWNLOAD"; cp draft/* release-assets/; }',
    'node() { if [[ "$1" == scripts/release/upload_release_assets.cjs ]]; then echo "UPLOAD $*"; else "$NODE_BIN" "$REPO_ROOT/$1" "${@:2}"; fi; }',
  ];
  const run = () => spawnSync(bash, ["--noprofile", "--norc", "-e", "-o", "pipefail", "-c", [...fakeCommands, ...scripts].join("\n")], {
    cwd: directory,
    encoding: "utf8",
    env: { ...process.env, NODE_BIN: process.execPath, REPO_ROOT: repoRoot, VERSION: "1.2.3", GITHUB_REPOSITORY: "example/cockpit-tools" },
  });
  const good = run();
  assert.ifError(good.error);
  assert.equal(good.status, 0, good.stderr);
  assert.equal(good.stdout.match(/^DOWNLOAD$/gm)?.length, 1);
  assert.match(good.stdout, /UPLOAD .* v1\.2\.3 latest\.json SHA256SUMS\.txt/);
  const latest = fs.readFileSync(path.join(directory, "latest.json"));
  const checksum = crypto.createHash("sha256").update(latest).digest("hex");
  const lines = fs.readFileSync(path.join(directory, "SHA256SUMS.txt"), "utf8").trim().split("\n");
  assert.ok(lines.includes(checksum + "  latest.json"));
  for (const archive of cliArchives) {
    assert.ok(lines.includes(crypto.createHash("sha256").update(archive).digest("hex") + "  " + archive));
  }
  assert.equal(lines.length, installers.length * 2 + cliArchives.length + 1);

  fs.rmSync(path.join(directory, "release-assets"), { recursive: true });
  fs.rmSync(path.join(fixtures, installers[0] + ".sig"));
  const missing = run();
  assert.ifError(missing.error);
  assert.notEqual(missing.status, 0);
  assert.match(missing.stderr, /Missing signature file/);
  assert.doesNotMatch(missing.stdout, /UPLOAD/);
});

function runPublishJob(overrides = {}) {
  const scripts = shellScripts(jobBody("publish-release", "update-homebrew-cask"));
  assert.equal(scripts.length, 2, "expected publication and verification shell steps");
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
