import Database from 'better-sqlite3';
import { readFileSync, existsSync } from 'node:fs';
import path from 'node:path';
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

describe('native Digest held delivery and checklist seam', () => {
  it('advertises only digest operations and rejects the retired feature names without creating state', async () => {
    await withSandbox(async (sandbox) => {
      const suffixes = [
        'policy.set',
        'policy.clear',
        'policy.show',
        'checklist.read',
        'checklist.claim',
        'checklist.settle',
        'checklist.dueNow',
        'stats.show',
      ];
      const discovery = await api(sandbox, 'capabilities', {});
      expect(discovery.status).toBe(0);
      const operations = discovery.body.operations as string[];
      expect(operations.filter((op) => op.startsWith('digest.')).sort()).toEqual(
        suffixes.map((suffix) => `digest.${suffix}`).sort()
      );
      expect(operations.some((op) => op.startsWith('focus.'))).toBe(false);
      for (const suffix of suffixes) {
        const rejected = await api(sandbox, `focus.${suffix}`, {});
        expect(rejected.status).toBe(1);
        expect(rejected.body.error.code).toBe('API_INPUT_INVALID');
      }
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });
  it('launch callbacks refuse malformed, recursive and unbound evidence without initializing storage', async () => {
    await withSandbox(async (sandbox) => {
      const launch = JSON.stringify({
        identity_id: randomUUID(),
        binding_id: randomUUID(),
        owner_pid: 1,
        owner_start: 'not-authority',
      });
      for (const [scope, input] of [
        [launch, { hook_event_name: 'Stop', session_id: 'session', stop_hook_active: false }],
        [launch, { hook_event_name: 'Stop', session_id: 'session', stop_hook_active: true }],
        [
          launch,
          { hook_event_name: 'SubagentStop', session_id: 'session', stop_hook_active: false },
        ],
        ['{}', { hook_event_name: 'Stop', session_id: 'session', stop_hook_active: false }],
      ] as const) {
        const result = await runCli(sandbox, ['__digest-hook', 'claude', '--launch', scope], {
          stdin: JSON.stringify(input),
        });
        expect(result).toMatchObject({ status: 0, stdout: '', stderr: '' });
        const legacy = await runCli(sandbox, ['__focus-hook', 'claude', '--launch', scope], {
          stdin: JSON.stringify(input),
        });
        expect(legacy).toMatchObject({
          status: result.status,
          stdout: result.stdout,
          stderr: result.stderr,
        });
      }
      expect(existsSync(sandbox.database)).toBe(false);
      expect(existsSync(path.join(sandbox.home, '.claude'))).toBe(false);
    });
  });
  it('stable Codex callback refuses missing turn, recursive, auxiliary and unbound admission without settings or storage writes', async () => {
    await withSandbox(async (sandbox) => {
      const good = {
        hook_event_name: 'Stop',
        session_id: randomUUID(),
        turn_id: randomUUID(),
        stop_hook_active: false,
      };
      for (const input of [
        good,
        { ...good, turn_id: '' },
        { ...good, stop_hook_active: true },
        { ...good, hook_event_name: 'SubagentStop' },
      ]) {
        const result = await runCli(sandbox, ['__digest-hook', 'codex', '--discover-launch'], {
          stdin: JSON.stringify(input),
        });
        expect(result).toMatchObject({ status: 0, stdout: '', stderr: '' });
        const legacy = await runCli(sandbox, ['__focus-hook', 'codex', '--discover-launch'], {
          stdin: JSON.stringify(input),
        });
        expect(legacy).toMatchObject({
          status: result.status,
          stdout: result.stdout,
          stderr: result.stderr,
        });
      }
      expect(existsSync(sandbox.database)).toBe(false);
      expect(existsSync(path.join(sandbox.home, '.codex'))).toBe(false);
    });
  });

  it('diagnoses a target checklist error without rejecting a new talk from its sender', async () => {
    await withSandbox(async (sandbox) => {
      const target = await identity(sandbox, 'Worker');
      const checklist = randomUUID();
      const db = new Database(sandbox.database);
      try {
        // Fault injection is isolated to the target's old checklist decoder;
        // the new request and its policy remain valid and independently writable.
        db.pragma('ignore_check_constraints = ON');
        db.prepare(
          `INSERT INTO focus_checklists
           (id,identity_id,attempt_token,through_sequence,state,created_at_ms)
           VALUES(?,?,?,1,'claimed',-1)`
        ).run(checklist, target, randomUUID());
      } finally {
        db.close();
      }
      const result = await runCli(sandbox, [
        'talk',
        'Worker',
        'Independent new request',
        '--detach',
        '--json',
      ]);
      expect(result.status).toBe(0);
      expect(result.stderr).toContain(`Could not flush Digest checklist for ${target}`);
      const queued = JSON.parse(result.stdout);
      expect(queued).toMatchObject({ status: 'queued', notification: 'not_attempted' });
      const oracle = new Database(sandbox.database, { readonly: true });
      try {
        expect(
          oracle
            .prepare('SELECT message_text,status FROM request_attempts WHERE request_id=?')
            .get(queued.requestId)
        ).toEqual({ message_text: 'Independent new request', status: 'queued' });
        expect(
          oracle
            .prepare('SELECT state,created_at_ms FROM focus_checklists WHERE id=?')
            .get(checklist)
        ).toEqual({ state: 'claimed', created_at_ms: -1 });
      } finally {
        oracle.close();
      }
    });
  });

  it('does not transfer a retired UUID policy or backlog to a reused name', async () => {
    await withSandbox(async (sandbox) => {
      const original = await identity(sandbox, 'Worker');
      const owner = await identity(sandbox, 'Owner');
      expect(
        (
          await api(sandbox, 'digest.policy.set', {
            identityId: original,
            ownerIdentityId: owner,
            setterIdentityId: owner,
            expectedRevision: 0,
            untilMs: Date.now() + 600_000,
          })
        ).status
      ).toBe(0);
      const held = await runCli(sandbox, ['talk', 'Worker', 'Original backlog', '--json']);
      expect(JSON.parse(held.stdout)).toMatchObject({ digest: true });
      expect((await runCli(sandbox, ['rm', 'Worker', '--force', '--json'])).status).toBe(0);
      const replacement = await identity(sandbox, 'Worker');
      expect(replacement).not.toBe(original);
      expect(
        (await api(sandbox, 'digest.policy.show', { identities: [replacement] })).body.policies
      ).toMatchObject([{ identityId: replacement, revision: 0, active: false, heldCount: 0 }]);
      expect(
        (await api(sandbox, 'digest.checklist.read', { identityId: original })).body.error.code
      ).toBe('DIGEST_IDENTITY_UNAVAILABLE');
      expect(
        (await api(sandbox, 'digest.checklist.read', { identityId: replacement })).body.items
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
      expect((await api(sandbox, 'digest.policy.set', set)).body).toMatchObject({
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
        digest: true,
        recipientIdentityId: target,
        digestUntilMs: untilMs,
        notification: 'held',
        waitingFor: 'digest_checklist',
      });
      expect(queued.offline).toBeUndefined();
      expect(queued).not.toHaveProperty('focus');
      expect(queued).not.toHaveProperty('focusUntilMs');
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
      const pending = (await api(sandbox, 'digest.checklist.read', { identityId: target })).body;
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
        (await api(sandbox, 'digest.policy.set', { ...set, expectedRevision: 0 })).body.error.code
      ).toBe('DIGEST_REVISION_CONFLICT');
      expect(
        (await api(sandbox, 'digest.policy.set', { ...set, expectedRevision: 1, everyMs: 1000 }))
          .body.error.code
      ).toBe('API_INPUT_INVALID');
      expect((await api(sandbox, 'digest.policy.set', { ...set, identityId: sender })).status).toBe(
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
      const results = (await api(sandbox, 'digest.checklist.read', { identityId: sender })).body;
      expect(results.total).toBe(1);
      expect(results.items[0]).toMatchObject({
        kind: 'result',
        source: 'result',
        resultPreview: 'Exact final Second line',
      });
      expect(results.items[0].replyCommand).toBeUndefined();
      expect(
        (await api(sandbox, 'digest.checklist.read', { identityId: target })).body.items[0]
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
          await api(sandbox, 'digest.policy.set', {
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
        (await api(sandbox, 'digest.checklist.claim', { identityId: target, opportunity: 'idle' }))
          .body
      ).toEqual({ claimed: false });
      const claims = await Promise.all(
        [0, 1].map(() =>
          api(sandbox, 'digest.checklist.claim', {
            identityId: target,
            opportunity: 'turn_boundary',
          })
        )
      );
      expect(claims.every((c) => c.status === 0)).toBe(true);
      expect(claims.filter((c) => c.body.claimed)).toHaveLength(1);
      const claim = claims.find((c) => c.body.claimed)!.body;
      expect(claim.page.items.map((i: { requestId: string }) => i.requestId)).toEqual(ids);
      expect(claim.text.match(/^TMT Digest checklist/g)).toHaveLength(1);
      const later = await runCli(sandbox, ['talk', 'Worker', 'Later arrival', '--json']);
      expect(later.status).toBe(0);
      const sealed = (
        await api(sandbox, 'digest.checklist.read', {
          identityId: target,
          checklistId: claim.checklist.checklistId,
          limit: 1,
        })
      ).body;
      expect(sealed.items[0].requestId).toBe(ids[0]);
      expect(sealed.remaining).toBe(1);
      const rest = (
        await api(sandbox, 'digest.checklist.read', {
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
          await api(sandbox, 'digest.checklist.settle', {
            ...settlement,
            attemptToken: randomUUID(),
          })
        ).body.error.code
      ).toBe('DIGEST_ATTEMPT_MISMATCH');
      expect((await api(sandbox, 'digest.checklist.settle', settlement)).body.changed).toBe(true);
      expect((await api(sandbox, 'digest.checklist.settle', settlement)).body.changed).toBe(false);
      expect(
        (
          await api(sandbox, 'digest.checklist.settle', {
            ...settlement,
            outcome: 'definitely_unsent',
          })
        ).body.error.code
      ).toBe('DIGEST_STATE_INVALID');
      const next = (
        await api(sandbox, 'digest.checklist.claim', {
          identityId: target,
          opportunity: 'turn_boundary',
        })
      ).body;
      expect(next.page.items.map((i: { requestId: string }) => i.requestId)).toEqual([
        JSON.parse(later.stdout).requestId,
      ]);
      expect(
        (
          await api(sandbox, 'digest.checklist.settle', {
            identityId: target,
            checklistId: next.checklist.checklistId,
            attemptToken: next.checklist.attemptToken,
            outcome: 'delivered',
          })
        ).status
      ).toBe(0);
      expect(
        (
          await api(sandbox, 'digest.checklist.claim', {
            identityId: target,
            opportunity: 'turn_boundary',
          })
        ).body.claimed
      ).toBe(false);
    });
  });

  it('prints remaining time; owner and urgent bypass only Digest, while explicit inbox remains pull-only', async () => {
    await withSandbox(async (sandbox) => {
      const target = await identity(sandbox, 'Worker');
      const owner = await identity(sandbox, 'Owner');
      expect(
        (
          await api(sandbox, 'digest.policy.set', {
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
      expect(human.stdout).toContain('is in digest mode for');
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
        expect(result.digest).toBeUndefined();
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
      const clear = await api(sandbox, 'digest.policy.clear', {
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
        (await api(sandbox, 'digest.checklist.claim', { identityId: target, opportunity: 'idle' }))
          .body.claimed
      ).toBe(false);
      expect(
        (await api(sandbox, 'digest.policy.show', { identities: [target] })).body.policies[0]
          .heldCount
      ).toBe(1);
    });
  });
});

describe('native Digest due-now and durable stats', () => {
  it('adds eligibility without changing policy and counts only successful settlements', async () => {
    await withSandbox(async (sandbox) => {
      const target = await identity(sandbox, 'Worker');
      const owner = await identity(sandbox, 'Owner');
      await identity(sandbox, 'Sender');
      const untilMs = Date.now() + 600_000;
      expect(
        (
          await api(sandbox, 'digest.policy.set', {
            identityId: target,
            ownerIdentityId: owner,
            setterIdentityId: owner,
            expectedRevision: 0,
            untilMs,
          })
        ).status
      ).toBe(0);
      const hold = async (text: string) => {
        const sent = await runCli(sandbox, [
          'talk',
          'Worker',
          text,
          '--identity',
          'Sender',
          '--json',
        ]);
        expect(sent.status).toBe(0);
        expect(JSON.parse(sent.stdout)).toMatchObject({ digest: true, status: 'queued' });
        return JSON.parse(sent.stdout).requestId;
      };
      const first = await hold('First');
      const before = (await api(sandbox, 'digest.stats.show', { identities: [target] })).body
        .stats[0];
      expect(before).toMatchObject({
        identityId: target,
        heldCount: 1,
        deliveredDigests: 0,
        dueCount: 0,
        nextEligibleAtMs: untilMs,
      });
      expect(before.oldestHeldAgeMs).toBeGreaterThanOrEqual(0);
      const due = await api(sandbox, 'digest.checklist.dueNow', { identityId: target });
      expect(due.status).toBe(0);
      expect(due.body).toMatchObject({ identityId: target, heldCount: 1 });
      const observedAtMs = Date.now();
      const receiver = new Database(sandbox.database);
      try {
        receiver
          .prepare(
            "INSERT INTO identity_session_preferences(identity_id,remembered_harness,runtime_mode,provider_session_id,driver_state_version,driver_state) VALUES(?,'claude','default','arrival-session',2,?)"
          )
          .run(target, JSON.stringify({ usage: { tokens: 182340, observedAtMs } }));
      } finally {
        receiver.close();
      }
      const later = await hold('Later');
      const changedUsage = new Database(sandbox.database);
      try {
        changedUsage
          .prepare(
            'UPDATE identity_session_preferences SET driver_state=NULL,driver_state_version=NULL WHERE identity_id=?'
          )
          .run(target);
      } finally {
        changedUsage.close();
      }
      const after = (await api(sandbox, 'digest.stats.show', { identities: [target] })).body
        .stats[0];
      expect(after).toMatchObject({ heldCount: 2, dueCount: 1, deliveredDigests: 0 });
      expect(after.nextEligibleAtMs).toBe(after.observedAtMs);
      const pending = (await api(sandbox, 'digest.checklist.read', { identityId: target })).body;
      expect(pending.items.map((item: { requestId: string }) => item.requestId)).toEqual([
        first,
        later,
      ]);
      expect(pending.items[0]).not.toHaveProperty('contextTokensAtArrival');
      expect(pending.items[0]).not.toHaveProperty('contextObservedAtMs');
      expect(pending.items[1]).toMatchObject({
        contextTokensAtArrival: 182340,
        contextObservedAtMs: observedAtMs,
      });
      expect(
        (await api(sandbox, 'digest.policy.show', { identities: [target] })).body.policies[0]
      ).toMatchObject({ revision: 1, active: true, digestUntilMs: untilMs });
      // Provider-admitted turn boundaries retain their existing broader eligibility.
      const claim = (
        await api(sandbox, 'digest.checklist.claim', {
          identityId: target,
          opportunity: 'turn_boundary',
        })
      ).body;
      expect(claim.claimed).toBe(true);
      expect(claim.page.total).toBe(2);
      const settle = {
        identityId: target,
        checklistId: claim.checklist.checklistId,
        attemptToken: claim.checklist.attemptToken,
        outcome: 'delivered',
      };
      expect((await api(sandbox, 'digest.checklist.settle', settle)).body.changed).toBe(true);
      expect((await api(sandbox, 'digest.checklist.settle', settle)).body.changed).toBe(false);
      expect(
        (await api(sandbox, 'digest.stats.show', { identities: [target] })).body.stats[0]
      ).toMatchObject({
        heldCount: 0,
        oldestHeldAgeMs: null,
        deliveredDigests: 1,
        dueCount: 0,
        nextEligibleAtMs: null,
      });
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(
          db
            .prepare(
              'SELECT due_through_sequence,delivered_digests FROM digest_counters WHERE identity_id=?'
            )
            .get(target)
        ).toEqual({ due_through_sequence: due.body.throughSequence, delivered_digests: 1 });
      } finally {
        db.close();
      }
    });
  });
  it('refuses malformed due-now and stats requests before creating storage', async () => {
    await withSandbox(async (sandbox) => {
      for (const [operation, input] of [
        ['digest.checklist.dueNow', { identityId: 'invalid' }],
        ['digest.checklist.dueNow', { identityId: randomUUID(), intervalMs: 1000 }],
        ['digest.stats.show', { identities: [] }],
        ['digest.stats.show', { identities: [randomUUID()], refresh: true }],
      ] as const) {
        const refused = await api(sandbox, operation, input);
        expect(refused.status).toBe(1);
        expect(refused.body.error.code).toBe('API_INPUT_INVALID');
      }
      expect(existsSync(sandbox.database)).toBe(false);
    });
  });
});
