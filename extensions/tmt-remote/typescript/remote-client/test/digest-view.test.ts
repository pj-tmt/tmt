import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import { mockDocument } from '../src/digest-mock.js';
import type { UsageDocument } from '../src/digest-source.js';
import {
  allowance,
  exchange,
  historyCharts,
  providers,
  providerGroups,
  windowKey,
  observedAgo,
  ranking,
  rankRows,
  resetsIn,
  seatName,
  tokenSeries,
} from '../src/digest-view.js';

const day = mockDocument('24h');

it('shapes every mock range like the public report: hourly for 24h, daily after', () => {
  expect(mockDocument('24h').usage.seats[0]!.series).toHaveLength(24);
  expect(mockDocument('7d').usage.seats[0]!.series).toHaveLength(7);
  expect(mockDocument('30d').usage.seats[0]!.series).toHaveLength(30);
  // Deterministic: a screenshot and a test see the same bytes on every run.
  expect(JSON.stringify(mockDocument('24h'))).toBe(JSON.stringify(mockDocument('24h')));
});

it('pins the mock to the digest usage vector: same keys at every level the page reads', () => {
  const vector = JSON.parse(
    readFileSync(
      new URL('../../../../tmt-digest/contracts/vectors/usage-report-v1.json', import.meta.url),
      'utf8',
    ),
  ).cases[0].expected;
  const keys = (value: object) => Object.keys(value).sort();
  const seat = vector.usage.seats[0];
  const mock = day.usage.seats[0]!;
  for (const key of ['id', 'name', 'retired', 'totals', 'lines', 'series', 'coverage']) {
    expect(keys(mock), key).toContain(key);
    expect(keys(seat), key).toContain(key);
  }
  expect(keys(mock.series[0]!)).toEqual(
    expect.arrayContaining(['fromMs', 'toMs', 'input', 'output', 'cached', 'complete']),
  );
  expect(keys(mock.coverage)).toEqual(keys(seat.coverage));
  expect(keys(day.limits[0]!)).toEqual(keys(vector.limits[0]));
  expect(keys(day.exchange!.pairs[0]!)).toEqual(
    expect.arrayContaining(['sender', 'recipient', 'kind', 'count']),
  );
  expect(keys(vector.exchange.pairs[0])).toEqual(
    expect.arrayContaining(['sender', 'recipient', 'kind', 'count']),
  );
  expect(day.version).toBe(vector.version);
});

it('token series put every seat on the union of buckets and keep an unobserved bucket null', () => {
  const data = tokenSeries(day, 'UTC');
  expect(data.x).toHaveLength(24);
  expect(data.x[0]).toMatch(/^\d\d:00$/);
  const last = data.layers.at(-1)!;
  expect(last.label).toBe('night-1');
  expect(last.values.slice(0, 12).every((value) => value === null)).toBe(true);
  expect(last.values.slice(12).some((value) => value !== null)).toBe(true);
  expect(data.partial).toBe(true);
  // Daily ranges label by date.
  expect(tokenSeries(mockDocument('7d'), 'UTC').x[0]).toMatch(/^\d\d-\d\d$/);
});

it('tokens are input plus output: cached input is never added again', () => {
  const doc: UsageDocument = {
    ...day,
    usage: {
      seats: [
        {
          ...day.usage.seats[0]!,
          totals: { input: 1000, output: 200, cached: 600 },
          series: [{ fromMs: 0, toMs: 1, input: 1000, output: 200, cached: 600, complete: true }],
        },
      ],
      unavailable: [],
    },
  };
  expect(rankRows(doc)[0]!.tokens).toBe(1200);
  expect(tokenSeries(doc).layers[0]!.values).toEqual([1200]);
});

it('ranks seats by tokens, shares sum to about 100 and a partial seat says so', () => {
  const rows = rankRows(day);
  const tokens = rows.map((row) => row.tokens);
  expect(tokens).toEqual(tokens.slice().sort((a, b) => b - a));
  expect(rows.reduce((sum, row) => sum + row.share, 0)).toBeGreaterThanOrEqual(98);
  expect(rows.find((row) => row.name === 'night-1')!.partial).toBe(true);
  expect(rows.find((row) => row.name === 'tmt-lead')!.partial).toBe(false);
  expect(seatName({ id: '12345678-aaaa', name: null, retired: true })).toBe('12345678 (retired)');
});

