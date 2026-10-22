import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import { limitHistoryFrom, reportFromAnswers } from '../src/digest-source.js';
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

it('turns the digest vector into the page report, keeping the document as it came', () => {
  const report = reportFromAnswers('7d', answerOf('digest: the usage report passes'), notSampled);
  expect(report).toMatchObject({ range: '7d', sample: false });
  expect(report.unavailable).toBeUndefined();
  expect(report.document).toMatchObject({ since: '7d', observedAtMs: 1792635300000 });
  // No kept samples is the owner's answer, not an empty history: the chart is simply left out.
  expect(report.document!.limitHistory).toBeUndefined();
});

it('says one plain sentence for each way the digest read is unavailable', () => {
  const limits = notSampled;
  const reasons = ['digest: the Digest extension is absent', 'digest: the report command fails'];
  const texts = reasons.map((name) => reportFromAnswers('24h', answerOf(name), limits).unavailable);
  expect(texts[0]).toContain('tmt extension install digest');
  expect(texts[1]).toContain('tmt digest usage report');
  expect(reportFromAnswers('24h', { state: 'no-session' }, limits).unavailable).toContain(
    'Pair it from Remote',
  );
  expect(reportFromAnswers('24h', { state: 'unreachable' }, limits).unavailable).toContain(
    'tries again shortly',
  );
  for (const text of texts) expect(text).not.toMatch(/sample|mock/i);
});

it('never shows a report for another range than the one asked for', () => {
  const report = reportFromAnswers('24h', answerOf('digest: the usage report passes'), notSampled);
  expect(report.document).toBeUndefined();
  expect(report.unavailable).toContain('not available right now');
});

it('maps the kept weekly samples to the history of the newest window, by model family', () => {
  const history = limitHistoryFrom(kept as never)!;
  expect(history).toEqual([
    {
      line: 'gpt',
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
  expect(limitHistoryFrom(two)![0]).toMatchObject({
    line: 'claude',
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
  expect(history.map((entry) => [entry.line, entry.points.length])).toEqual([
    ['claude', 1],
    ['gpt', 1],
  ]);
  expect(limitHistoryFrom(answer('not a list'))).toBeUndefined();
  expect(limitHistoryFrom(answer(undefined))).toBeUndefined();
});
