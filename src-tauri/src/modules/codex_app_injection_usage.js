// Pure rollout metrics and composer identity helpers shared by the injected footer.
(() => {
  const ownSelector = '[data-cockpit-quota-footer], [data-cockpit-quota-details], [data-cockpit-deepseek-balance], [data-cockpit-deepseek-balance-details], [data-cockpit-usage-statusline]';
  const editorSelector = '.ProseMirror[contenteditable="true"], textarea:not(:disabled):not([readonly]), [role="textbox"][contenteditable="true"]';
  const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  const nonnegative = (value) => typeof value === 'number' && Number.isFinite(value) && value >= 0;
  const turn = (event) => typeof event.turn_id === 'string' && event.turn_id ? event.turn_id : null;

  function percent(numerator, denominator) {
    if (!nonnegative(numerator) || !nonnegative(denominator) || denominator === 0) return null;
    return Math.min(100, Math.max(0, numerator / denominator * 100));
  }

  function timestamp(value) {
    if (typeof value === 'number') return Number.isFinite(value) ? value : NaN;
    return typeof value === 'string' ? Date.parse(value) : NaN;
  }

  // Rollout tails can contain several turns. Never reuse a previous turn's speed
  // or cache hit rate, including the period before the new turn writes tokens.
  function latestSegment(events, currentTurnId) {
    const candidates = events.filter((event) => event && typeof event === 'object'
      && (currentTurnId == null || turn(event) === currentTurnId));
    let start = 0;
    let previousTurn = null;
    let previousOutput = null;
    candidates.forEach((event, index) => {
      const currentTurn = turn(event);
      const currentOutput = nonnegative(event.turn_output_tokens) ? event.turn_output_tokens : null;
      if ((currentTurn && previousTurn && currentTurn !== previousTurn)
        || (currentOutput !== null && previousOutput !== null && currentOutput < previousOutput)) {
        start = index;
        previousOutput = null;
      }
      if (currentTurn) previousTurn = currentTurn;
      if (currentOutput !== null) previousOutput = currentOutput;
    });
    return candidates.slice(start);
  }

  function snapshot(events, now = Date.now(), currentTurnId = null) {
    if (!Array.isArray(events)) return null;
    const segment = latestSegment(events, currentTurnId);
    if (!segment.length) return null;
    const rates = [];
    let input = 0;
    let cached = 0;
    segment.forEach((event, index) => {
      // Missing or invalid cache counters are unknown, not zero cache hits.
      if (nonnegative(event.input_tokens) && event.input_tokens > 0
        && nonnegative(event.cached_input_tokens)) {
        input += event.input_tokens;
        cached += Math.min(event.input_tokens, event.cached_input_tokens);
      }
      if (!index) return;
      const previous = segment[index - 1];
      if (!nonnegative(previous.turn_output_tokens) || !nonnegative(event.turn_output_tokens)) return;
      const output = event.turn_output_tokens - previous.turn_output_tokens;
      const seconds = (timestamp(event.t) - timestamp(previous.t)) / 1000;
      if (output > 0 && seconds >= 0.5 && seconds <= 120) {
        const rate = output / seconds;
        if (Number.isFinite(rate)) rates.push(rate);
      }
    });
    const recent = rates.slice(-3);
    const observedAt = timestamp(segment[segment.length - 1].t);
    return {
      speed: recent.length ? recent.reduce((sum, value) => sum + value / recent.length, 0) : null,
      cache: percent(cached, input),
      observedAt: Number.isFinite(observedAt) ? observedAt : null,
      stale: Number.isFinite(observedAt) && now - observedAt > 120000,
    };
  }

  function threadId(card) {
    const root = card?.closest('[data-codex-composer-root]');
    if (!root) return '';
    const fromComposer = root.querySelector('[data-above-composer-conversation-id]')
      ?.getAttribute('data-above-composer-conversation-id');
    // An explicit draft/invalid ID must never fall back to the previous chat.
    const candidate = fromComposer || (root.getAttribute('data-composer-placement') === 'thread'
      ? document.querySelector('[data-app-action-sidebar-thread-selected="true"][data-app-action-sidebar-thread-id]')
        ?.getAttribute('data-app-action-sidebar-thread-id') : '');
    const id = String(candidate || '').trim().replace(/^local:/, '');
    return uuid.test(id) ? id.toLowerCase() : '';
  }

  function visible(element) {
    if (!element || element.getClientRects().length === 0) return false;
    const style = getComputedStyle(element);
    if (style.display === 'none' || style.visibility === 'hidden' || style.visibility === 'collapse') return false;
    const rect = element.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }

  function visibleCard() {
    for (const card of document.querySelectorAll('[data-composer-surface-variant]')) {
      if (!visible(card) || card.closest(ownSelector)) continue;
      if ([...card.querySelectorAll(editorSelector)].some(visible)) return card;
    }
    return null;
  }

  return { snapshot, threadId, visibleCard, percent };
})()
