import { describe, expect, it } from 'vitest';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture } from './harness.js';

interface StatusResult {
  identityId: string;
  status: null | {
    activity: string;
    mood: string | null;
    updatedAtMs: number;
    expiresAtMs: number;
    stale: boolean;
  };
}

describe('self-reported status from verified tmux callers', { concurrent: false }, () => {
  it('uses the bound Contractor UUID and preserves its status through promotion', async () => {
    await withE2EFixture(async (fixture) => {
      const caller = expectJsonResult(
        await fixture.runJsonCli<{ id: string }>(['name', 'Contractor'])
      );
      const set = expectJsonResult(
        await fixture.runJsonCli<StatusResult>([
          'identity',
          'status',
          'set',
          'Reviewing',
          '--mood',
          'focused',
          '--for',
          '60m',
        ])
      );
      expect(set.identityId).toBe(caller.id);
      expect(set.status).toMatchObject({ activity: 'Reviewing', mood: 'focused', stale: false });
      const temporary = expectJsonResult(
        await fixture.runJsonCli<{ identity: { id: string; lifetime: string } }>([
          'identity',
          'show',
          'Contractor',
        ])
      );
      expect(temporary.identity).toMatchObject({ id: caller.id, lifetime: 'temporary' });
      const saved = expectJsonResult(
        await fixture.runJsonCli<{ identity: { id: string; lifetime: string } }>([
          'identity',
          'create',
          'Contractor',
        ])
      );
      expect(saved.identity).toMatchObject({ id: caller.id, lifetime: 'saved' });
      const outside = expectJsonResult(
        await fixture.runJsonCli<StatusResult>(
          ['identity', 'status', 'show', '--identity', 'Contractor'],
          { outsideTmux: true }
        )
      );
      expect(outside).toEqual(set);
      expectJsonResult(await fixture.runJsonCli(['identity', 'status', 'clear']));
      expect(
        expectJsonResult(await fixture.runJsonCli<StatusResult>(['identity', 'status', 'show']))
          .status
      ).toBeNull();
    });
  });
});
