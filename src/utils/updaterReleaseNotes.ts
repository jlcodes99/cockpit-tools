const ZH_SECTION_HEADER = '## 更新日志（中文）';
const EN_SECTION_HEADER = '## Changelog (English)';
const GITHUB_RELEASE_TAG_BASE_URL =
  'https://github.com/jlcodes99/cockpit-tools/releases/tag/v';
const RELEASE_HIGHLIGHTS: Record<string, { zh: string; en: string }> = {
  '1.3.1': {
    zh: `### 重要更新

- **Codex API 生图兼容恢复**：修复第三方 API 服务与 API Key provider 无法使用 Codex 内置生图的问题；提供 \`gpt-image-2\` 的供应商及多开实例现在会自动写入所需配置。
- **Codex SSH 账号同步**：支持主机管理、连接测试、同步 \`auth.json\` / \`config.toml\`、远端哈希校验、切号后自动同步，并在可能时重载远端 Codex app-server/daemon。
- **Codex 切号支持同步 Hermes 鉴权**：开启后，OAuth 账号切号会同步写入 \`~/.hermes/auth.json\`；API Key 账号自动跳过，同步失败不会阻断切号。`,
    en: `### Highlights

- **Codex API image generation compatibility restored**: third-party API Service and API Key providers can use built-in Codex image generation again; providers exposing \`gpt-image-2\` and managed instances now receive the required configuration.
- **Codex account sync over SSH**: manage hosts, test connections, sync \`auth.json\` / \`config.toml\`, verify remote hashes, sync after account switches, and reload the remote Codex app-server/daemon when possible.
- **Optional Hermes auth sync on Codex switch**: OAuth account switches can update \`~/.hermes/auth.json\`; API Key accounts are skipped and sync failures do not block switching.`,
  },
};

export interface ParsedUpdaterReleaseNotes {
  releaseNotes: string;
  releaseNotesZh: string;
}

function normalizeNotes(notes?: string): string {
  if (!notes) {
    return '';
  }
  return notes.replace(/\r\n/g, '\n').trim();
}

function normalizeVersion(version: string): string {
  return version.trim().replace(/^v/i, '');
}

function getUpdaterReleaseHighlights(version: string, language: string): string {
  const highlights = RELEASE_HIGHLIGHTS[normalizeVersion(version)];
  if (!highlights) {
    return '';
  }
  return language.toLowerCase().startsWith('zh') ? highlights.zh : highlights.en;
}

export function getUpdaterReleaseHighlightLines(
  version: string,
  language: string,
): string[] {
  const highlights = getUpdaterReleaseHighlights(version, language);
  return highlights
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.startsWith('- '))
    .map((line) => line.slice(2));
}

export function parseUpdaterReleaseNotes(notes?: string): ParsedUpdaterReleaseNotes {
  const normalized = normalizeNotes(notes);
  if (!normalized) {
    return {
      releaseNotes: '',
      releaseNotesZh: '',
    };
  }

  const zhIndex = normalized.indexOf(ZH_SECTION_HEADER);
  const enIndex = normalized.indexOf(EN_SECTION_HEADER);

  if (zhIndex >= 0 && enIndex >= 0) {
    if (zhIndex < enIndex) {
      return {
        releaseNotesZh: normalized
          .slice(zhIndex + ZH_SECTION_HEADER.length, enIndex)
          .trim(),
        releaseNotes: normalized.slice(enIndex + EN_SECTION_HEADER.length).trim(),
      };
    }

    return {
      releaseNotes: normalized
        .slice(enIndex + EN_SECTION_HEADER.length, zhIndex)
        .trim(),
      releaseNotesZh: normalized.slice(zhIndex + ZH_SECTION_HEADER.length).trim(),
    };
  }

  // 没有中英文分段时，直接复用同一份说明。
  return {
    releaseNotes: normalized,
    releaseNotesZh: normalized,
  };
}

export function prependUpdaterReleaseHighlights(
  version: string,
  notes: string,
  language: string,
): string {
  const normalizedNotes = normalizeNotes(notes);
  const highlights = getUpdaterReleaseHighlights(version, language);
  if (!highlights) {
    return normalizedNotes;
  }

  const isZh = language.toLowerCase().startsWith('zh');
  const heading = isZh ? '### 重要更新' : '### Highlights';
  if (normalizedNotes.includes(heading)) {
    return normalizedNotes;
  }

  return normalizedNotes ? `${highlights}\n\n${normalizedNotes}` : highlights;
}

function getStringFromRawJson(raw: Record<string, unknown>, key: string): string {
  const value = raw[key];
  if (typeof value !== 'string') {
    return '';
  }
  const trimmed = value.trim();
  return trimmed;
}

export function resolveUpdaterDownloadUrl(
  version: string,
  rawJson?: Record<string, unknown>,
): string {
  const raw = rawJson ?? {};
  const preferredKeys = ['html_url', 'download_url', 'url', 'details_url'];
  for (const key of preferredKeys) {
    const url = getStringFromRawJson(raw, key);
    if (url) {
      return url;
    }
  }

  const safeVersion = version.trim();
  if (!safeVersion) {
    return 'https://github.com/jlcodes99/cockpit-tools/releases/latest';
  }
  return `${GITHUB_RELEASE_TAG_BASE_URL}${encodeURIComponent(safeVersion)}`;
}
