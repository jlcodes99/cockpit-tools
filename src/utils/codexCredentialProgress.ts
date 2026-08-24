/** Keep the actionable OAuth failure while stripping wrapper text used by account status UI. */
export function conciseCodexCredentialFailure(value: unknown): string {
  const raw = typeof value === "string" ? value.trim() : "";
  if (!raw) return "";
  const originalMarker = raw.lastIndexOf("原始错误:");
  if (originalMarker >= 0) {
    return raw.slice(originalMarker + "原始错误:".length).trim();
  }
  return raw.replace(/^Error:\s*/, "");
}
