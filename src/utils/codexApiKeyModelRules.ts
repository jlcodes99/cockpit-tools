export function parseModelRuleText(value: string): string[] {
  const seen = new Set<string>();
  return value.split(/[\n,]/).map((rule) => rule.trim()).filter((rule) => {
    const key = rule.toLowerCase();
    if (!key || seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

// Match the allowlist's single wildcard, with literal text on both sides.
function matchesRule(rule: string, model: string): boolean {
  const parts = rule.toLowerCase().split("*");
  const first = parts[0];
  if (parts.length === 1) return model === first;
  const last = parts[parts.length - 1];
  if (model.length < first.length + last.length || !model.startsWith(first) || !model.endsWith(last)) {
    return false;
  }
  const end = model.length - last.length;
  let offset = first.length;
  for (const part of parts.slice(1, -1)) {
    const index = model.indexOf(part, offset);
    if (index < 0 || index + part.length > end) return false;
    offset = index + part.length;
  }
  return true;
}

export function modelsOutsideCodexApiKeyAllowlist(modelIds: string[], rules: string[]): string[] {
  if (!rules.length) return [];
  return modelIds.filter((id) => {
    const model = id.trim().toLowerCase();
    const suffix = model.endsWith(")") ? model.lastIndexOf("(") : -1;
    const base = suffix >= 0 ? model.slice(0, suffix) : model;
    return model && !rules.some((rule) => matchesRule(rule, model) || matchesRule(rule, base));
  });
}
