import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'vite-plus/test';
import {
  ACTIVE_SECONDS_PER_DAY,
  BUDGET_EXHAUSTED,
  BudgetModel,
  BudgetModelError,
  READS_PER_DAY,
  REFUSE_PERCENT,
  WARN_PERCENT,
  WRITES_PER_DAY,
  classifyProviderExhausted,
  decide,
  exhaustedIsCurrent,
  newUsage,
  pacificDay,
  recordedUsage,
  usageAt,
} from '../src/budget.js';

// The Rust guard's vectors, produced by an independent Python script: both implementations
// must satisfy the same file.
const fixtures = (name: string) =>
  JSON.parse(
    readFileSync(
      new URL(`../../../rust/tmt-remote/tests/fixtures/firestore_budget/${name}`, import.meta.url),
      'utf8',
    ),
  );
const vectors = fixtures('vectors.json');
const model = (members: number, writers: number) => BudgetModel.create(members, writers);
const refused = (members: number, writers: number) => {
  try {
    BudgetModel.create(members, writers);
  } catch (error) {
    assert.ok(error instanceof BudgetModelError);
    return error.code;
  }
  return undefined;
};

test('the arithmetic matches the independent vectors', () => {
  for (const row of vectors.append) {
    const m = model(row.members, row.writers);
    assert.deepEqual(
      [
        m.readsPerAppend(),
        m.dailyAppendLimit(),
        m.share(),
        m.warnAt(),
        m.refuseAt(),
        m.minFlushIntervalMs(),
      ],
      [
        row.readsPerAppend,
        row.dailyAppendLimit,
        row.share,
        row.warnAt,
        row.refuseAt,
        row.minFlushIntervalMs,
      ],
      JSON.stringify(row),
    );
    // The optimistic figure in the vectors is documentation: the guard is pessimistic.
    assert.ok(row.optimisticReadsPerAppend <= m.readsPerAppend());
  }
  assert.ok(vectors.assess.length > 50);
  for (const row of vectors.assess) {
    assert.equal(
      model(row.members, row.writers).assess(row.used),
      row.verdict,
      JSON.stringify(row),
    );
  }
  assert.equal(vectors.pacific.length, 18);
  for (const row of vectors.pacific) {
    assert.deepEqual(
      pacificDay(row.nowMs),
      { day: row.day, resetAtMs: row.resetAtMs },
      JSON.stringify(row),
    );
  }
});

test('the constants equal the numbers read from the official pages', () => {
  const published = fixtures('limits-member.json');
  assert.equal(READS_PER_DAY, published.limits.readsPerDay);
  assert.equal(WRITES_PER_DAY, published.limits.writesPerDay);
  assert.equal(WARN_PERCENT, published.guard.warnPercent);
  assert.equal(REFUSE_PERCENT, published.guard.refusePercent);
  assert.equal(ACTIVE_SECONDS_PER_DAY, 7200);
});

test('a hand-worked row and the named thresholds', () => {
  // Five members, three writers: 4 + 4 x (1 + 2) = 16 reads per append; 50,000 / 16 = 3,125
  // appends; a writer's share is 1,041, warned at 728 (70%) and refused at 936 (90%).
  const m = model(5, 3);
  assert.deepEqual(
    [m.readsPerAppend(), m.dailyAppendLimit(), m.share(), m.warnAt(), m.refuseAt()],
    [16, 3125, 1041, 728, 936],
  );
  assert.deepEqual(
    [727, 728, 935, 936].map((n) => m.assess(n)),
    ['ok', 'warn', 'warn', 'refuse'],
  );
  assert.equal(model(1, 1).readsPerAppend(), 4);
  for (const members of [1, 2, 10, 100]) {
    const alone = model(members, 1);
    assert.ok(alone.dailyAppendLimit() * alone.readsPerAppend() <= READS_PER_DAY);
  }
});

