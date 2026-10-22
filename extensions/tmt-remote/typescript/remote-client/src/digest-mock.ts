/**
 * MOCK: seeded sample usage shaped like the public report, for tests, the gallery and dev. The
 * product entry never imports this module (a test pins that), so the product page keeps its honest
 * empty state until #4325 serves the real read.
 */
import type {
  DigestSource,
  FlowPair,
  LimitHistory,
  LimitWindow,
  UsageDocument,
  UsagePoint,
  UsageRange,
  UsageSeat,
} from './digest-source.js';

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
/** A fixed clock: the sample never moves, so a screenshot or test is the same every run. */
export const MOCK_NOW_MS = 1_800_000_000_000;

function random(seed: number) {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 2 ** 32;
  };
}

const SEATS = [
  { id: '10000000-0000-4000-8000-000000000001', name: 'tmt-lead', line: 'claude', scale: 3 },
  { id: '10000000-0000-4000-8000-000000000002', name: 'core-1', line: 'gpt', scale: 5 },
  { id: '10000000-0000-4000-8000-000000000003', name: 'ux-1', line: 'claude', scale: 4 },
  { id: '10000000-0000-4000-8000-000000000004', name: 'feat-1', line: 'gpt', scale: 3 },
  { id: '10000000-0000-4000-8000-000000000005', name: 'remote-1', line: 'claude', scale: 2 },
  { id: '10000000-0000-4000-8000-000000000006', name: 'night-1', line: 'gpt', scale: 1 },
] as const;

function seat(index: number, range: UsageRange): UsageSeat {
  const spec = SEATS[index]!;
  const next = random(100 + index);
  const step = range === '24h' ? HOUR : DAY;
  const count = range === '24h' ? 24 : range === '7d' ? 7 : 30;
  const start = Math.floor(MOCK_NOW_MS / step) * step - count * step;
  const series: UsagePoint[] = [];
  for (let at = 0; at < count; at++) {
    // The last seat has no evidence before the second half: a partial seat, never a zero.
    if (index === 5 && at < count / 2) continue;
    const quiet = next() < 0.2;
    const input = quiet ? 0 : Math.round(next() * 90_000 * spec.scale * (step / HOUR) ** 0.5);
    const output = Math.round(input * 0.12);
    series.push({
      fromMs: start + at * step,
      toMs: start + (at + 1) * step,
      input,
      output,
      cached: Math.round(input * 0.6),
      complete: !(index === 5 && at === count / 2),
    });
  }
  const totals = series.reduce(
    (sum, point) => ({
      input: sum.input + point.input,
      output: sum.output + point.output,
      cached: sum.cached + point.cached,
    }),
    { input: 0, output: 0, cached: 0 },
  );
  return {
    id: spec.id,
    name: spec.name,
    retired: false,
    totals,
    lines: [spec.line],
    series,
    coverage: {
      coveredMs: series.length * step,
      windowMs: count * step,
      startsAtMs: series[0]?.fromMs ?? null,
      complete: series.length === count && series.every((point) => point.complete),
      gap: series.length !== count,
    },
  };
}

function flow(): FlowPair[] {
  const next = random(9);
  const pairs: FlowPair[] = [];
  for (const from of SEATS) {
    for (const to of SEATS) {
      if (from.id === to.id || next() < 0.4) continue;
      const party = (s: (typeof SEATS)[number]) => ({ id: s.id, name: s.name, retired: false });
      pairs.push({
        sender: party(from),
        recipient: party(to),
        kind: next() < 0.7 ? 'talk' : 'reply',
        count: 1 + Math.round(next() * 40),
      });
    }
  }
  pairs.push({
    sender: { id: SEATS[0].id, name: SEATS[0].name, retired: false },
    recipient: { id: SEATS[1].id, name: SEATS[1].name, retired: false },
    kind: 'ack',
    count: 99,
  });
  return pairs;
}

const limits: LimitWindow[] = [
  {
    windowMinutes: 10080,
    resetsAtMs: MOCK_NOW_MS + 40 * HOUR,
    usedPercent: 74,
    observedAtMs: MOCK_NOW_MS - 10 * 60_000,
    elapsedPercent: 76.19,
    paceDeltaPoints: -2.19,
    stale: false,
    seats: [SEATS[0].id, SEATS[2].id, SEATS[4].id],
  },
  {
    windowMinutes: 10080,
    resetsAtMs: MOCK_NOW_MS + 100 * HOUR,
    usedPercent: 45,
    observedAtMs: MOCK_NOW_MS - 20 * 60_000,
    elapsedPercent: 40.48,
    paceDeltaPoints: 4.52,
    stale: false,
    seats: [SEATS[1].id, SEATS[3].id],
  },
  {
    windowMinutes: 300,
    resetsAtMs: MOCK_NOW_MS + 2 * HOUR,
    usedPercent: 30,
    observedAtMs: MOCK_NOW_MS - 5 * 60_000,
    elapsedPercent: 60,
    paceDeltaPoints: -30,
    stale: false,
    seats: [SEATS[0].id],
  },
];

function limitHistory(): LimitHistory[] {
  return [
    { line: 'claude', left: 26, end: 40 },
    { line: 'gpt', left: 55, end: 100 },
  ].map(({ line, left, end }) => ({
    line,
    windowMinutes: 10080,
    resetsAtMs: MOCK_NOW_MS + end * HOUR,
    points: Array.from({ length: 8 * 6 }, (_, at) => ({
      atMs: MOCK_NOW_MS - (47 - at) * 4 * HOUR,
      percentLeft: Math.max(0, 100 - ((100 - left) * (at + 1)) / 48 - (at === 20 ? 2 : 0)),
    })).filter((_, at) => at !== 20 && at !== 21),
  }));
}

export function mockDocument(range: UsageRange): UsageDocument {
  return {
    version: 1,
    observedAtMs: MOCK_NOW_MS,
    since: range,
    usage: { seats: SEATS.map((_, index) => seat(index, range)), unavailable: [] },
    limits,
    limitsUnavailable: [],
    exchange: { sinceMs: MOCK_NOW_MS - DAY, observedAtMs: MOCK_NOW_MS, pairs: flow() },
    limitHistory: limitHistory(),
  };
}

export const mockDigestSource: DigestSource = {
  async report(range) {
    return { range, sample: true, document: mockDocument(range) };
  },
};
