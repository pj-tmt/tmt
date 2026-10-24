/**
 * The numbers the Digest page paints, derived from the public usage document. Pure: no DOM, no
 * clock, so every section is tested on plain data. A missing observation stays missing (`null`):
 * it is never turned into a zero here.
 */
import type { Layer, SeriesChartOptions } from '@tmt/browser-ui/chart';
import type { LimitWindow, UsageDocument, UsagePoint, UsageSeat } from './digest-source.js';

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
  /** Observed window clipped to clock-aligned presentation buckets. */
  starts: number[];
  ends: number[];
}

const SLOTS: Record<string, { count: number; stepMs: number; group: number }> = {
  '24h': { count: 24, stepMs: HOUR, group: 4 },
  '7d': { count: 7, stepMs: DAY, group: 1 },
  '30d': { count: 30, stepMs: DAY, group: 5 },
};

/**
 * Group real hourly/daily reads into four-hour, daily or five-day activity bars. Bins align to
 * the displayed local four-hour clock (daily groups retain report boundaries). A rolling 24h
 * normally has seven clipped bars, with
 * partial first/last bins. No extra observations are invented. If any constituent is missing,
 * the whole bar is hollow; an observed zero remains zero. Incomplete reads remain lower bounds.
 */
export function activity(doc: UsageDocument): Activity | null {
  const shape = SLOTS[doc.since];
  if (!shape) return null;
  const latest = Math.max(
    ...doc.usage.seats.flatMap((seat) => seat.series.map((point) => point.fromMs)),
  );
  if (!Number.isFinite(latest)) return null;
  const first = latest - (shape.count - 1) * shape.stepMs;
  const end = latest + shape.stepMs;
  const stepMs = shape.stepMs * shape.group;
  const groupAt = (at: number) =>
    Math.floor(
      (at - (doc.since === '24h' ? new Date(at).getTimezoneOffset() * 60_000 : 0)) / stepMs,
    );
  const bins: number[][] = [];
  for (let at = first; at < end; at += shape.stepMs) {
    const bucket = groupAt(at);
    const previous = bins[bins.length - 1];
    if (previous && groupAt(previous[0]!) === bucket) previous.push(at);
    else bins.push([at]);
  }
  return {
    starts: bins.map((bin) => bin[0]!),
    ends: bins.map((bin) => bin[bin.length - 1]! + shape.stepMs),
    rows: ranked(doc.usage.seats).map((seat) => {
      const byStart = new Map(seat.series.map((point) => [point.fromMs, point]));
      return {
        id: seat.id,
        label: seatName(seat),
        slots: bins.map((bin) =>
          bin.every((at) => byStart.has(at))
            ? bin.reduce((sum, at) => sum + tokensOf(byStart.get(at)!), 0)
            : null,
        ),
      };
    }),
  };
}

const PROVIDER_LABELS: Record<string, string> = { claude: 'Claude', codex: 'OpenAI' };

/** One hue per source, shared by its meter and history. */
export function providerStyle(provider: string): number {
  return provider === 'claude' ? 1 : provider === 'codex' ? 2 : 0;
}

/** Exact source/window/reset identity, also used for history and stable disclosure IDs. */
export function windowKey(
  window: Pick<LimitWindow, 'provider' | 'windowMinutes' | 'resetsAtMs'>,
): string {
  return `${encodeURIComponent(window.provider)}-${window.windowMinutes}-${window.resetsAtMs}`;
}

export interface AllowanceRow {
  key: string;
  source: string;
  provider: string;
  leftPercent: number;
  /** Share of the window elapsed, null for a stale observation. */
  elapsedPercent: number | null;
  /** Used minus elapsed in points; negative is under an even pace. */
  paceDeltaPoints: number | null;
  resetsAtMs: number;
  observedAtMs: number;
  reporters: string[];
  current: boolean;
  stale: boolean;
}

