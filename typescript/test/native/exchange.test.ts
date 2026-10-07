import Database from 'better-sqlite3';
import { readFileSync, writeFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import {
  expectError,
  parseWholeStdout,
  runCli,
  withSandbox,
  type Sandbox,
} from '../support/cli-process.js';
import { seedResponse, responseSnapshot } from './response-fixture.js';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';

async function json(sandbox: Sandbox, args: string[]) {
  const result = await runCli(sandbox, [...args, '--json']);
  expect(result.status, result.stdout || result.stderr).toBe(0);
  return parseWholeStdout(result);
}

function attentionFixture(sandbox: Sandbox, owner: string, requestId: string) {
  const seeded = seedResponse(sandbox.database, requestId);
  const database = new Database(sandbox.database);
  try {
    database.transaction(() => {
      database
        .prepare(
          'INSERT INTO request_attention_identities (identity_id, latest_revision, acknowledged_through) VALUES (?, 1, 0)'
        )
        .run(owner);
      database
        .prepare(
          "UPDATE request_attempts SET originator_kind = 'explicit', originator_identity_id = ?, attention_revision = 1 WHERE request_id = ?"
        )
        .run(owner, requestId);
    })();
  } finally {
    database.close();
  }
  return seeded;
}

describe('native exchange attention process contract', () => {
  it('renders human attention and reply results without confusing acknowledgment with completion', async () => {
    await withSandbox(async (sandbox) => {
      const log = await calibrateTmuxTripwire(sandbox);
      const owner = (await json(sandbox, ['identity', 'create', 'Reader'])).identity as {
        id: string;
      };
      const seeded = attentionFixture(sandbox, owner.id, 'human-exchange');
      const before = responseSnapshot(sandbox.database, seeded.requestId);
      const listed = await runCli(sandbox, ['x', '--identity', 'Reader']);
      expect(listed.status).toBe(0);
      expect(listed.stderr).toBe('');
      expect(listed.stdout).toBe(
        'EXCHANGES 1\n' + '  human-exchange  -  sent  not_submitted  r1\n'
      );
      expect(responseSnapshot(sandbox.database, seeded.requestId)).toEqual(before);
      const acknowledged = await runCli(sandbox, [
        'x',
        'ack',
        seeded.requestId,
        '--revision',
        '1',
        '--identity',
        'Reader',
      ]);
      expect(acknowledged.status).toBe(0);
      expect(acknowledged.stdout).toBe('✓ Acknowledged human-exchange at revision 1\n');
      const pending = await json(sandbox, ['x', 'show', seeded.requestId, '--identity', 'Reader']);
      expect(pending.exchange).toMatchObject({
        acknowledged: true,
        settled: false,
        final: { status: 'not_submitted' },
      });
      const empty = await runCli(sandbox, ['x', '--identity', 'Reader']);
      expect(empty.status).toBe(0);
      expect(empty.stdout).toBe('No unacknowledged exchanges.\n');

      const body = '  final\tresult\r\nsecond line \u001b[31mred\u001b[0m  ';
      const submitted = await runCli(sandbox, [
        'reply',
        seeded.requestId,
        '--receipt',
        seeded.compactReceipt,
        '--message',
        body,
      ]);
      expect(submitted.status).toBe(0);
      expect(submitted.stderr).toBe('');
      expect(submitted.stdout).toBe(
        `✓ Submitted response for request human-exchange (${Buffer.byteLength(body)} bytes)\n`
      );
      expect(responseSnapshot(sandbox.database, seeded.requestId).response).toMatchObject({
        body,
        body_bytes: Buffer.byteLength(body),
      });
      const result = await runCli(sandbox, ['result', seeded.requestId]);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      expect(result.stdout).toBe(`Response for request 'human-exchange':\n${body}\n`);
      const shown = await runCli(sandbox, ['x', 'show', seeded.requestId, '--identity', 'Reader']);
      expect(shown.status).toBe(0);
      expect(shown.stdout).toBe(
        'human-exchange\n' +
          '  delivery      sent\n' +
          '  urgent        false\n' +
          '  focus kind    fyi\n' +
          '  final         retained\n' +
          '  revision      2\n' +
          '  acknowledged  false\n' +
          '  settled       false\n' +
          `PROMPT retained\nprompt for human-exchange\nFINAL\n${body}\n`
      );
      const all = await runCli(sandbox, ['x', 'ackall', '--identity', 'Reader']);
      expect(all.status).toBe(0);
      expect(all.stdout).toBe(
        "✓ Acknowledged 'Reader' through revision 2; later revisions remain unacknowledged\n"
      );
      expect(
        (await json(sandbox, ['x', 'show', seeded.requestId, '--identity', 'Reader'])).exchange
      ).toMatchObject({ acknowledged: true, settled: true });
      expect(readFileSync(log, 'utf8')).toBe('\n');
    });
  });

  it('keeps retired-name history with its original UUID rather than the replacement identity', async () => {
    await withSandbox(async (sandbox) => {
      await calibrateTmuxTripwire(sandbox);
      const owner = (await json(sandbox, ['identity', 'create', 'Reusable'])).identity as {
        id: string;
      };
      attentionFixture(sandbox, owner.id, 'old-owner-request');
      const before = responseSnapshot(sandbox.database, 'old-owner-request');
      await json(sandbox, ['rm', 'Reusable', '--force']);
      const replacement = (await json(sandbox, ['identity', 'create', 'Reusable'])).identity as {
        id: string;
      };
      expect(replacement.id).not.toBe(owner.id);
      expect(await json(sandbox, ['x', '--identity', 'Reusable'])).toMatchObject({ items: [] });
      expect(await json(sandbox, ['x', 'ackall', '--identity', 'Reusable'])).toMatchObject({
        acknowledgedThrough: 0,
      });
      const hidden = await runCli(sandbox, [
        'x',
        'show',
        'old-owner-request',
        '--identity',
        'Reusable',
        '--json',
      ]);
      expect(hidden.status).toBe(3);
      expectError(hidden, 'X_NOT_FOUND');
      expect(responseSnapshot(sandbox.database, 'old-owner-request')).toEqual(before);
      const oracle = new Database(sandbox.database, { readonly: true });
      try {
        expect(oracle.prepare('SELECT originator_identity_id FROM request_attempts').get()).toEqual(
          { originator_identity_id: owner.id }
        );
        expect(
          oracle
            .prepare('SELECT identity_id, acknowledged_through FROM request_attention_identities')
            .all()
        ).toEqual([{ identity_id: owner.id, acknowledged_through: 0 }]);
      } finally {
        oracle.close();
      }
    });
  });

  it('rejects an unverified implicit caller before request housekeeping', async () => {
    await withSandbox(async (sandbox) => {
      await calibrateTmuxTripwire(sandbox);
      await json(sandbox, ['identity', 'create', 'Offline']);
      seedResponse(sandbox.database, 'unread-expired-prompt');
      const writer = new Database(sandbox.database);
      try {
        writer
          .prepare('UPDATE request_attempts SET message_expires_at_ms = ? WHERE request_id = ?')
          .run(Date.now() - 1000, 'unread-expired-prompt');
      } finally {
        writer.close();
      }
      const before = responseSnapshot(sandbox.database, 'unread-expired-prompt');
      for (const operation of [[], ['show', 'unread-expired-prompt'], ['ackall']]) {
        const result = await runCli(sandbox, ['x', ...operation, '--json']);
        expect(result.status).toBe(1);
        expectError(result, 'IDENTITY_REQUIRED');
        expect(responseSnapshot(sandbox.database, 'unread-expired-prompt')).toEqual(before);
      }
      // Calibrate the housekeeping oracle: valid explicit access must scrub it.
      await json(sandbox, ['x', '--identity', 'Offline']);
      expect(
        responseSnapshot(sandbox.database, 'unread-expired-prompt').attempt?.message_text
      ).toBeNull();
    });
  });

  it('uses explicit offline identity without config or tmux and keeps reads separate from acknowledgment', async () => {
    await withSandbox(async (sandbox) => {
      const log = await calibrateTmuxTripwire(sandbox);
      const created = await json(sandbox, ['identity', 'create', 'Owner']);
      const identity = created.identity as {
        id: string;
        name: string;
        canonicalName: string;
        lifetime: string;
      };
      expect(identity).toMatchObject({
        id: expect.any(String),
        name: 'Owner',
        canonicalName: 'owner',
        lifetime: 'saved',
      });
      const seeded = attentionFixture(sandbox, identity.id, 'native-attention');
      writeFileSync(sandbox.globalConfig, '{ invalid config');
      writeFileSync(sandbox.localConfig, '{ invalid config');
      const listed = await json(sandbox, ['x', '--identity', 'OWNER']);
      expect(listed).toMatchObject({
        identity,
        nextAfter: null,
        items: [
          {
            requestId: seeded.requestId,
            revision: 1,
            delivery: 'sent',
            final: { status: 'not_submitted' },
            acknowledged: false,
            settled: false,
          },
        ],
      });
      const shown = await json(sandbox, ['x', 'show', seeded.requestId, '--identity', 'Owner']);
      expect(shown).toMatchObject({
        identity,
        exchange: {
          revision: 1,
          prompt: { status: 'retained', message: `prompt for ${seeded.requestId}` },
        },
      });
      expect(await json(sandbox, ['x', '--identity', 'Owner'])).toEqual(listed);
      expect(await json(sandbox, ['x', 'ackall', '--identity', 'Owner'])).toEqual({
        identity,
        acknowledgedThrough: 1,
      });
      expect(await json(sandbox, ['x', '--identity', 'Owner'])).toEqual({
        identity,
        items: [],
        nextAfter: null,
      });
      const body = '\uFEFF exact final\r\n日本語  ';
      await json(sandbox, [
        'reply',
        seeded.requestId,
        '--receipt',
        seeded.compactReceipt,
        '--message',
        body,
      ]);
      const after = await json(sandbox, ['x', 'list', '--identity', 'Owner']);
      expect(after).toMatchObject({
        items: [
          {
            revision: 2,
            acknowledged: false,
            settled: false,
            final: { status: 'retained', bodyBytes: Buffer.byteLength(body) },
          },
        ],
      });
      expect(JSON.stringify(after)).not.toContain('exact final');
      expect(JSON.stringify(after)).not.toContain('prompt for');
      expect(JSON.stringify(after)).not.toContain('receipt');
      const stale = await runCli(sandbox, [
        'x',
        'ack',
        seeded.requestId,
        '--revision',
        '1',
        '--identity',
        'Owner',
        '--json',
      ]);
      expect(stale.status).toBe(5);
      expectError(stale, 'X_REVISION_CONFLICT');
      expect(
        await json(sandbox, [
          'x',
          'ack',
          seeded.requestId,
          '--revision',
          '2',
          '--identity',
          'Owner',
        ])
      ).toEqual({
        identity,
        requestId: seeded.requestId,
        revision: 2,
        acknowledged: true,
        changed: true,
      });
      expect(
        await json(sandbox, [
          'x',
          'ack',
          seeded.requestId,
          '--revision',
          '2',
          '--identity',
          'Owner',
        ])
      ).toMatchObject({ changed: false });
      expect(
        await json(sandbox, ['x', 'show', seeded.requestId, '--identity', 'Owner'])
      ).toMatchObject({
        exchange: {
          acknowledged: true,
          settled: true,
          final: { status: 'retained', response: body },
        },
      });
      expect(readFileSync(log, 'utf8')).toBe('\n');
      const oracle = new Database(sandbox.database, { readonly: true });
      try {
        expect(
          oracle
            .prepare(
              'SELECT latest_revision, acknowledged_through FROM request_attention_identities'
            )
            .get()
        ).toEqual({ latest_revision: 2, acknowledged_through: 1 });
        expect(
          oracle
            .prepare(
              'SELECT attention_revision, attention_acknowledged_revision FROM request_attempts'
            )
            .get()
        ).toEqual({ attention_revision: 2, attention_acknowledged_revision: 2 });
      } finally {
        oracle.close();
      }
    });
  });

  it('rejects unknown and foreign selectors without creating identities or acknowledging another owner', async () => {
    await withSandbox(async (sandbox) => {
      const log = await calibrateTmuxTripwire(sandbox);
      const owner = (await json(sandbox, ['identity', 'create', 'Owner'])).identity as {
        id: string;
      };
      expect(owner.id).toEqual(expect.any(String));
      await json(sandbox, ['identity', 'create', 'Foreign']);
      attentionFixture(sandbox, owner.id, 'private-exchange');
      const identitiesBefore = await json(sandbox, ['identity', 'list']);
      const before = responseSnapshot(sandbox.database, 'private-exchange');
      for (const operation of [
        ['show', 'private-exchange'],
        ['ack', 'private-exchange', '--revision', '1'],
      ]) {
        const result = await runCli(sandbox, [
          'x',
          ...operation,
          '--identity',
          'Foreign',
          '--json',
        ]);
        expect(result.status).toBe(3);
        expectError(result, 'X_NOT_FOUND');
      }
      const missing = await runCli(sandbox, [
        'x',
        'ackall',
        '--identity',
        'NeverCreated',
        '--json',
      ]);
      expect(missing.status).toBe(3);
      expectError(missing, 'NAME_NOT_FOUND');
      expect(await json(sandbox, ['x', 'ackall', '--identity', 'Foreign'])).toMatchObject({
        acknowledgedThrough: 0,
      });
      expect(responseSnapshot(sandbox.database, 'private-exchange')).toEqual(before);
      expect(await json(sandbox, ['identity', 'list'])).toEqual(identitiesBefore);
      expect(readFileSync(log, 'utf8')).toBe('\n');
    });
  });
});
