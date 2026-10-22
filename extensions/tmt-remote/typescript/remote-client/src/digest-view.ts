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

/** Seats the ranking lists before the rest fold into one row. */
export const RANKING_KEPT = 8;

export interface Ranking {
  /** The leading seats, largest first. */
  top: RankRow[];
  /** Every seat after them, folded: null when none is left out. */
  other: { seats: number; tokens: number; share: number } | null;
  /** True when any seat, listed or folded, lacks evidence for part of the window. */
  partial: boolean;
}

/** The ranking's top `kept` seats and one folded row for the rest. */
export function ranking(rows: readonly RankRow[], kept = RANKING_KEPT): Ranking {
  const rest = rows.slice(kept);
  const total = rows.reduce((sum, row) => sum + row.tokens, 0);
  const tokens = rest.reduce((sum, row) => sum + row.tokens, 0);
  return {
    top: rows.slice(0, kept),
    other: rest.length
      ? { seats: rest.length, tokens, share: total ? Math.round((tokens / total) * 100) : 0 }
      : null,
    partial: rows.some((row) => row.partial),
  };
}

export interface Exchange {
  participants: { id: string; label: string }[];
  value: (from: string, to: string) => number | null;
}

const COUNTED = new Set(['talk', 'reply']);

/**
 * Who talks to whom: only talk and reply count (an ack or a result is not a conversation),
 * all seats ranked by messages sent plus received. Null without a flow
 * read; a pair with no message is a known none (0).
 */
export function exchange(doc: UsageDocument): Exchange | null {
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
    .sort(
      (a, b) => (sent.get(b.id) ?? 0) - (sent.get(a.id) ?? 0) || a.label.localeCompare(b.label),
    );
  return { participants, value: (from, to) => pairs.get(`${from}>${to}`) ?? 0 };
}

/** Seats by tokens in the window, largest first (ties by name), as `rankRows` orders them. */
function ranked(seats: readonly UsageSeat[]): UsageSeat[] {
  return [...seats].sort(
    (a, b) => tokensOf(b.totals) - tokensOf(a.totals) || seatName(a).localeCompare(seatName(b)),
  );
}

export interface Activity {
  rows: { id: string; label: string; slots: (number | null)[] }[];
  /** The start of each slot, oldest first. */
  starts: number[];
  /** What one slot covers: an hour in the 24 h report, a day in the 7d and 30d ones. */
  slot: 'hour' | 'day';
}

const SLOTS: Record<string, { count: number; stepMs: number; slot: 'hour' | 'day' }> = {
  '24h': { count: 24, stepMs: 3_600_000, slot: 'hour' },
  '7d': { count: 7, stepMs: 86_400_000, slot: 'day' },
  '30d': { count: 30, stepMs: 86_400_000, slot: 'day' },
};

/**
 * One slot per seat across the range, at the resolution the report carries: hourly for 24 h,
 * daily for 7d and 30d. The slots end at the newest bucket; one with no bucket is not observed
 * (null), never zero. Seats come in the ranking's order: most tokens first.
 */
export function activity(doc: UsageDocument): Activity | null {
  const shape = SLOTS[doc.since];
  if (!shape) return null;
  const latest = Math.max(
    ...doc.usage.seats.flatMap((seat) => seat.series.map((point) => point.fromMs)),
  );
  if (!Number.isFinite(latest)) return null;
  const starts = Array.from(
    { length: shape.count },
    (_, at) => latest - (shape.count - 1 - at) * shape.stepMs,
  );
  return {
    starts,
    slot: shape.slot,
    rows: ranked(doc.usage.seats).map((seat: UsageSeat) => {
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

/** A provider keeps one hue across its meter and its chart: orange for Claude, aqua for OpenAI. */
export function lineStyle(line: string): number {
  return line === 'claude' ? 1 : line === 'gpt' ? 2 : 0;
}

export interface AllowanceRow {
  /** The model family the window belongs to (`claude`, `gpt`), empty when unknown. */
  line: string;
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
        line,
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
        title: 'Weekly percent left',
        kind: 'line' as const,
        style: lineStyle(history.line),
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

export interface ProviderView extends AllowanceRow {
  style: number;
  /** The percent-left history, when the read serves it for this provider. */
  chart: SeriesChartOptions | undefined;
}

/** One entry per weekly window: its numbers beside its history chart, matched by provider. */
export function providers(doc: UsageDocument, zone?: string): ProviderView[] {
  const charts = new Map(historyCharts(doc, zone).map((chart) => [chart.series[0]!.key, chart]));
  return allowance(doc).map((row) => ({
    ...row,
    style: lineStyle(row.line),
    chart: charts.get(row.line),
  }));
}

/** Days between two instants, for the page's "resets in" words. */
export function resetsIn(resetsAtMs: number, nowMs: number): string {
  const ms = resetsAtMs - nowMs;
  if (ms <= 0) return 'now';
  if (ms >= DAY) return `${Math.floor(ms / DAY)}d ${Math.floor((ms % DAY) / HOUR)}h`;
  return `${Math.max(1, Math.floor(ms / HOUR))}h`;
}
