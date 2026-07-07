export interface MailVerificationCodePreview {
  code: string;
  snippet: string;
}

const CODE_REGEX = /(^|\D)(\d{6})(?!\d)/;

function decodeHtmlEntities(value: string): string {
  if (typeof document !== "undefined") {
    const textarea = document.createElement("textarea");
    textarea.innerHTML = value;
    return textarea.value;
  }
  return value
    .replace(/&nbsp;/gi, " ")
    .replace(/&amp;/gi, "&")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&quot;/gi, '"')
    .replace(/&#39;/gi, "'");
}

export function extractVisibleMailText(raw: string): string {
  return decodeHtmlEntities(
    raw
      .replace(/<!--[\s\S]*?-->/g, " ")
      .replace(/<script\b[\s\S]*?<\/script>/gi, " ")
      .replace(/<style\b[\s\S]*?<\/style>/gi, " ")
      .replace(/<noscript\b[\s\S]*?<\/noscript>/gi, " ")
      .replace(/<[^>]+>/g, " "),
  )
    .replace(/\r/g, "\n")
    .replace(/[ \t\f\v]+/g, " ")
    .replace(/\n\s+/g, "\n")
    .replace(/\s+\n/g, "\n")
    .trim();
}

export function findFirstMailVerificationCode(
  raw: string,
): MailVerificationCodePreview | null {
  const visibleText = extractVisibleMailText(raw);
  const match = CODE_REGEX.exec(visibleText);
  if (!match || match.index == null) return null;

  const code = match[2];
  const codeIndex = match.index + match[1].length;
  const snippetStart = Math.max(0, codeIndex - 90);
  const snippetEnd = Math.min(visibleText.length, codeIndex + code.length + 140);
  const prefix = snippetStart > 0 ? "..." : "";
  const suffix = snippetEnd < visibleText.length ? "..." : "";
  const snippet = `${prefix}${visibleText
    .slice(snippetStart, snippetEnd)
    .replace(/\s+/g, " ")
    .trim()}${suffix}`;

  return { code, snippet };
}