it('a large team ranks the top 8 and folds the rest into one Other row', () => {
  const team = rankRows(mockDocument('24h', { seats: 24 }));
  expect(team).toHaveLength(24);
  const view = ranking(team);
  expect(view.top.map((row) => row.id)).toEqual(team.slice(0, 8).map((row) => row.id));
  const rest = team.slice(8).reduce((sum, row) => sum + row.tokens, 0);
  const total = team.reduce((sum, row) => sum + row.tokens, 0);
  expect(view.other).toEqual({ seats: 16, tokens: rest, share: Math.round((rest / total) * 100) });
  expect(view.partial).toBe(true);
  // A team of 8 or fewer folds nothing; a team with full evidence is not partial.
  expect(ranking(rankRows(day)).other).toBeNull();
  expect(ranking(team.map((row) => ({ ...row, partial: false }))).partial).toBe(false);
});

it('a one-provider sample has only the OpenAI weekly window and history', () => {
  const one = mockDocument('24h', { oneProvider: true });
  expect(providers(one).map((view) => view.provider)).toEqual(['OpenAI']);
  expect(one.limitHistory!.map((history) => history.provider)).toEqual(['codex']);
});

it('exchange counts talk and reply only, ranks by traffic and keeps a known none at zero', () => {
  const view = exchange(day)!;
  expect(view.participants).toHaveLength(day.usage.seats.length);
  const [a, b] = view.participants;
  const total = (id: string) =>
    view.participants.reduce(
      (sum, other) => sum + (view.value(id, other.id) ?? 0) + (view.value(other.id, id) ?? 0),
      0,
    );
  expect(total(a!.id)).toBeGreaterThanOrEqual(total(b!.id) - 0);
  // The mock's 99 acks from tmt-lead to core-1 are not messages.
  const lead = day.usage.seats[0]!.id;
  const core = day.usage.seats[1]!.id;
  const counted = day
    .exchange!.pairs.filter(
      (pair) => pair.sender.id === lead && pair.recipient.id === core && pair.kind !== 'ack',
    )
    .reduce((sum, pair) => sum + pair.count, 0);
  const all = exchange(day)!;
  expect(all.value(lead, core)).toBe(counted);
  expect(all.value(lead, core)).toBeLessThan(99);
  expect(exchange({ ...day, exchange: null })).toBeNull();
});

it('exchange retains every seat instead of silently keeping only the busiest eight', () => {
  const document = mockDocument('24h', { seats: 24 });
  const view = exchange(document)!;
  expect(view.participants.map((person) => person.id).sort()).toEqual(
    document.usage.seats.map((seat) => seat.id).sort(),
  );
});

it('allowance lists the weekly windows only, left = 100 - used, with the provider named', () => {
  const rows = allowance(day);
  expect(rows.map((row) => [row.provider, row.leftPercent])).toEqual([
    ['OpenAI', 55],
    ['OpenAI', 26],
    ['OpenAI', 62],
  ]);
  const stale = allowance({
    ...day,
    limits: [{ ...day.limits[0]!, stale: true, elapsedPercent: null, paceDeltaPoints: null }],
  });
  expect(stale[0]).toMatchObject({ stale: true, elapsedPercent: null, paceDeltaPoints: null });
  expect(resetsIn(day.observedAtMs + 40 * 3_600_000, day.observedAtMs)).toBe('1d 16h');
  expect(resetsIn(day.observedAtMs + 90 * 60_000, day.observedAtMs)).toBe('1h');
  expect(resetsIn(day.observedAtMs - 1, day.observedAtMs)).toBe('now');
});

it('history charts exist only when the history field is served', () => {
  const charts = historyCharts(day, 'UTC');
  expect(charts).toHaveLength(3);
  expect(charts[0]!.series[0]!.values.length).toBe(charts[0]!.x.length);
  expect(charts[0]!.reference!.values.every((v) => v !== null && v >= 0 && v <= 100)).toBe(true);
  expect(historyCharts({ ...day, limitHistory: undefined })).toEqual([]);
});

