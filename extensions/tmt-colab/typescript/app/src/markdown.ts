import { validPagePrefix } from './short-links.js';

/**
 * A small, safe Markdown subset for comment and Chat bodies (#4112). The parser outputs a tree, never
 * HTML: the renderer turns it into React elements, so no author text is ever parsed as markup.
 *
 * Blocks: paragraph, fenced code, block quote, bullet and numbered lists (nested to `MAX_DEPTH`).
 * Inline: `code`, **strong**, *emphasis* and `[text](link)`. Everything else stays literal text: raw
 * HTML, entities as typed, headings, tables, images and any other syntax. A source over `MAX_SOURCE`
 * characters, or a tree over `MAX_NODES`, falls back to plain paragraphs.
 */
export type Inline =
  | { kind: 'text'; value: string }
  | { kind: 'code'; value: string }
  | { kind: 'strong' | 'emphasis'; children: Inline[] }
  | { kind: 'link'; href: string; children: Inline[] };
export type Block =
  | { kind: 'paragraph'; children: Inline[] }
  | { kind: 'code'; value: string }
  | { kind: 'quote'; children: Block[] }
  | { kind: 'list'; ordered: boolean; start: number; items: Block[][] };

export const MAX_SOURCE = 20_000;
export const MAX_DEPTH = 3;
export const MAX_NODES = 4_000;
const MAX_HREF = 2_048;

class Overflow extends Error {}
interface Budget {
  nodes: number;
}
const spend = (budget: Budget) => {
  if (++budget.nodes > MAX_NODES) throw new Overflow();
};

/** What a link target is: a page of this space (opens in the app), an outside URL (shown inert until
 * the app confirms outside links) or nothing safe (kept as the text the author wrote). */
export type HrefKind =
  | { kind: 'page'; href: string }
  | { kind: 'outside'; href: string }
  | { kind: 'none' };

export function classifyHref(raw: string, origin: string): HrefKind {
  if (raw.length === 0 || raw.length > MAX_HREF) return { kind: 'none' };
  // No whitespace or control character anywhere, so no scheme can be split or hidden by one.
  if (/[\s\p{Cc}\\]/u.test(raw)) return { kind: 'none' };
  if (raw.startsWith('//')) return { kind: 'none' };
  const relative = /^\/p\/([0-9a-fA-F-]+)(#[A-Za-z0-9_-]{1,64})?$/.exec(raw);
  if (relative) {
    return validPagePrefix(relative[1].toLowerCase())
      ? { kind: 'page', href: raw }
      : { kind: 'none' };
  }
  if (!/^https?:\/\//i.test(raw)) return { kind: 'none' };
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return { kind: 'none' };
  }
  if (url.username || url.password) return { kind: 'none' };
  const same = /^\/p\/([0-9a-fA-F-]+)$/.exec(url.pathname);
  if (url.origin === origin && same && validPagePrefix(same[1].toLowerCase()))
    return { kind: 'page', href: `${url.pathname}${url.hash}` };
  const local = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
  if (url.protocol === 'https:' || (url.protocol === 'http:' && local))
    return { kind: 'outside', href: url.href };
  return { kind: 'none' };
}

const plain = (source: string): Block[] =>
  source
    .split(/\n{2,}/)
    .filter((part) => part.trim() !== '')
    .map((part) => ({ kind: 'paragraph', children: [{ kind: 'text', value: part }] }));

/** The tree of a body; a body that is too large or too deep for the subset stays plain text. */
export function parseMarkdown(source: string, origin: string): Block[] {
  const normalized = source.replace(/\r\n?/g, '\n');
  if (normalized.length > MAX_SOURCE) return plain(normalized);
  try {
    return blocks(normalized.split('\n'), 0, { nodes: 0 }, origin);
  } catch (error) {
    if (error instanceof Overflow) return plain(normalized);
    throw error;
  }
}

const FENCE = /^ {0,3}(`{3,}|~{3,})[^`]*$/;
const QUOTE = /^ {0,3}>[ \t]?(.*)$/;
const ITEM = /^( {0,3})([-*+]|\d{1,9}[.)])[ \t]+(\S.*)$/;
const blank = (line: string) => line.trim() === '';
const startsBlock = (line: string) => FENCE.test(line) || QUOTE.test(line) || ITEM.test(line);

