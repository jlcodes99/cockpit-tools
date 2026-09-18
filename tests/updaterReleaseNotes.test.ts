import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  getUpdaterReleaseHighlightLines,
  prependUpdaterReleaseHighlights,
  resolveUpdaterDownloadUrl,
} from "../src/utils/updaterReleaseNotes.ts";

describe("independent updater download links", () => {
  const base = "https://github.com/xiaolong2438/cockpit-tools/releases";

  it("uses the fork for release and latest fallbacks", () => {
    assert.equal(resolveUpdaterDownloadUrl("1.3.52"), `${base}/tag/v1.3.52`);
    assert.equal(resolveUpdaterDownloadUrl(" v1.3.52 "), `${base}/tag/v1.3.52`);
    assert.equal(resolveUpdaterDownloadUrl("  "), `${base}/latest`);
  });

  it("keeps manifest download URLs ahead of the fallback", () => {
    assert.equal(resolveUpdaterDownloadUrl("1.3.52", {
      html_url: `${base}/tag/v1.3.52`,
      url: "https://example.com/setup.exe",
    }), `${base}/tag/v1.3.52`);
    assert.equal(resolveUpdaterDownloadUrl("1.3.52", {
      url: "https://example.com/setup.exe",
    }), "https://example.com/setup.exe");
  });
});

describe("updater release highlights", () => {
  it("prepends the three Chinese highlights for version 1.3.1", () => {
    const notes = prependUpdaterReleaseHighlights(
      "1.3.1",
      "### 其他更新\n\n- 原有更新内容",
      "zh-CN",
    );

    assert.ok(notes.startsWith("### 重要更新"));
    assert.match(notes, /Codex API 生图兼容恢复/);
    assert.match(notes, /Codex SSH 账号同步/);
    assert.match(notes, /Codex 切号支持同步 Hermes 鉴权/);
    assert.ok(notes.endsWith("### 其他更新\n\n- 原有更新内容"));
    assert.doesNotMatch(notes, /感谢|#1404|#1434/);
  });

  it("prepends the three English highlights for a v-prefixed version", () => {
    const notes = prependUpdaterReleaseHighlights(
      "v1.3.1",
      "### Other changes\n\n- Existing release note",
      "en-US",
    );

    assert.ok(notes.startsWith("### Highlights"));
    assert.match(notes, /Codex API image generation compatibility restored/);
    assert.match(notes, /Codex account sync over SSH/);
    assert.match(notes, /Optional Hermes auth sync on Codex switch/);
    assert.ok(notes.endsWith("### Other changes\n\n- Existing release note"));
  });

  it("does not prepend a duplicate highlights section", () => {
    const original = "### 重要更新\n\n- 已存在的重要更新";

    assert.equal(
      prependUpdaterReleaseHighlights("1.3.1", original, "zh-CN"),
      original,
    );
  });

  it("leaves other versions unchanged", () => {
    const original = "### Changed\n\n- Other version";

    assert.equal(
      prependUpdaterReleaseHighlights("1.3.2", original, "en"),
      original,
    );
  });

  it("provides three localized lines for release history", () => {
    const chinese = getUpdaterReleaseHighlightLines("v1.3.1", "zh-CN");
    const english = getUpdaterReleaseHighlightLines("1.3.1", "en-US");

    assert.equal(chinese.length, 3);
    assert.equal(english.length, 3);
    assert.match(chinese[0], /Codex API 生图兼容恢复/);
    assert.match(english[0], /Codex API image generation compatibility restored/);
    assert.deepEqual(
      getUpdaterReleaseHighlightLines("1.3.2", "zh-CN"),
      [],
    );
  });
});
