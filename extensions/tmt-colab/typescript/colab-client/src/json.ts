import { decodeText, requireValue, text } from './bytes.js';
/** Reject duplicates before JSON.parse loses them. No unsigned reserialization is used. */
export function strictJson(raw: Uint8Array, max: number, integerNumbers = false): unknown {
  requireValue(raw.length <= max);
  const source = decodeText(raw);
  let offset = 0;
  const ws = () => {
    while ([' ', '\t', '\n', '\r'].includes(source[offset] ?? 'x')) offset++;
  };
  const string = (): string => {
    const start = offset;
    requireValue(source[offset++] === '"');
    // Scan the token without a repeated regex: legal large strings can exhaust
    // browser regex stacks. JSON.parse still owns escape and control grammar.
    while (offset < source.length) {
      const next = source[offset++];
      if (next === '\\') offset++;
      else if (next === '"') {
        const value: string = JSON.parse(source.slice(start, offset));
        text(value);
        return value;
      }
    }
    requireValue(false);
  };
  const value = (depth: number): unknown => {
    requireValue(depth <= 128);
    ws();
    const first = source[offset];
    if (first === '"') return string();
    if (first === '[' || first === '{') {
      const object = first === '{',
        close = object ? '}' : ']';
      offset++;
      ws();
      const result: unknown[] | Record<string, unknown> = object ? Object.create(null) : [];
      const keys = new Set<string>();
      if (source[offset] === close) {
        offset++;
        return result;
      }
      while (true) {
        ws();
        if (object) {
          const key = string();
          requireValue(!keys.has(key));
          keys.add(key);
          ws();
          requireValue(source[offset++] === ':');
          (result as Record<string, unknown>)[key] = value(depth + 1);
        } else (result as unknown[]).push(value(depth + 1));
        ws();
        const next = source[offset++];
        if (next === close) return result;
        requireValue(next === ',');
      }
    }
    const start = offset;
    while (offset < source.length && !' \t\n\r,]}'.includes(source[offset])) offset++;
    const token = source.slice(start, offset);
    const parsed: unknown = JSON.parse(token);
    requireValue(typeof parsed !== 'number' || Number.isFinite(parsed));
    if (integerNumbers && typeof parsed === 'number')
      requireValue(Number.isSafeInteger(parsed) && parsed >= 0 && token === String(parsed));
    return parsed;
  };
  const out = value(0);
  ws();
  requireValue(offset === source.length);
  return out;
}
