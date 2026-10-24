// Apply only to remote suggestions, never to stored catalogs or explicit model IDs.
export function filterMiniMaxModelSuggestions<T extends { id: string }>(baseUrl: string, models: T[]): T[] {
  let host: string;
  try {
    host = new URL(baseUrl).hostname;
  } catch {
    return models;
  }
  const direct = host === 'api.minimax.io' || host === 'api.minimaxi.com';
  const openCode = host === 'opencode.ai';
  const siliconFlow = host === 'api.siliconflow.cn' || host === 'api.siliconflow.com';
  if (!direct && !openCode && !siliconFlow) return models;
  const current = direct ? ['minimax-m3.1-flash', 'minimax-m3']
    : openCode ? ['minimax-m3.1-flash-free', 'minimax-m3']
    // These are retained only if the authenticated upstream actually returns them.
    : ['pro/minimaxai/minimax-m3', 'minimaxai/minimax-m3'];
  return models.filter(model => {
    const id = model.id.toLowerCase();
    return !/^(?:pro\/)?(?:minimaxai\/)?minimax-m\d/.test(id) || current.includes(id);
  }).sort((left, right) => {
    const rank = (id: string) => {
      const index = current.indexOf(id.toLowerCase());
      return index < 0 ? current.length : index;
    };
    return rank(left.id) - rank(right.id);
  });
}
