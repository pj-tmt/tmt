/**
 * The numbers the Digest page paints, derived from the public usage document. Pure: no DOM, no
 * clock, so every section is tested on plain data. A missing observation stays missing (`null`):
 * it is never turned into a zero here.
 */
import type { Layer, SeriesChartOptions } from '@tmt/browser-ui/chart';
import type { UsageDocument, UsagePoint, UsageSeat } from './digest-source.js';

const HOUR = 3_600_000;
const DAY = 24 * HOUR;

/** Input plus output; cached input is part of input and is never added. */
export const tokensOf = (point: Pick<UsagePoint, 'input' | 'output'>) => point.input + point.output;

export function seatName(seat: { id: string; name: string | null; retired: boolean }): string {
  const base = seat.name ?? seat.id.slice(0, 8);
  return seat.retired ? `${base} (retired)` : base;
}

function stamp(ms: number, range: UsageDocument['since'], zone?: string): string {
  const parts = new Intl.DateTimeFormat('en-GB', {
    timeZone: zone,
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).formatToParts(new Date(ms));
  const at = (type: string) => parts.find((part) => part.type === type)!.value;
  return range === '24h' ? `${at('hour')}:${at('minute')}` : `${at('month')}-${at('day')}`;
}

export interface TokenSeries {
  x: string[];
  layers: Layer[];
  /** True when any bucket is a lower bound (an incomplete read). */
  partial: boolean;
}

/** Tokens per bucket for every seat, on the union of the seats' bucket starts. */
export function tokenSeries(doc: UsageDocument, zone?: string): TokenSeries {
  const starts = [
    ...new Set(doc.usage.seats.flatMap((seat) => seat.series.map((point) => point.fromMs))),
  ].sort((a, b) => a - b);
  return {
    x: starts.map((at) => stamp(at, doc.since, zone)),
    layers: doc.usage.seats.map((seat) => {
      const byStart = new Map(seat.series.map((point) => [point.fromMs, point]));
      return {
        key: seat.id,
        label: seatName(seat),
        values: starts.map((at) => {
          const point = byStart.get(at);
          return point ? tokensOf(point) : null;
        }),
      };
    }),
    partial: doc.usage.seats.some((seat) => seat.series.some((point) => !point.complete)),
  };
}

export interface RankRow {
  id: string;
  name: string;
  tokens: number;
  /** Whole percent of the listed seats' tokens. */
  share: number;
  /** The seat's evidence does not cover the whole window. */
  partial: boolean;
}

/** Seats by tokens in the window, largest first (ties by name). */
export function rankRows(doc: UsageDocument): RankRow[] {
  const rows = doc.usage.seats.map((seat) => ({
    id: seat.id,
    name: seatName(seat),
    tokens: tokensOf(seat.totals),
    partial: !seat.coverage.complete,
  }));
  const total = rows.reduce((sum, row) => sum + row.tokens, 0);
  return rows
    .map((row) => ({ ...row, share: total ? Math.round((row.tokens / total) * 100) : 0 }))
    .sort((a, b) => b.tokens - a.tokens || a.name.localeCompare(b.name));
}

export interface Exchange {
  participants: { id: string; label: string }[];
  value: (from: string, to: string) => number | null;
}

const COUNTED = new Set(['talk', 'reply']);

/**
 * Who talks to whom: only talk and reply count (an ack or a result is not a conversation),
 * seats ranked by messages sent plus received, the busiest `limit` kept. Null without a flow
 * read; a pair with no message is a known none (0).
 */
export function exchange(doc: UsageDocument, limit = 8): Exchange | null {
  const flow = doc.exchange;
  if (!flow) return null;
  const labels = new Map<string, string>();
  for (const seat of doc.usage.seats) labels.set(seat.id, seatName(seat));
  const sent = new Map<string, number>();
  const pairs = new Map<string, number>();
  for (const pair of flow.pairs) {
    const { sender, recipient } = pair;
    if (!COUNTED.has(pair.kind) || !sender.id || !recipient.id || sender.id === recipient.id) {
      continue;
    }
    for (const party of [sender, recipient]) {
      if (!labels.has(party.id!)) labels.set(party.id!, seatName({ ...party, id: party.id! }));
    }
    pairs.set(
      `${sender.id}>${recipient.id}`,
      (pairs.get(`${sender.id}>${recipient.id}`) ?? 0) + pair.count,
    );
    for (const id of [sender.id, recipient.id]) sent.set(id, (sent.get(id) ?? 0) + pair.count);
  }
  const participants = [...labels]
    .map(([id, label]) => ({ id, label }))
    .sort((a, b) => (sent.get(b.id) ?? 0) - (sent.get(a.id) ?? 0) || a.label.localeCompare(b.label))
    .slice(0, limit);
  return { participants, value: (from, to) => pairs.get(`${from}>${to}`) ?? 0 };
}

export interface Activity {
  rows: { id: string; label: string; slots: (number | null)[] }[];
  /** The start of each slot, oldest first. */
  starts: number[];
}

/**
 * One hourly slot per seat for the last 24 hours. Only the 24 h report has hourly buckets; a
 * longer range has none, so there is no activity view for it (null), not a coarse imitation.
 */
export function activity(doc: UsageDocument): Activity | null {
  if (doc.since !== '24h') return null;
  const starts = [
    ...new Set(doc.usage.seats.flatMap((seat) => seat.series.map((point) => point.fromMs))),
  ]
    .sort((a, b) => a - b)
    .slice(-24);
  if (starts.length !== 24) return null;
  return {
    starts,
    rows: doc.usage.seats.map((seat: UsageSeat) => {
      const byStart = new Map(seat.series.map((point) => [point.fromMs, point]));
      return {
        id: seat.id,
        label: seatName(seat),
        slots: starts.map((at) => {
          const point = byStart.get(at);
          return point ? tokensOf(point) : null;
        }),
      };
    }),
  };
}

const LINE_LABELS: Record<string, string> = { claude: 'Claude', gpt: 'OpenAI' };

export interface AllowanceRow {
  provider: string;
  leftPercent: number;
  /** Share of the window elapsed, null for a stale window. */
  elapsedPercent: number | null;
  /** Used minus elapsed in points; negative is under an even pace. */
  paceDeltaPoints: number | null;
  resetsAtMs: number;
  stale: boolean;
}

/** The weekly windows, one row each: how much is left and whether the pace is even. */
export function allowance(doc: UsageDocument): AllowanceRow[] {
  const lines = new Map(doc.usage.seats.map((seat) => [seat.id, seat.lines[0] ?? '']));
  return doc.limits
    .filter((limit) => limit.windowMinutes === 10080)
    .map((limit) => {
      const line = limit.seats.map((id) => lines.get(id)).find(Boolean) ?? '';
      return {
        provider: LINE_LABELS[line] ?? (line ? line[0]!.toUpperCase() + line.slice(1) : 'Account'),
        leftPercent: Math.round((100 - limit.usedPercent) * 10) / 10,
        elapsedPercent: limit.elapsedPercent,
        paceDeltaPoints: limit.paceDeltaPoints,
        resetsAtMs: limit.resetsAtMs,
        stale: limit.stale,
      };
    })
    .sort((a, b) => a.provider.localeCompare(b.provider) || a.resetsAtMs - b.resetsAtMs);
}

/** Weekly percent-left history charts, one per provider; none without the history field. */
export function historyCharts(doc: UsageDocument, zone?: string): SeriesChartOptions[] {
  return (doc.limitHistory ?? [])
    .filter((history) => history.points.length > 1)
    .map((history) => {
      const x = history.points.map((point) => stamp(point.atMs, '7d', zone));
      const window = history.windowMinutes * 60_000;
      return {
        title: `${LINE_LABELS[history.line] ?? history.line}, weekly percent left`,
        kind: 'line' as const,
        x,
        series: [
          {
            key: history.line,
            label: 'left',
            values: history.points.map((point) => point.percentLeft),
          },
        ],
        reference: {
          label: 'even pace',
          values: history.points.map((point) =>
            Math.max(0, Math.min(100, (100 * (history.resetsAtMs - point.atMs)) / window)),
          ),
        },
        max: 100,
        format: (value: number) => `${Math.round(value)}%`,
      };
    });
}

/** Days between two instants, for the page's "resets in" words. */
export function resetsIn(resetsAtMs: number, nowMs: number): string {
  const ms = resetsAtMs - nowMs;
  if (ms <= 0) return 'now';
  if (ms >= DAY) return `${Math.floor(ms / DAY)}d ${Math.floor((ms % DAY) / HOUR)}h`;
  return `${Math.max(1, Math.floor(ms / HOUR))}h`;
}
