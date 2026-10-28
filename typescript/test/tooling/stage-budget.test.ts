import { describe, expect, it } from 'vite-plus/test';
import { StageBudget } from '../e2e/stage-budget.js';

function clock() {
  let now = 1_000;
  return { now: () => now, advance: (ms: number) => (now += ms) };
}

describe('StageBudget', () => {
  it('gives a stage its cap, or the time left when that is smaller', async () => {
    const time = clock();
    const budget = new StageBudget({ totalMs: 15_000, cleanupReserveMs: 3_000, now: time.now });
    expect(budget.remainingMs()).toBe(12_000);
    const allowances: number[] = [];
    await budget.stage('first', (allowanceMs) => allowances.push(allowanceMs), { capMs: 4_000 });
    time.advance(9_000);
    await budget.stage('second', (allowanceMs) => allowances.push(allowanceMs), { capMs: 4_000 });
    await budget.stage('third', (allowanceMs) => allowances.push(allowanceMs));
    expect(allowances).toEqual([4_000, 3_000, 3_000]);
  });

  it('keeps the cleanup reserve out of every wait', async () => {
    const time = clock();
    const budget = new StageBudget({ totalMs: 15_000, cleanupReserveMs: 3_000, now: time.now });
    time.advance(11_000);
    expect(budget.remainingMs()).toBe(1_000);
    time.advance(5_000);
    expect(budget.remainingMs()).toBe(0);
  });

  it('names the stage that failed, the stages before it and the original cause', async () => {
    const time = clock();
    const budget = new StageBudget({ totalMs: 15_000, cleanupReserveMs: 3_000, now: time.now });
    await budget.stage('fixture setup', () => time.advance(2_000));
    await budget.stage('sample acquisition', () => time.advance(1_500));
    const cause = new Error('Timed out waiting for changed capture publishes.');
    const failure = await budget
      .stage(
        'changed-layout publication',
        () => {
          time.advance(4_000);
          throw cause;
        },
        { capMs: 4_000 }
      )
      .catch((error: Error) => error);
    expect(failure).toBeInstanceOf(Error);
    const message = (failure as Error).message;
    expect(message).toContain('Stage "changed-layout publication" failed after 4000 ms');
    expect(message).toContain('4000 ms allowance');
    expect(message).toContain(
      'fixture setup 2000 ms ok; sample acquisition 1500 ms ok; changed-layout publication 4000 ms FAILED'
    );
    expect(message).toContain('7500 of 15000 ms spent, 3000 ms reserved for cleanup');
    expect(message).toContain(cause.message);
    expect((failure as Error).cause).toBe(cause);
  });

  it('fails a stage that has no time left without running it', async () => {
    const time = clock();
    const budget = new StageBudget({ totalMs: 15_000, cleanupReserveMs: 3_000, now: time.now });
    time.advance(12_000);
    let ran = false;
    await expect(
      budget.stage('late stage', () => {
        ran = true;
      })
    ).rejects.toThrow('Stage "late stage" failed after 0 ms of its 0 ms allowance');
    expect(ran).toBe(false);
  });

  it('reports a stage that is still running, as the framework timeout would leave it', async () => {
    const time = clock();
    const budget = new StageBudget({ totalMs: 15_000, cleanupReserveMs: 3_000, now: time.now });
    await budget.stage('fixture setup', () => time.advance(1_000));
    const stalled = budget.stage('changed-layout publication', () => new Promise<void>(() => {}));
    time.advance(11_000);
    expect(budget.evidence()).toBe(
      'Stages: fixture setup 1000 ms ok; changed-layout publication still running after 11000 ms. ' +
        '12000 of 15000 ms spent, 3000 ms reserved for cleanup.'
    );
    void stalled;
  });

  it('refuses a reserve that leaves no time for stages', () => {
    expect(() => new StageBudget({ totalMs: 3_000, cleanupReserveMs: 3_000 })).toThrow(
      'The cleanup reserve must be smaller than the scenario timeout.'
    );
  });
});