function blocks(lines: string[], depth: number, budget: Budget, origin: string): Block[] {
  const out: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i]!;
    if (blank(line)) {
      i++;
      continue;
    }
    const fence = FENCE.exec(line);
    if (fence) {
      const marker = fence[1]!;
      const close = new RegExp(
        `^ {0,3}${marker[0] === '`' ? '`' : '~'}{${marker.length},}[ \\t]*$`,
      );
      const body: string[] = [];
      i++;
      while (i < lines.length && !close.test(lines[i]!)) body.push(lines[i++]!);
      i++;
      spend(budget);
      out.push({ kind: 'code', value: body.join('\n') });
      continue;
    }
    if (QUOTE.test(line)) {
      const inner: string[] = [];
      while (i < lines.length && QUOTE.test(lines[i]!)) inner.push(QUOTE.exec(lines[i++]!)![1]!);
      spend(budget);
      // Past the depth cap the quote's own text is kept, markers included.
      out.push(
        depth >= MAX_DEPTH
          ? { kind: 'paragraph', children: [{ kind: 'text', value: inner.join('\n') }] }
          : { kind: 'quote', children: blocks(inner, depth + 1, budget, origin) },
      );
      continue;
    }
    const item = ITEM.exec(line);
    if (item) {
      i = list(lines, i, depth, budget, origin, out);
      continue;
    }
    const paragraph: string[] = [];
    while (
      i < lines.length &&
      !blank(lines[i]!) &&
      (paragraph.length === 0 || !startsBlock(lines[i]!))
    )
      paragraph.push(lines[i++]!);
    spend(budget);
    out.push({ kind: 'paragraph', children: inline(paragraph.join('\n'), 0, budget, origin) });
  }
  return out;
}

function list(
  lines: string[],
  start: number,
  depth: number,
  budget: Budget,
  origin: string,
  out: Block[],
): number {
  const first = ITEM.exec(lines[start]!)!;
  const ordered = /\d/.test(first[2]!);
  const base = first[1]!.length;
  const items: Block[][] = [];
  let i = start;
  while (i < lines.length) {
    const match = ITEM.exec(lines[i]!);
    if (!match || /\d/.test(match[2]!) !== ordered || match[1]!.length > base + 1) break;
    const contentAt = match[1]!.length + match[2]!.length + 1;
    const content = [match[3]!];
    i++;
    // The item's own lines: more indented than its marker (continuations and nested lists).
    while (i < lines.length) {
      const next = lines[i]!;
      if (blank(next)) {
        // A blank line stays inside the list only when another item of this list follows.
        let j = i;
        while (j < lines.length && blank(lines[j]!)) j++;
        const following = j < lines.length ? ITEM.exec(lines[j]!) : null;
        if (following && /\d/.test(following[2]!) === ordered && following[1]!.length <= base + 1)
          i = j;
        break;
      }
      const indent = /^ */.exec(next)![0].length;
      if (indent < base + 2) break;
      content.push(next.slice(Math.min(indent, contentAt)));
      i++;
    }
    spend(budget);
    items.push(
      depth >= MAX_DEPTH
        ? [{ kind: 'paragraph', children: [{ kind: 'text', value: content.join('\n') }] }]
        : blocks(content, depth + 1, budget, origin),
    );
  }
  const number = ordered ? Number.parseInt(first[2]!, 10) : 1;
  spend(budget);
  out.push({ kind: 'list', ordered, start: Number.isFinite(number) ? number : 1, items });
  return i;
}

