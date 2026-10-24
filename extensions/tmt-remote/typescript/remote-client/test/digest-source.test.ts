import { readFileSync } from 'node:fs';
import { expect, it, vi } from 'vite-plus/test';
import {
  allowanceFromAnswers,
  createDigestSource,
  limitHistoryFrom,
  reportFromAnswers,
} from '../src/digest-source.js';
import { ownerAnswer } from '../src/owner-reads.js';

interface Vectors {
  cases: { name: string; target: string; expect: Record<string, unknown> }[];
}
const vectors = JSON.parse(
  readFileSync(new URL('../../../../../contracts/owner-reads-v1.json', import.meta.url), 'utf8'),
) as Vectors;
const answerOf = (name: string) => {
  const found = vectors.cases.find((vector) => vector.name.startsWith(name));
  if (!found) throw new Error(`no vector: ${name}`);
  return ownerAnswer<never>({ ...found.expect, observedAtMs: 1792635300000 });
};
const notSampled = answerOf('limits: no kept samples');
const kept = answerOf('limits: the kept weekly samples');
const weekly = answerOf('digest: the usage report passes');
const snapshot = allowanceFromAnswers(weekly, notSampled);
const read = vi.hoisted(() => vi.fn());
vi.mock('../src/owner-reads.js', async (original) => ({
  ...(await original<typeof import('../src/owner-reads.js')>()),
  ownerRead: read,
}));

it('turns the digest vector into the page report, keeping the document as it came', () => {
  const report = reportFromAnswers('7d', answerOf('digest: the usage report passes'), snapshot);
  expect(report).toMatchObject({ range: '7d', sample: false });
  expect(report.unavailable).toBeUndefined();
  expect(report.document).toMatchObject({ since: '7d', observedAtMs: 1792635300000 });
  // No kept samples is the owner's answer, not an empty history: the chart is simply left out.
  expect(report.document!.limitHistory).toBeUndefined();
});

it('says one plain sentence for each way the digest read is unavailable', () => {
  const reasons = ['digest: the Digest extension is absent', 'digest: the report command fails'];
  const texts = reasons.map(
    (name) => reportFromAnswers('24h', answerOf(name), snapshot).unavailable,
  );
  expect(texts[0]).toContain('tmt extension install digest');
  expect(texts[1]).toContain('tmt digest usage report');
  expect(reportFromAnswers('24h', { state: 'no-session' }, snapshot).unavailable).toContain(
    'Pair it from Remote',
  );
  expect(reportFromAnswers('24h', { state: 'unreachable' }, snapshot).unavailable).toContain(
    'tries again shortly',
  );
  for (const text of texts) expect(text).not.toMatch(/sample|mock/i);
});

it('never shows a report for another range than the one asked for', () => {
  const report = reportFromAnswers('24h', answerOf('digest: the usage report passes'), snapshot);
  expect(report.document).toBeUndefined();
  expect(report.unavailable).toContain('not available right now');
});

it('maps the kept weekly samples to the history of each reset, by source provider', () => {
  const history = limitHistoryFrom(kept as never)!;
  expect(history).toEqual([
    {
      provider: 'codex',
      windowMinutes: 10080,
      resetsAtMs: 1792500000000,
      points: [
        { atMs: 1792000000000, percentLeft: 80.5 },
        { atMs: 1792100000000, percentLeft: 70.25 },
      ],
    },
  ]);
  // Points of an earlier window (another reset time) are not drawn on this window's line.
  const two = {
    state: 'ok' as const,
    observedAtMs: 1,
    payload: {
      limitHistory: [
        {
          provider: 'claude',
          windowMinutes: 10080,
          points: [
            [1, 90, 100],
            [2, 10, 100],
            [3, 95, 200],
            [4, 80, 200],
          ] as const,
        },
      ],
    },
  };
  expect(limitHistoryFrom(two)![1]).toMatchObject({
    provider: 'claude',
    resetsAtMs: 200,
    points: [
      { atMs: 3, percentLeft: 95 },
      { atMs: 4, percentLeft: 80 },
    ],
  });
  expect(limitHistoryFrom({ state: 'unreachable' })).toBeUndefined();
  expect(limitHistoryFrom(notSampled as never)).toBeUndefined();
});

