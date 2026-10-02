import { decodeText, requireValue, text } from './bytes.js';
/** Reject duplicates before JSON.parse loses them. No unsigned reserialization is used. */
export function strictJson(raw: Uint8Array, max: number): unknown {
  requireValue(raw.length <= max);
  const source = decodeText(raw);
  let offset = 0;
  const ws = () => {
    while ([' ', '\t', '\n', '\r'].includes(source[offset] ?? 'x')) offset++;
  };
  const string = (): string => {
    // JSON forbids unescaped U+0000..U+001F; this range is required by its grammar.
    // eslint-disable-next-line no-control-regex
    const match = /^"(?:[^"\\\x00-\x1f]|\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4}))*"/.exec(
      source.slice(offset),
    );
    requireValue(match !== null);
    offset += match[0].length;
    const value: string = JSON.parse(match[0]);
    text(value);
    return value;
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
    const token = /^(?:null|true|false|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)/.exec(
      source.slice(offset),
    );
    requireValue(token !== null);
    offset += token[0].length;
    const parsed: unknown = JSON.parse(token[0]);
    requireValue(typeof parsed !== 'number' || Number.isFinite(parsed));
    return parsed;
  };
  const out = value(0);
  ws();
  requireValue(offset === source.length);
  return out;
}