test('impossible models are refused', () => {
  const impossible: [number, number][] = [
    [0, 0],
    [1, 0],
    [0, 1],
    [2, 3],
    [10_001, 1],
    [1.5, 1],
    [2, 1.5],
    [Number.NaN, 1],
    [-1, 1],
  ];
  for (const [members, writers] of impossible)
    assert.equal(refused(members, writers), 'out_of_range', `${members} ${writers}`);
});

test('a page the free plan cannot host is refused when the model is built', () => {
  assert.ok(vectors.hosting.length >= 5);
  for (const row of vectors.hosting) {
    if (row.accepted) {
      const m = model(row.members, row.writers);
      assert.ok(m.refuseAt() >= 1 && m.assess(0) === 'ok', JSON.stringify(row));
    } else assert.equal(refused(row.members, row.writers), 'free_plan_cannot_host');
  }
  // Hand-checked for one writer: 3N + 1 reads per append, so 50,000 / (3N + 1) is 2 appends at N = 8,333.
  assert.ok(model(8333, 1));
  assert.equal(refused(8334, 1), 'free_plan_cannot_host');
});

const T = 1_800_000_000_000; // an instant in Pacific winter time
test('the guard warns then refuses before the limit and the refusal is recoverable', () => {
  const m = model(5, 3);
  const { resetAtMs } = pacificDay(T);
  const usage = (appends: number) => ({ ...newUsage(T), appends });
  assert.deepEqual(decide(m, usage(0), T), { decision: 'allow' });
  assert.deepEqual(decide(m, usage(727), T), { decision: 'allow' });
  assert.deepEqual(decide(m, usage(728), T), { decision: 'warn' });
  assert.deepEqual(decide(m, usage(935), T), { decision: 'warn' });
  const refusal = decide(m, usage(936), T);
  assert.deepEqual(refusal, { decision: 'refuse', resetAtMs, retryAfterMs: resetAtMs - T });
  // Refusal comes at 90% of the share, strictly before the share and the page allowance.
  assert.ok(m.refuseAt() < m.share() && m.share() * 3 <= m.dailyAppendLimit());
  // Not silent and not permanent: after the reset the same usage is a fresh day.
  assert.deepEqual(decide(m, usage(936), resetAtMs), { decision: 'allow' });
  assert.deepEqual(decide(m, usage(936), resetAtMs - 1), {
    decision: 'refuse',
    resetAtMs,
    retryAfterMs: 1,
  });
  // Counting rolls over by itself.
  const sent = recordedUsage(usage(5), T);
  assert.equal(sent.appends, 6);
  assert.equal(recordedUsage(sent, resetAtMs).appends, 1);
  assert.equal(usageAt(sent, resetAtMs).day, sent.day + 1);
});

test('a provider refusal after a possible write is unknown, never the budget error', () => {
  const { resetAtMs } = pacificDay(T);
  assert.equal(BUDGET_EXHAUSTED, 'REMOTE_BUDGET_EXHAUSTED');
  // A read cannot have changed anything: a refusal with the reset time.
  const read = classifyProviderExhausted({ mutation: false }, T);
  assert.deepEqual(read.outcome, { outcome: 'refused', resetAtMs, retryAfterMs: resetAtMs - T });
  // A write may have been applied: unknown, with no reset promise and no budget code.
  const write = classifyProviderExhausted({ mutation: true }, T);
  assert.deepEqual(write.outcome, { outcome: 'unknown' });
  assert.deepEqual(read.evidence, write.evidence);
  // Both record `exhausted` evidence, which says nothing once the quota has reset.
  assert.deepEqual(read.evidence, { day: pacificDay(T).day, resetAtMs });
  assert.ok(
    exhaustedIsCurrent(read.evidence, T) && exhaustedIsCurrent(read.evidence, resetAtMs - 1),
  );
  assert.ok(!exhaustedIsCurrent(read.evidence, resetAtMs));
});