it('reads each history entry as data: a malformed one shows no history for its provider', () => {
  const ok = { provider: 'claude', windowMinutes: 10080, points: [[1, 50, 100]] };
  const answer = (limitHistory: unknown) => ({
    state: 'ok' as const,
    observedAtMs: 1,
    payload: { limitHistory } as never,
  });
  const history = limitHistoryFrom(
    answer([
      ok,
      { provider: 'codex', windowMinutes: 10080, points: 'not points' },
      { provider: 7, windowMinutes: 10080, points: [[1, 2, 3]] },
      { provider: 'codex', windowMinutes: Number.NaN, points: [[1, 2, 3]] },
      null,
      { provider: 'codex', windowMinutes: 10080, points: [[1, 'x', 3], [2, 40, 100], 'bad'] },
    ]),
  )!;
  expect(history.map((entry) => [entry.provider, entry.points.length])).toEqual([
    ['claude', 1],
    ['codex', 1],
  ]);
  expect(limitHistoryFrom(answer('not a list'))).toBeUndefined();
  expect(limitHistoryFrom(answer(undefined))).toBeUndefined();
});

it('reads fixed weekly allowance for every range, sharing the selected 7d read and abort signal', async () => {
  const { mockDocument } = await import('../src/digest-mock.js');
  const signal = new AbortController().signal;
  for (const range of ['24h', '7d', '30d'] as const) {
    read.mockReset();
    read.mockImplementation(async (target: string) =>
      target === 'limits'
        ? kept
        : {
            state: 'ok',
            observedAtMs: 1,
            payload: {
              since: target.slice(7),
              report: mockDocument(target.slice(7) as typeof range),
            },
          },
    );
    const report = await createDigestSource().report(range, signal);
    expect(report.document!.since).toBe(range);
    expect(report.allowance!.since).toBe('7d');
    expect(report.allowance!.limits).toEqual(mockDocument('7d').limits);
    expect(read.mock.calls.filter(([target]) => target === 'digest/7d')).toHaveLength(1);
    expect(read).toHaveBeenCalledTimes(range === '7d' ? 2 : 3);
    expect(read.mock.calls.every(([, actual]) => actual === signal)).toBe(true);
  }
});

it('does not substitute the selected range when the fixed allowance read fails or is miscorrelated', () => {
  for (const answer of [
    { state: 'unreachable' } as const,
    { state: 'ok', observedAtMs: 1, payload: { since: '24h', report: {} } } as never,
  ]) {
    const report = reportFromAnswers('7d', weekly, allowanceFromAnswers(answer, kept));
    expect(report.document).toBeDefined();
    expect(report.allowance).toBeUndefined();
    expect(report.allowanceUnavailable).toContain('Allowance could not be read');
  }
});

it('reuses one weekly read across switches, including in-flight requests, and refreshes at the mount cadence', async () => {
  const { mockDocument } = await import('../src/digest-mock.js');
  read.mockReset();
  let release!: () => void;
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  read.mockImplementation(async (target: string) => {
    if (target === 'limits') return kept;
    if (target === 'digest/7d') await pending;
    return {
      state: 'ok',
      observedAtMs: 1,
      payload: { since: target.slice(7), report: mockDocument(target.slice(7) as '7d') },
    };
  });
  const source = createDigestSource();
  const first = source.report('24h');
  const second = source.report('30d');
  const third = source.report('7d');
  expect(read.mock.calls.map(([target]) => target)).toEqual([
    'digest/24h',
    'digest/7d',
    'limits',
    'digest/30d',
  ]);
  release();
  const results = await Promise.all([first, second, third]);
  expect(results.every((report) => report.allowance === results[0]!.allowance)).toBe(true);
  read.mockClear();
  await source.report('24h');
  expect(read.mock.calls.map(([target]) => target)).toEqual(['digest/24h']);
  read.mockClear();
  await source.report('7d');
  expect(read).not.toHaveBeenCalled();
  await source.report('30d', undefined, true);
  expect(read.mock.calls.map(([target]) => target)).toEqual(['digest/30d', 'digest/7d', 'limits']);
  read.mockReset();
  read.mockResolvedValue({ state: 'unreachable' });
  const failedSource = createDigestSource();
  await failedSource.report('24h');
  read.mockClear();
  const failed = await failedSource.report('30d');
  expect(read.mock.calls.map(([target]) => target)).toEqual(['digest/30d']);
  expect(failed.allowanceUnavailable).toContain('Allowance could not be read');
  read.mockClear();
  await createDigestSource().report('24h');
  expect(read.mock.calls.map(([target]) => target)).toEqual(['digest/24h', 'digest/7d', 'limits']);
});
