import { inShell } from './entry.js';
import { shortLocation, shortLocationPrefix, validPagePrefix } from './short-links.js';

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

export function classifyHref(raw: string, origin: string, shell = inShell()): HrefKind {
  if (raw.length === 0 || raw.length > MAX_HREF) return { kind: 'none' };
  // No whitespace or control character anywhere, so no scheme can be split or hidden by one.
  if (/[\s\p{Cc}\\]/u.test(raw)) return { kind: 'none' };
  if (raw.startsWith('//')) return { kind: 'none' };
  // An authored page link is always `/p/<prefix>`; the app opens it at its own short location.
  const relative = /^(\/(?:colab\/)?p\/[0-9a-fA-F-]+)(#[A-Za-z0-9_-]{1,64})?$/.exec(raw);
  if (relative) {
    const prefix = shortLocationPrefix(relative[1], shell)?.toLowerCase();
    return prefix && validPagePrefix(prefix)
      ? { kind: 'page', href: `${shortLocation(prefix, shell)}${relative[2] ?? ''}` }
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
  const same = shortLocationPrefix(url.pathname, shell)?.toLowerCase();
  if (url.origin === origin && same && validPagePrefix(same))
    return { kind: 'page', href: `${shortLocation(same, shell)}${url.hash}` };
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

/**
 * What one `inline` call has learned, so no scan over its text is repeated: a search that found
 * nothing from a position finds nothing from any later one, and the brackets are matched once. Every
 * scan over a body is therefore linear, however many unmatched openers it holds.
 */
interface Scan {
  /** The position from which a backtick run of this length found no closing run. */
  noSpan: Map<number, number>;
  /** The position from which this emphasis delimiter found no closer. */
  noCloser: Map<string, number>;
  /** Each `[` that has a `]`, mapped to the index just past it; matched on first use. */
  brackets?: Map<number, number>;
}

const TICKS = /`+/y;
/** The length of the backtick run that starts at `at`, which must hold a backtick. */
function tickRun(source: string, at: number): number {
  TICKS.lastIndex = at;
  return TICKS.exec(source)![0].length;
}

/** The index of the run of exactly `length` backticks that closes a span whose opening run ends at
 * `from`, or -1. */
function spanClose(source: string, from: number, length: number, scan: Scan): number {
  const failed = scan.noSpan.get(length);
  if (failed !== undefined && from >= failed) return -1;
  const ticks = '`'.repeat(length);
  let close = source.indexOf(ticks, from);
  // A longer run is a different delimiter.
  while (close >= 0 && source[close + length] === '`')
    close = source.indexOf(ticks, close + tickRun(source, close));
  if (close < 0) scan.noSpan.set(length, from);
  return close;
}

/** Each `[` that has a matching `]` (nested pairs count, escapes and code spans do not), in one pass. */
function matchBrackets(source: string, scan: Scan): Map<number, number> {
  const ends = new Map<number, number>();
  const open: number[] = [];
  for (let i = 0; i < source.length; i++) {
    const char = source[i]!;
    if (char === '\\') i++;
    else if (char === '`') {
      const run = tickRun(source, i);
      const close = spanClose(source, i + run, run, scan);
      i = (close >= 0 ? close + run : i + run) - 1;
    } else if (char === '[') open.push(i);
    else if (char === ']' && open.length > 0) ends.set(open.pop()!, i + 1);
  }
  return ends;
}

/** The index of the delimiter that closes an emphasis whose text starts at `from`, or -1. */
function emphasisClose(
  source: string,
  from: number,
  char: string,
  width: number,
  scan: Scan,
): number {
  const delimiter = char.repeat(width);
  const failed = scan.noCloser.get(delimiter);
  if (failed !== undefined && from >= failed) return -1;
  let j = from;
  while (j < source.length) {
    if (source[j] === '\\') j += 2;
    else if (source[j] === '`') {
      const run = tickRun(source, j);
      const close = spanClose(source, j + run, run, scan);
      j = close >= 0 ? close + run : j + run;
    } else if (
      source.startsWith(delimiter, j) &&
      j > from &&
      !space(source[j - 1]) &&
      (char === '*' || !alnum(source[j + width])) &&
      !(width === 1 && source[j + 1] === char)
    )
      return j;
    else j++;
  }
  scan.noCloser.set(delimiter, from);
  return -1;
}

function inline(source: string, depth: number, budget: Budget, origin: string, linked = false) {
  const scan: Scan = { noSpan: new Map(), noCloser: new Map() };
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
      const run = tickRun(source, i);
      const close = spanClose(source, i + run, run, scan);
      if (close >= 0) {
        let value = source.slice(i + run, close).replace(/\n/g, ' ');
        if (value.length > 2 && value.startsWith(' ') && value.endsWith(' '))
          value = value.slice(1, -1);
        flush();
        spend(budget);
        out.push({ kind: 'code', value });
        i = close + run;
      } else {
        text += '`'.repeat(run);
        i += run;
      }
      continue;
    }
    if (char === '[' && !linked && depth <= MAX_DEPTH) {
      const end = (scan.brackets ??= matchBrackets(source, scan)).get(i) ?? -1;
      if (end > 0 && source[end] === '(') {
        // A target is at most MAX_HREF long, so the search for its `)` never reads further.
        const reach = source.slice(end + 1, end + 2 + MAX_HREF);
        const length = reach.indexOf(')');
        const target = length >= 0 ? reach.slice(0, length) : '';
        const kind = classifyHref(target, origin);
        if (length >= 0 && kind.kind !== 'none') {
          const label = inline(source.slice(i + 1, end - 1), depth + 1, budget, origin, true);
          flush();
          spend(budget);
          if (kind.kind === 'page') out.push({ kind: 'link', href: kind.href, children: label });
          // Outside links stay inert until the app confirms them (#4083): the words, then the URL.
          else out.push(...label, { kind: 'text', value: ` (${target})` });
          i = end + 1 + length + 1;
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
      const close = opens ? emphasisClose(source, i + width, char, width, scan) : -1;
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
