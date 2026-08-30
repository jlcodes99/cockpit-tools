const MONTHLY_CREDITS_KEY = "monthly_credits";

export function placeCodexMonthlyCreditsLast<T extends { key: string }>(
  items: readonly T[],
): T[] {
  const monthlyCredits = items.find((item) => item.key === MONTHLY_CREDITS_KEY);
  if (!monthlyCredits || items[items.length - 1] === monthlyCredits) {
    return [...items];
  }

  return [
    ...items.filter((item) => item !== monthlyCredits),
    monthlyCredits,
  ];
}
