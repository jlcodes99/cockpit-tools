import type { JsonValue } from './benchmarkTypes.ts';

// New fixed-answer cases use bounded JSON without duplicate keys or lossy numbers.
// Legacy JSON format cases intentionally retain their original parser contract.
export function parseExactJson(text: string): JsonValue {
  let offset = 0;
  const whitespace = () => { while (/[\t\n\r ]/.test(text[offset] ?? '') && offset < text.length) offset++; };
  const fail = (): never => { throw new Error('Invalid exact JSON'); };
  const string = (): string => {
    if (text[offset++] !== '"') return fail();
    const start = offset - 1;
    while (offset < text.length) {
      const char = text[offset++];
      if (char === '\\') offset++;
      else if (char === '"') return JSON.parse(text.slice(start, offset)) as string;
    }
    return fail();
  };
  const decimal = (token: string): string => {
    const [coefficient, exponent = '0'] = token.toLowerCase().split('e');
    const [whole, fraction = ''] = coefficient.replace(/^-/, '').split('.');
    let digits = (whole + fraction).replace(/^0+/, '');
    if (!digits) return '0';
    // An end-anchored /0+$/ retries at every zero when a long run ends in
    // a nonzero digit. Scan backwards once to keep adversarial input linear.
    let end = digits.length;
    while (digits[end - 1] === '0') end--;
    const power = Number(exponent) - fraction.length + digits.length - end;
    digits = digits.slice(0, end);
    return `${coefficient.startsWith('-') ? '-' : ''}${digits}e${power}`;
  };
  const value = (depth: number): JsonValue => {
    if (depth > 64) return fail();
    whitespace();
    const char = text[offset];
    if (char === '"') return string();
    if (char === '{' || char === '[') {
      const object = char === '{';
      const result: Record<string, JsonValue> = Object.create(null);
      const items: JsonValue[] = [];
      const keys = new Set<string>();
      const close = object ? '}' : ']';
      offset++; whitespace();
      if (text[offset] === close) { offset++; return object ? result : items; }
      while (offset < text.length) {
        whitespace();
        let key = '';
        if (object) {
          key = string();
          if (keys.has(key)) return fail();
          keys.add(key);
          whitespace();
          if (text[offset++] !== ':') return fail();
        }
        const item = value(depth + 1);
        if (object) result[key] = item;
        else items.push(item);
        whitespace();
        const delimiter = text[offset++];
        if (delimiter === close) return object ? result : items;
        if (delimiter !== ',') return fail();
      }
      return fail();
    }
    for (const literal of ['true', 'false', 'null']) {
      if (text.startsWith(literal, offset)) { offset += literal.length; return JSON.parse(literal) as JsonValue; }
    }
    const number = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(text.slice(offset))?.[0];
    if (!number) return fail();
    offset += number.length;
    const parsed = Number(number);
    if (!Number.isFinite(parsed) || decimal(number) !== decimal(String(parsed))) return fail();
    return parsed;
  };
  const result = value(0);
  whitespace();
  if (offset !== text.length) return fail();
  return result;
}

export function exactJsonEqual(actual: JsonValue | undefined, expected: JsonValue): boolean {
  if (actual === expected) return true;
  if (actual === null || expected === null || typeof actual !== 'object' || typeof expected !== 'object') return false;
  if (Array.isArray(actual) !== Array.isArray(expected)) return false;
  const keys = Object.keys(expected);
  return Object.keys(actual).length === keys.length && keys.every((key) =>
    Object.prototype.hasOwnProperty.call(actual, key) && exactJsonEqual((actual as Record<string, JsonValue>)[key], (expected as Record<string, JsonValue>)[key]));
}
