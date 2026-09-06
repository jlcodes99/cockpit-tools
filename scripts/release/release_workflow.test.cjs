const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
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