const ESCAPABLE = /[\\`*_{}[\]()#+\-.!>~|<]/;
const alnum = (char: string | undefined) => char !== undefined && /[\p{L}\p{N}]/u.test(char);
const space = (char: string | undefined) => char === undefined || /\s/u.test(char);

/** The index just past the closing bracket of a `[` at `from`, or -1. */
function closeBracket(source: string, from: number): number {
  let nesting = 0;
  for (let i = from; i < source.length; i++) {
    const char = source[i]!;
    if (char === '\\') i++;
    else if (char === '`') {
      const run = /^`+/.exec(source.slice(i))![0].length;
      const close = source.indexOf('`'.repeat(run), i + run);
      if (close >= 0) i = close + run - 1;
      else i += run - 1;
    } else if (char === '[') nesting++;
    else if (char === ']' && --nesting === 0) return i + 1;
  }
  return -1;
}

function inline(source: string, depth: number, budget: Budget, origin: string, linked = false) {
  const out: Inline[] = [];
  let text = '';
  const flush = () => {
    if (text === '') return;
    spend(budget);
    out.push({ kind: 'text', value: text });
    text = '';
  };
  let i = 0;
  while (i < source.length) {
    const char = source[i]!;
    if (char === '\\' && i + 1 < source.length && ESCAPABLE.test(source[i + 1]!)) {
      text += source[i + 1];
      i += 2;
      continue;
    }
    if (char === '`') {
      const run = /^`+/.exec(source.slice(i))![0].length;
      const ticks = '`'.repeat(run);
      let close = source.indexOf(ticks, i + run);
      // A longer run is a different delimiter.
      while (close >= 0 && source[close + run] === '`') {
        const longer = /^`+/.exec(source.slice(close))![0].length;
        close = source.indexOf(ticks, close + longer);
      }
      if (close >= 0) {
        let value = source.slice(i + run, close).replace(/\n/g, ' ');
        if (value.length > 2 && value.startsWith(' ') && value.endsWith(' '))
          value = value.slice(1, -1);
        flush();
        spend(budget);
        out.push({ kind: 'code', value });
        i = close + run;
      } else {
        text += ticks;
        i += run;
      }
      continue;
    }
    if (char === '[' && !linked && depth <= MAX_DEPTH) {
      const end = closeBracket(source, i);
      if (end > 0 && source[end] === '(') {
        const stop = source.indexOf(')', end + 1);
        const target = stop > 0 ? source.slice(end + 1, stop) : '';
        const kind = classifyHref(target, origin);
        if (stop > 0 && kind.kind !== 'none') {
          const label = inline(source.slice(i + 1, end - 1), depth + 1, budget, origin, true);
          flush();
          spend(budget);
          if (kind.kind === 'page') out.push({ kind: 'link', href: kind.href, children: label });
          // Outside links stay inert until the app confirms them (#4083): the words, then the URL.
          else out.push(...label, { kind: 'text', value: ` (${target})` });
          i = stop + 1;
          continue;
        }
      }
    }
    if ((char === '*' || char === '_') && depth < MAX_DEPTH) {
      const width = source.startsWith(char.repeat(2), i) ? 2 : 1;
      const delimiter = char.repeat(width);
      const opens =
        !space(source[i + width]) &&
        (char === '*' || !alnum(source[i - 1])) &&
        !(width === 1 && source[i + 1] === char);
      if (opens) {
        let j = i + width;
        let close = -1;
        while (j < source.length) {
          if (source[j] === '\\') j += 2;
          else if (source[j] === '`') {
            const run = /^`+/.exec(source.slice(j))![0].length;
            const end = source.indexOf('`'.repeat(run), j + run);
            j = end >= 0 ? end + run : j + run;
          } else if (
            source.startsWith(delimiter, j) &&
            j > i + width &&
            !space(source[j - 1]) &&
            (char === '*' || !alnum(source[j + width])) &&
            !(width === 1 && source[j + 1] === char)
          ) {
            close = j;
            break;
          } else j++;
        }
        if (close > 0) {
          flush();
          spend(budget);
          out.push({
            kind: width === 2 ? 'strong' : 'emphasis',
            children: inline(source.slice(i + width, close), depth + 1, budget, origin, linked),
          });
          i = close + width;
          continue;
        }
      }
      text += delimiter;
      i += width;
      continue;
    }
    text += char;
    i++;
  }
  flush();
  return out;
}
