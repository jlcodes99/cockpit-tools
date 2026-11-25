(used, limit, labels, estimated = false) => {
  if (!Number.isFinite(used) || used < 0 || !Number.isFinite(limit) || limit <= 0) {
    const label = `${labels.context} · ${labels.noData}`;
    return { percent: null, label, amount: null, tooltip: label };
  }

  const percent = `${Math.round(Math.min(1, used / limit) * 100)}%`;
  const label = `${labels.context} ${percent}`;
  const amount = `${estimated ? '≈ ' : ''}${used.toLocaleString()} / ${limit.toLocaleString()}`;
  return { percent, label, amount, tooltip: `${label}\n${amount} ${labels.token}` };
}
