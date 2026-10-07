import Database from 'better-sqlite3';
import { readFileSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import { describe, expect, it } from 'vite-plus/test';
import { runCli, withSandbox, type Sandbox } from '../support/cli-process.js';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';

async function api(sandbox: Sandbox, operation: string, input: unknown) {
  const result = await runCli(sandbox, ['api'], {
    stdin: JSON.stringify({ version: 1, operation, input }),
  });
  expect(result.stderr).toBe('');
  return { ...result, body: JSON.parse(result.stdout) };
}

async function identity(sandbox: Sandbox, name: string) {
  const result = await runCli(sandbox, ['identity', 'create', name, '--json']);
  expect(result.status).toBe(0);
  return JSON.parse(result.stdout).identity.id as string;
}

describe('native Focus held delivery and checklist seam', () => {
  it('does not transfer a retired UUID policy or backlog to a reused name', async () => {
    await withSandbox(async (sandbox) => {
      const original = await identity(sandbox, 'Worker');
      const owner = await identity(sandbox, 'Owner');
      expect(
        (
          await api(sandbox, 'focus.policy.set', {
            identityId: original,
            ownerIdentityId: owner,
            setterIdentityId: owner,
            expectedRevision: 0,
            untilMs: Date.now() + 600_000,
          })
        ).status
      ).toBe(0);
      const held = await runCli(sandbox, ['talk', 'Worker', 'Original backlog', '--json']);
      expect(JSON.parse(held.stdout)).toMatchObject({ focus: true });
      expect((await runCli(sandbox, ['rm', 'Worker', '--force', '--json'])).status).toBe(0);
      const replacement = await identity(sandbox, 'Worker');
      expect(replacement).not.toBe(original);
      expect(
        (await api(sandbox, 'focus.policy.show', { identities: [replacement] })).body.policies
      ).toMatchObject([{ identityId: replacement, revision: 0, active: false, heldCount: 0 }]);
      expect(
        (await api(sandbox, 'focus.checklist.read', { identityId: original })).body.error.code
      ).toBe('FOCUS_IDENTITY_UNAVAILABLE');
      expect(
        (await api(sandbox, 'focus.checklist.read', { identityId: replacement })).body.items
      ).toEqual([]);
    });
  });

  it('queues ordinary talk without input or waiter and retains the original receipt and final', async () => {
    await withSandbox(async (sandbox) => {
      const log = await calibrateTmuxTripwire(sandbox);
      const target = await identity(sandbox, 'Worker');
      const owner = await identity(sandbox, 'Owner');
      const sender = await identity(sandbox, 'Sender');
      const untilMs = Date.now() + 600_000;
      const set = {
        identityId: target,
        ownerIdentityId: owner,
        setterIdentityId: owner,
        expectedRevision: 0,
        untilMs,
      };
      expect((await api(sandbox, 'focus.policy.set', set)).body).toMatchObject({
        revision: 1,
        active: true,
        heldCount: 0,
      });
      const text = 'Original first line\nExact second line';
      const result = await runCli(sandbox, [
        'talk',
        'Worker',
        text,
        '--identity',
        'Sender',
        '--kind',
        'decision',
        '--timeout',
        '1',
        '--json',
      ]);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      const queued = JSON.parse(result.stdout);
      expect(queued).toMatchObject({
        status: 'queued',
        focus: true,
        recipientIdentityId: target,
        focusUntilMs: untilMs,
        notification: 'held',
        waitingFor: 'focus_checklist',
      });
      expect(queued.offline).toBeUndefined();
      expect(queued.remainingMs).toBeGreaterThan(0);
      expect(queued.remainingMs).toBeLessThanOrEqual(600_000);
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(
          db
            .prepare(
              'SELECT status,wait_active,message_text FROM request_attempts WHERE request_id=?'
            )
            .get(queued.requestId)
        ).toEqual({ status: 'queued', wait_active: 0, message_text: text });
        expect(
          db
            .prepare('SELECT waiter_pid FROM request_notifications WHERE request_id=?')
            .get(queued.requestId)
        ).toEqual({ waiter_pid: null });
      } finally {
        db.close();
      }
      const pending = (await api(sandbox, 'focus.checklist.read', { identityId: target })).body;
      expect(pending.total).toBe(1);
      expect(pending.items[0]).toMatchObject({
        requestId: queued.requestId,
        kind: 'decision',
        source: 'incoming',
        sender: { identityId: sender, name: 'Sender' },
      });
      const receipt = pending.items[0].replyCommand.match(/--receipt (\S+)/)[1];
      const original = await runCli(sandbox, [
        'x',
        'show',
        queued.requestId,
        '--incoming',
        '--identity',
        'Worker',
        '--json',
      ]);
      expect(JSON.parse(original.stdout).exchange.reply.receipt).toBe(receipt);
      expect(
        (await api(sandbox, 'focus.policy.set', { ...set, expectedRevision: 0 })).body.error.code
      ).toBe('FOCUS_REVISION_CONFLICT');
      expect(
        (await api(sandbox, 'focus.policy.set', { ...set, expectedRevision: 1, everyMs: 1000 }))
          .body.error.code
      ).toBe('API_INPUT_INVALID');
      expect((await api(sandbox, 'focus.policy.set', { ...set, identityId: sender })).status).toBe(
        0
      );
      const reply = await runCli(sandbox, [
        'reply',
        queued.requestId,
        '--receipt',
        receipt,
        '--message',
        'Exact final\nSecond line',
        '--json',
      ]);
      expect(reply.status).toBe(0);
      const final = await runCli(sandbox, ['result', queued.requestId, '--json']);
      expect(final.status).toBe(0);
      expect(JSON.parse(final.stdout).response).toBe('Exact final\nSecond line');
      const results = (await api(sandbox, 'focus.checklist.read', { identityId: sender })).body;
      expect(results.total).toBe(1);
      expect(results.items[0]).toMatchObject({
        kind: 'result',
        source: 'result',
        resultPreview: 'Exact final Second line',
      });
      expect(results.items[0].replyCommand).toBeUndefined();
      expect(
        (await api(sandbox, 'focus.checklist.read', { identityId: target })).body.items[0]
          .replyCommand
      ).toBeUndefined();
      expect(readFileSync(log, 'utf8')).toBe('\n');
    });
  });

  it('claims one sealed checklist, fences concurrent consumers and never replays uncertain delivery', async () => {
    await withSandbox(async (sandbox) => {
      const target = await identity(sandbox, 'Worker');
      const owner = await identity(sandbox, 'Owner');
      const untilMs = Date.now() + 600_000;
      expect(
        (
          await api(sandbox, 'focus.policy.set', {
            identityId: target,
            ownerIdentityId: owner,
            setterIdentityId: owner,
            expectedRevision: 0,
            untilMs,
          })
        ).status
      ).toBe(0);
      const ids: string[] = [];
      for (const message of ['First', 'Second']) {
        const talk = await runCli(sandbox, ['talk', 'Worker', message, '--json']);
        expect(talk.status).toBe(0);
        ids.push(JSON.parse(talk.stdout).requestId);
      }
      expect(
        (await api(sandbox, 'focus.checklist.claim', { identityId: target, opportunity: 'idle' }))
          .body
      ).toEqual({ claimed: false });
      const claims = await Promise.all(
        [0, 1].map(() =>
          api(sandbox, 'focus.checklist.claim', {
            identityId: target,
            opportunity: 'turn_boundary',
          })
        )
      );
      expect(claims.every((c) => c.status === 0)).toBe(true);
      expect(claims.filter((c) => c.body.claimed)).toHaveLength(1);
      const claim = claims.find((c) => c.body.claimed)!.body;
      expect(claim.page.items.map((i: { requestId: string }) => i.requestId)).toEqual(ids);
      expect(claim.text.match(/^TMT Focus checklist/g)).toHaveLength(1);
      const later = await runCli(sandbox, ['talk', 'Worker', 'Later arrival', '--json']);
      expect(later.status).toBe(0);
      const sealed = (
        await api(sandbox, 'focus.checklist.read', {
          identityId: target,
          checklistId: claim.checklist.checklistId,
          limit: 1,
        })
      ).body;
      expect(sealed.items[0].requestId).toBe(ids[0]);
      expect(sealed.remaining).toBe(1);
      const rest = (
        await api(sandbox, 'focus.checklist.read', {
          identityId: target,
          checklistId: claim.checklist.checklistId,
          after: sealed.nextAfter,
        })
      ).body;
      expect(rest.items.map((i: { requestId: string }) => i.requestId)).toEqual([ids[1]]);
      const settlement = {
        identityId: target,
        checklistId: claim.checklist.checklistId,
        attemptToken: claim.checklist.attemptToken,
        outcome: 'uncertain',
      };
      expect(
        (
          await api(sandbox, 'focus.checklist.settle', {
            ...settlement,
            attemptToken: randomUUID(),
          })
        ).body.error.code
      ).toBe('FOCUS_ATTEMPT_MISMATCH');
      expect((await api(sandbox, 'focus.checklist.settle', settlement)).body.changed).toBe(true);
      expect((await api(sandbox, 'focus.checklist.settle', settlement)).body.changed).toBe(false);
      expect(
        (
          await api(sandbox, 'focus.checklist.settle', {
            ...settlement,
            outcome: 'definitely_unsent',
          })
        ).body.error.code
      ).toBe('FOCUS_STATE_INVALID');
      const next = (
        await api(sandbox, 'focus.checklist.claim', {
          identityId: target,
          opportunity: 'turn_boundary',
        })
      ).body;
      expect(next.page.items.map((i: { requestId: string }) => i.requestId)).toEqual([
        JSON.parse(later.stdout).requestId,
      ]);
      expect(
        (
          await api(sandbox, 'focus.checklist.settle', {
            identityId: target,
            checklistId: next.checklist.checklistId,
            attemptToken: next.checklist.attemptToken,
            outcome: 'delivered',
          })
        ).status
      ).toBe(0);
      expect(
        (
          await api(sandbox, 'focus.checklist.claim', {
            identityId: target,
            opportunity: 'turn_boundary',
          })
        ).body.claimed
      ).toBe(false);
    });
  });

  it('prints remaining time; owner and urgent bypass only Focus, while explicit inbox remains pull-only', async () => {
    await withSandbox(async (sandbox) => {
      const target = await identity(sandbox, 'Worker');
      const owner = await identity(sandbox, 'Owner');
      expect(
        (
          await api(sandbox, 'focus.policy.set', {
            identityId: target,
            ownerIdentityId: owner,
            setterIdentityId: owner,
            expectedRevision: 0,
            untilMs: Date.now() + 600_000,
          })
        ).status
      ).toBe(0);
      const human = await runCli(sandbox, ['talk', 'Worker', 'Held text']);
      expect(human.status).toBe(0);
      expect(human.stdout).toContain('is in focus for');
      expect(human.stdout).toContain('delivery is in its next checklist');
      for (const flags of [['--urgent'], ['--identity', 'Owner']]) {
        const talk = await runCli(sandbox, [
          'talk',
          'Worker',
          'Bypass',
          '--detach',
          '--json',
          ...flags,
        ]);
        expect(talk.status).toBe(0);
        const result = JSON.parse(talk.stdout);
        expect(result.focus).toBeUndefined();
        expect(result.status).toBe('queued');
        if (flags[0] === '--urgent') {
          const history = await runCli(sandbox, [
            'x',
            'show',
            result.requestId,
            '--incoming',
            '--identity',
            'Worker',
            '--json',
          ]);
          expect(JSON.parse(history.stdout).exchange.urgent).toBe(true);
        }
      }
      const pull = await runCli(sandbox, [
        'talk',
        'Worker',
        'Pull only',
        '--inbox',
        '--detach',
        '--json',
      ]);
      expect(pull.status).toBe(0);
      expect(JSON.parse(pull.stdout)).toMatchObject({
        notification: 'not_attempted',
        waitingFor: 'recipient_inbox_pull',
      });
      const clear = await api(sandbox, 'focus.policy.clear', {
        identityId: target,
        ownerIdentityId: owner,
        setterIdentityId: owner,
        expectedRevision: 1,
      });
      expect(clear.body).toMatchObject({
        revision: 2,
        active: false,
        remainingMs: 0,
        heldCount: 1,
      });
      expect(
        (await api(sandbox, 'focus.checklist.claim', { identityId: target, opportunity: 'idle' }))
          .body.claimed
      ).toBe(false);
      expect(
        (await api(sandbox, 'focus.policy.show', { identities: [target] })).body.policies[0]
          .heldCount
      ).toBe(1);
    });
  });
});