it('providers pair each weekly window with its history chart and a stable hue', () => {
  const views = providers(day, 'UTC');
  expect(views.map((view) => [view.provider, view.style])).toEqual([
    ['OpenAI', 2],
    ['OpenAI', 2],
    ['OpenAI', 2],
  ]);
  expect(views.every((view) => view.chart?.series[0]!.key === view.key)).toBe(true);
  expect(views[0]!.chart!.style).toBe(2);
  expect(providers({ ...day, limitHistory: undefined }).every((view) => !view.chart)).toBe(true);
});

it('the product entry never pulls in the sample: no import of the mock from page code', () => {
  for (const file of ['digest-page', 'digest-source', 'digest-view', 'shell-app', 'browser']) {
    const source = readFileSync(new URL(`../src/${file}.ts`, import.meta.url), 'utf8');
    expect(source, file).not.toMatch(/from '\.\/digest-mock/);
  }
});

it('titles limits from their source even on Claude-model seats, and never infers a missing provider', () => {
  const doc = { ...day, limits: [{ ...day.limits[0]! }] };
  expect(doc.usage.seats.find((seat) => seat.id === doc.limits[0]!.seats[0])!.lines).toEqual([
    'claude',
  ]);
  expect(allowance(doc)[0]!.provider).toBe('OpenAI');
  expect(
    allowance({ ...doc, limits: [{ ...doc.limits[0]!, provider: undefined as never }] }),
  ).toEqual([]);
  expect(
    allowance({ ...doc, limits: [{ ...doc.limits[0]!, provider: 'future-source' }] })[0]!.provider,
  ).toBe('future-source');
  expect(providerGroups(doc).find((group) => group.source === 'claude')!.primary).toBeUndefined();
});

it('chooses the latest current-seat window and drops resets already passed', () => {
  const [older, current] = day.limits;
  const retired = { ...day.usage.seats[0]!, retired: true };
  const doc = {
    ...day,
    usage: { ...day.usage, seats: [retired, ...day.usage.seats.slice(1)] },
    limits: [
      { ...older!, seats: [retired.id], observedAtMs: day.observedAtMs - 1 },
      { ...current!, seats: [...current!.seats, retired.id] },
      { ...older!, resetsAtMs: day.observedAtMs },
    ],
  };
  const group = providerGroups(doc)[0]!;
  expect(group.primary!.leftPercent).toBe(55);
  expect(group.primary!.reporters).toContain('tmt-lead (retired)');
  expect(group.other).toHaveLength(1);
  expect(group.other[0]!.reporters).toEqual(['tmt-lead (retired)']);
  expect(
    providerGroups({
      ...doc,
      usage: {
        seats: [],
        unavailable: [
          { id: current!.seats[0]!, name: 'unread', retired: false, reason: 'not_sampled' },
        ],
      },
    })[0]!.primary,
  ).toBeDefined();
  expect(providerGroups({ ...doc, limits: [doc.limits[0]!] })[0]!.primary).toBeUndefined();
});

it('matches history only to the same source, duration and reset, keeping multiple windows distinct', () => {
  const views = providers(day, 'UTC');
  expect(new Set(views.map((view) => view.key)).size).toBe(3);
  for (const view of views) {
    expect(view.chart!.series[0]!.key).toBe(view.key);
    expect(view.chart!.series[0]!.values.at(-1)).toBe(view.leftPercent);
  }
  const first = day.limitHistory![0]!;
  for (const mismatched of [
    { ...first, provider: 'claude' },
    { ...first, windowMinutes: 300 },
    { ...first, resetsAtMs: first.resetsAtMs + 1 },
  ])
    expect(providers({ ...day, limitHistory: [mismatched] }).every((view) => !view.chart)).toBe(
      true,
    );
  expect(windowKey({ provider: 'codex', windowMinutes: 10080, resetsAtMs: 1 })).not.toBe(
    windowKey({ provider: 'codex', windowMinutes: 300, resetsAtMs: 1 }),
  );
  expect(observedAgo(day.observedAtMs - 600_000, day.observedAtMs)).toBe('10m ago');
  expect(observedAgo(day.observedAtMs + 1, day.observedAtMs)).toBe('just now');
});
