export function contextWindowDraftsFromRecord(
  windows?: Record<string, number> | null,
  catalog: string[] = [],
): Record<string, string> {
  const drafts: Record<string, string> = {};
  const keys = catalog.length > 0 ? catalog : Object.keys(windows ?? {});
  for (const model of keys) {
    const trimmed = model.trim();
    if (!trimmed) continue;
    const raw =
      windows?.[trimmed] ??
      Object.entries(windows ?? {}).find(
        ([name]) => name.trim().toLowerCase() === trimmed.toLowerCase(),
      )?.[1];
    if (typeof raw === "number" && Number.isInteger(raw) && raw > 0) {
      drafts[trimmed] = String(raw);
    }
  }
  return drafts;
}

export function parseContextWindowDrafts(
  drafts: Record<string, string>,
  catalog: string[] = [],
):
  | { ok: true; windows: Record<string, number> }
  | { ok: false } {
  const windows: Record<string, number> = {};
  const keys = catalog.length > 0 ? catalog : Object.keys(drafts);
  for (const model of keys) {
    const trimmed = model.trim();
    if (!trimmed) continue;
    const raw =
      drafts[trimmed] ??
      Object.entries(drafts).find(
        ([name]) => name.trim().toLowerCase() === trimmed.toLowerCase(),
      )?.[1] ??
      "";
    const value = raw.trim();
    if (!value) continue;
    const parsed = Number(value);
    if (!Number.isInteger(parsed) || parsed <= 0) {
      return { ok: false };
    }
    windows[trimmed] = parsed;
  }
  return { ok: true, windows };
}

export function lookupContextWindowDraft(
  drafts: Record<string, string>,
  ...keys: Array<string | null | undefined>
): string {
  for (const key of keys) {
    const trimmed = key?.trim() ?? "";
    if (!trimmed) continue;
    if (Object.prototype.hasOwnProperty.call(drafts, trimmed)) {
      return drafts[trimmed] ?? "";
    }
    const matched = Object.entries(drafts).find(
      ([name]) => name.trim().toLowerCase() === trimmed.toLowerCase(),
    );
    if (matched) return matched[1] ?? "";
  }
  return "";
}