/** Weekly windows still active at the owner's read time. Unknown sources stay explicit. */
export function allowance(doc: UsageDocument): AllowanceRow[] {
  const seats = new Map(
    [...doc.usage.seats, ...doc.usage.unavailable].map((seat) => [seat.id, seat]),
  );
  // Core owns exact source/window/reset deduplication. The page only selects active weeks.
  return doc.limits
    .filter(
      (limit) =>
        typeof limit.provider === 'string' &&
        limit.provider.length > 0 &&
        limit.windowMinutes === 10080 &&
        limit.resetsAtMs > doc.observedAtMs,
    )
    .map((limit) => ({
      key: windowKey(limit),
      source: limit.provider,
      provider: PROVIDER_LABELS[limit.provider] ?? limit.provider,
      leftPercent: Math.round((100 - limit.usedPercent) * 10) / 10,
      elapsedPercent: limit.elapsedPercent,
      paceDeltaPoints: limit.paceDeltaPoints,
      resetsAtMs: limit.resetsAtMs,
      observedAtMs: limit.observedAtMs,
      reporters: limit.seats
        .map((id) => (seats.has(id) ? seatName(seats.get(id)!) : id.slice(0, 8)))
        .sort(),
      current: limit.seats.some((id) => seats.has(id) && !seats.get(id)!.retired),
      stale: limit.stale,
    }))
    .sort((a, b) => b.observedAtMs - a.observedAtMs || a.key.localeCompare(b.key));
}

/** One chart per exact source/window/reset, never borrowed by another window. */
export function historyCharts(doc: UsageDocument, zone?: string): SeriesChartOptions[] {
  return (doc.limitHistory ?? [])
    .filter((history) => history.points.length > 1)
    .map((history) => ({
      title: 'Weekly percent left',
      kind: 'line' as const,
      style: providerStyle(history.provider),
      x: history.points.map((point) => stamp(point.atMs, '7d', zone)),
      series: [
        {
          key: windowKey(history),
          label: 'left',
          values: history.points.map((point) => point.percentLeft),
        },
      ],
      reference: {
        label: 'even pace',
        values: history.points.map((point) =>
          Math.max(
            0,
            Math.min(
              100,
              (100 * (history.resetsAtMs - point.atMs)) / (history.windowMinutes * 60_000),
            ),
          ),
        ),
      },
      max: 100,
      format: (value: number) => `${Math.round(value)}%`,
    }));
}

export interface ProviderView extends AllowanceRow {
  style: number;
  chart: SeriesChartOptions | undefined;
}

export function providers(doc: UsageDocument, zone?: string): ProviderView[] {
  const charts = new Map(historyCharts(doc, zone).map((chart) => [chart.series[0]!.key, chart]));
  return allowance(doc).map((row) => ({
    ...row,
    style: providerStyle(row.source),
    chart: charts.get(row.key),
  }));
}

export interface ProviderGroup {
  source: string;
  label: string;
  primary: ProviderView | undefined;
  other: ProviderView[];
}

/** Most recent current-seat observation leads; retired-only windows remain in the disclosure. */
export function providerGroups(doc?: UsageDocument): ProviderGroup[] {
  const views = doc ? providers(doc) : [];
  const sources = [...new Set(['codex', 'claude', ...views.map((view) => view.source)])];
  return sources.map((source) => {
    const windows = views.filter((view) => view.source === source);
    const primary = windows.find((view) => view.current);
    return {
      source,
      label: PROVIDER_LABELS[source] ?? source,
      primary,
      other: windows.filter((view) => view !== primary),
    };
  });
}

/** Age against the owner's clock, never the browser's possibly different wall clock. */
export function observedAgo(observedAtMs: number, nowMs: number): string {
  const minutes = Math.max(0, Math.floor((nowMs - observedAtMs) / 60_000));
  return minutes < 1
    ? 'just now'
    : minutes < 60
      ? `${minutes}m ago`
      : minutes < 1440
        ? `${Math.floor(minutes / 60)}h ago`
        : `${Math.floor(minutes / 1440)}d ago`;
}

/** Days between two instants, for the page's "resets in" words. */
export function resetsIn(resetsAtMs: number, nowMs: number): string {
  const ms = resetsAtMs - nowMs;
  if (ms <= 0) return 'now';
  if (ms >= DAY) return `${Math.floor(ms / DAY)}d ${Math.floor((ms % DAY) / HOUR)}h`;
  return `${Math.max(1, Math.floor(ms / HOUR))}h`;
}
