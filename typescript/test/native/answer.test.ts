import { existsSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { expectError, parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { installTmuxTripwire } from './tmux-tripwire.js';

type Sandbox = Parameters<typeof runCli>[0];

async function json(sandbox: Sandbox, args: string[], stdin?: string) {
  const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 5_000, stdin });
  expect(result.status).toBe(0);
  return parseWholeStdout(result);
}

async function ask(sandbox: Sandbox, from: string, text: string) {
  const queued = await json(sandbox, [
    'talk',
    'ben',
    text,
    '--inbox',
    '--identity',
    from,
    '--detach',
  ]);
  return queued.requestId as string;
}

describe('tmt inbox and tmt answer', () => {
  it('lists what waits on you and answers it without a receipt', async () => {
    await withSandbox(async (sandbox) => {
      const tmuxLog = installTmuxTripwire(sandbox);
      await json(sandbox, ['identity', 'create', 'ben']);
      const alice = await json(sandbox, ['identity', 'create', 'Alice']);
      await json(sandbox, ['identity', 'create', 'bob']);
      const first = await ask(sandbox, 'alice', 'merge it?\nsecond line');
      const second = await ask(sandbox, 'alice', 'which branch?');
      const third = await ask(sandbox, 'bob', 'ship tonight?');

      const inbox = await json(sandbox, ['inbox', '--identity', 'ben']);
      expect(inbox).toMatchObject({
        identity: { canonicalName: 'ben' },
        items: [
          {
            requestId: first,
            from: {
              identityId: (alice.identity as { id: string }).id,
              name: 'Alice',
              canonicalName: 'alice',
            },
            delivery: 'queued',
            preview: 'merge it?\nsecond line',
          },
          { requestId: second },
          { requestId: third, from: { canonicalName: 'bob' } },
        ],
        more: false,
      });
      expect(
        await json(sandbox, ['inbox', '--identity', 'ben', '--from', 'bob', '--limit', '1'])
      ).toMatchObject({
        items: [{ requestId: third }],
        more: false,
      });

      const human = await runCli(sandbox, ['inbox', '--identity', 'ben'], { deadlineMs: 5_000 });
      expect(human.status).toBe(0);
      expect(human.stdout).toContain('WAITING ON YOU 3');
      expect(human.stdout).toContain(first);
      expect(human.stdout).toContain('merge it?');
      expect(human.stdout).not.toContain('second line');

      // Two open requests from alice: nothing is sent until one is chosen.
      const ambiguous = await runCli(
        sandbox,
        ['answer', 'alice', 'yes', '--identity', 'ben', '--json'],
        {
          deadlineMs: 5_000,
        }
      );
      expect(ambiguous.status).toBe(1);
      const refusal = expectError(ambiguous, 'ANSWER_AMBIGUOUS');
      expect(JSON.stringify(refusal)).toContain(first);
      expect(JSON.stringify(refusal)).toContain(second);

      const answered = await json(sandbox, [
        'answer',
        'alice',
        'Yes, merge it',
        '--request',
        first,
        '--identity',
        'ben',
      ]);
      expect(answered).toMatchObject({
        status: 'submitted',
        requestId: first,
        from: { canonicalName: 'alice' },
        bodyBytes: 13,
      });
      const file = path.join(sandbox.root, 'answer.md');
      writeFileSync(file, 'main\n');
      expect(
        await json(sandbox, ['answer', 'alice', '--file', file, '--identity', 'ben'])
      ).toMatchObject({
        requestId: second,
        bodyBytes: 5,
      });
      expect(
        await json(sandbox, ['answer', 'bob', '--stdin', '--identity', 'ben'], 'not tonight')
      ).toMatchObject({
        requestId: third,
      });

      for (const [id, response] of [
        [first, 'Yes, merge it'],
        [second, 'main\n'],
        [third, 'not tonight'],
      ]) {
        expect(await json(sandbox, ['result', id])).toMatchObject({
          status: 'completed',
          response,
        });
      }
      expect(await json(sandbox, ['inbox', '--identity', 'ben'])).toMatchObject({
        items: [],
        more: false,
      });

      const idle = await runCli(
        sandbox,
        ['answer', 'alice', 'again', '--identity', 'ben', '--json'],
        {
          deadlineMs: 5_000,
        }
      );
      expect(idle.status).toBe(3);
      expectError(idle, 'ANSWER_NOT_WAITING');
      const conflict = await runCli(
        sandbox,
        ['answer', 'alice', 'changed', '--request', first, '--identity', 'ben', '--json'],
        { deadlineMs: 5_000 }
      );
      expect(conflict.status).toBe(5);
      expectError(conflict, 'RESPONSE_CONFLICT');
      const wrongOwner = await runCli(
        sandbox,
        ['answer', 'bob', 'x', '--request', first, '--identity', 'ben', '--json'],
        { deadlineMs: 5_000 }
      );
      expect(wrongOwner.status).toBe(3);
      expectError(wrongOwner, 'X_NOT_FOUND');

      for (const output of [JSON.stringify(inbox), JSON.stringify(answered), human.stdout]) {
        expect(output).not.toContain('receipt');
        expect(output).not.toMatch(/v2_[A-Za-z0-9_-]{22}/);
      }
      // With --identity nothing touches tmux.
      expect(existsSync(tmuxLog)).toBe(false);

      // Without it the shared caller lookup runs, and outside a bound pane
      // the identity is required, as for `x`.
      const anonymous = await runCli(sandbox, ['inbox', '--json'], { deadlineMs: 5_000 });
      expect(anonymous.status).toBe(1);
      expectError(anonymous, 'IDENTITY_REQUIRED');

      // An anonymous sender's request is listed, cannot be chosen by name,
      // and is answered by its request ID alone.
      const anonymousTalk = await runCli(
        sandbox,
        ['talk', 'ben', 'who am I?', '--inbox', '--detach', '--json'],
        {
          deadlineMs: 5_000,
        }
      );
      expect(anonymousTalk.status).toBe(0);
      const anonymousId = parseWholeStdout(anonymousTalk).requestId as string;
      expect(await json(sandbox, ['inbox', '--identity', 'ben'])).toMatchObject({
        items: [{ requestId: anonymousId, from: null, preview: 'who am I?' }],
      });
      const byName = await runCli(
        sandbox,
        ['answer', 'alice', 'x', '--identity', 'ben', '--json'],
        {
          deadlineMs: 5_000,
        }
      );
      expect(byName.status).toBe(3);
      expectError(byName, 'ANSWER_NOT_WAITING');
      expect(
        await json(sandbox, [
          'answer',
          '--request',
          anonymousId,
          'nobody knows',
          '--identity',
          'ben',
        ])
      ).toMatchObject({
        status: 'submitted',
        requestId: anonymousId,
        from: null,
      });
      expect(await json(sandbox, ['result', anonymousId])).toMatchObject({
        response: 'nobody knows',
      });
      expect(await json(sandbox, ['inbox', '--identity', 'ben'])).toMatchObject({ items: [] });
    });
  });
});
