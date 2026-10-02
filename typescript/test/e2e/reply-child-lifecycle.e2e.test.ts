import fs from 'node:fs';
import { describe, expect, it } from 'vitest';
import { E2EFixture, withE2EFixture } from './harness.js';

function processGroupIsRunning(pid: number): boolean {
  try {
    process.kill(-pid, 0);
    return true;
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ESRCH') return false;
    throw error;
  }
}

describe('durable reply child lifecycle', { concurrent: false }, () => {
  it('cleans a real reply child held before stdin EOF when the mock is SIGKILLed', async () => {
    let failedFixture: E2EFixture | undefined;
    let childPid = 0;
    try {
      await expect(
        withE2EFixture(
          async (fixture) => {
            failedFixture = fixture;
            const sent = await fixture.runJsonCli([
              'talk',
              fixture.pane,
              'held reply child',
              '--no-preamble',
              '--detach',
            ]);
            expect(sent.code).toBe(0);
            const child = await fixture.waitForEvent(
              (event) => event.event === 'child-start' && event.requestId === sent.json?.requestId
            );
            childPid = child.childPid ?? 0;
            expect(childPid).toBeGreaterThan(0);
            expect(processGroupIsRunning(childPid)).toBe(true);
            process.kill(-childPid, 'SIGSTOP');
            process.kill(fixture.panePid, 'SIGKILL');
            await fixture.waitFor(() => !fixture.mockProcessIsRunning(), 1_000, 'mock SIGKILL');
            expect(processGroupIsRunning(childPid)).toBe(true);
            throw new Error('simulated scenario failure after mock SIGKILL');
          },
          { holdReplyEof: true }
        )
      ).rejects.toThrow('simulated scenario failure after mock SIGKILL');
      expect(failedFixture).toBeDefined();
      expect(processGroupIsRunning(childPid)).toBe(false);
      expect(failedFixture?.serverIsRunning()).toBe(false);
      expect(failedFixture?.mockProcessIsRunning()).toBe(false);
      expect(failedFixture && fs.existsSync(failedFixture.root)).toBe(false);
    } finally {
      // Safety cleanup follows the assertions, so it cannot hide a fixture leak.
      if (childPid > 0 && processGroupIsRunning(childPid)) process.kill(-childPid, 'SIGKILL');
    }
  });
});
