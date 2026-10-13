/** One anchored change against the source the editor opened; mirrors `Hunk` in `page/patch.rs`.
 * `old` is whole lines (the removed lines with their context) and `oldStart` is the 1-based line
 * of the opened source where it starts. */
export interface SaveHunk {
  oldStart: number;
  old: string;
  new: string;
}

/** Lines of unchanged text kept on each side of a change, so it finds its place again. */
const CONTEXT = 3;
/** The most hunks a Save carries; the serve's `WireHunk` list bound. */
export const SAVE_HUNKS = 64;
/** The most text the hunks may hold: more cannot fit the 64 KiB frame a Save travels in. */
const HUNK_CHARACTERS = 64 * 1024;
/** The most changed lines the diff will work out before the whole source is saved instead. */
const CHANGED_LINES = 2000;

/** The lines of `source`, each with its terminating LF (the last may lack one). */
function linesOf(source: string): string[] {
  const lines: string[] = [];
  let from = 0;
  while (from < source.length) {
    const at = source.indexOf('\n', from);
    const end = at < 0 ? source.length : at + 1;
    lines.push(source.slice(from, end));
    from = end;
  }
  return lines;
}

interface Region {
  /** Lines `[from, to)` of the old source became `[start, end)` of the new one. */
  from: number;
  to: number;
  start: number;
  end: number;
}

/** The changed regions between two line lists (Myers), or `undefined` past `limit` changed lines. */
function regionsOf(a: string[], b: string[], limit: number): Region[] | undefined {
  const n = a.length,
    m = b.length,
    max = Math.min(n + m, limit),
    offset = max + 1,
    v = new Int32Array(2 * max + 3),
    trace: Int32Array[] = [];
  let found = -1;
  for (let d = 0; d <= max && found < 0; d++) {
    // The furthest points of the previous round, for k in [-d - 1, d + 1].
    trace.push(v.slice(offset - d - 1, offset + d + 2));
    for (let k = -d; k <= d; k += 2) {
      let x =
        k === -d || (k !== d && v[offset + k - 1] < v[offset + k + 1])
          ? v[offset + k + 1]
          : v[offset + k - 1] + 1;
      let y = x - k;
      while (x < n && y < m && a[x] === b[y]) {
        x++;
        y++;
      }
      v[offset + k] = x;
      if (x >= n && y >= m) {
        found = d;
        break;
      }
    }
  }
  if (found < 0) return undefined;
  // Walk back from the end, collecting each single-line step.
  const steps: ('delete' | 'insert' | 'keep')[] = [];
  let x = n,
    y = m;
  for (let d = found; d > 0; d--) {
    const previous = trace[d],
      at = (k: number) => previous[k + d + 1];
    const k = x - y;
    const from = k === -d || (k !== d && at(k - 1) < at(k + 1)) ? k + 1 : k - 1;
    const px = at(from),
      py = px - from;
    while (x > px && y > py) {
      steps.push('keep');
      x--;
      y--;
    }
    steps.push(x === px ? 'insert' : 'delete');
    x = px;
    y = py;
  }
  while (x > 0 && y > 0) {
    steps.push('keep');
    x--;
    y--;
  }
  steps.reverse();
  const regions: Region[] = [];
  let ai = 0,
    bi = 0,
    open: Region | null = null;
  for (const step of steps) {
    if (step === 'keep') {
      open = null;
      ai++;
      bi++;
      continue;
    }
    if (!open) {
      open = { from: ai, to: ai, start: bi, end: bi };
      regions.push(open);
    }
    if (step === 'delete') open.to = ++ai;
    else open.end = ++bi;
  }
  return regions;
}

/**
 * The editor's change as anchored hunks with three lines of context, for a Save to carry in case
 * the page changed elsewhere since `base` was opened. `undefined` when there is nothing to anchor
 * (no change, or an empty base) or the change is too large to send this way; the whole source is
 * then saved as before.
 */
export function hunksOf(base: string, draft: string): SaveHunk[] | undefined {
  if (base === '' || base === draft) return undefined;
  const a = linesOf(base),
    b = linesOf(draft);
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head++;
  let tail = 0;
  while (
    tail < a.length - head &&
    tail < b.length - head &&
    a[a.length - 1 - tail] === b[b.length - 1 - tail]
  )
    tail++;
  const found = regionsOf(
    a.slice(head, a.length - tail),
    b.slice(head, b.length - tail),
    CHANGED_LINES,
  );
  if (!found || found.length === 0) return undefined;
  const regions = found.map((r) => ({
    from: r.from + head,
    to: r.to + head,
    start: r.start + head,
    end: r.end + head,
  }));
  // Changes whose context would touch are one hunk.
  const groups: Region[][] = [];
  for (const region of regions) {
    const last = groups.at(-1);
    if (last && region.from - last.at(-1)!.to <= 2 * CONTEXT) last.push(region);
    else groups.push([region]);
  }
  if (groups.length > SAVE_HUNKS) return undefined;
  const hunks = groups.map((group) => {
    const first = group[0],
      last = group.at(-1)!;
    const from = Math.max(0, first.from - CONTEXT),
      to = Math.min(a.length, last.to + CONTEXT);
    let after = a.slice(from, first.from).join('');
    for (let i = 0; i < group.length; i++) {
      after += b.slice(group[i].start, group[i].end).join('');
      const next = group[i + 1];
      after += a.slice(group[i].to, next ? next.from : to).join('');
    }
    return { oldStart: from + 1, old: a.slice(from, to).join(''), new: after };
  });
  // Hunks that cannot fit one frame are not worth sending: the whole source is saved instead.
  const characters = hunks.reduce((sum, hunk) => sum + hunk.old.length + hunk.new.length, 0);
  return characters > HUNK_CHARACTERS ? undefined : hunks;
}
