import Database from 'better-sqlite3';
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import { parseWholeStdout, runCli, withSandbox, type Sandbox } from '../support/cli-process.js';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';

interface Room {
  id: string;
  name: string;
  revision: number;
  retired: boolean;
  memberIds: string[];
}

interface DispatchReceipt {
  operationId: string;
  createdAtMs: number;
  items: { recipientId: string; requestId: string; acceptance: string }[];
}

async function json(sandbox: Sandbox, args: string[]) {
  const result = await runCli(sandbox, [...args, '--json']);
  expect(result.status).toBe(0);
  expect(result.stderr).toBe('');
  return parseWholeStdout(result);
}

async function room(sandbox: Sandbox, args: string[]): Promise<Room> {
  const result = await json(sandbox, ['room', ...args]);
  expect(result.room).toEqual({
    id: expect.stringMatching(/^[0-9a-f-]{36}$/),
    name: expect.any(String),
    revision: expect.any(Number),
    retired: expect.any(Boolean),
    memberIds: expect.any(Array),
  });
  return result.room as Room;
}

describe('shared local rooms without Office or tmux', () => {
  it('retires without deleting the roster and rejects new routing or reusing the old UUID', async () => {
    await withSandbox(async (sandbox) => {
      await json(sandbox, ['identity', 'create', 'Alice']);
      const original = await room(sandbox, ['create', 'Design']);
      const joined = await room(sandbox, ['join', original.id, '--identity', 'Alice']);
      const retired = await room(sandbox, ['retire', original.id]);
      expect(retired).toEqual({ ...joined, retired: true, revision: joined.revision + 1 });
      expect(await room(sandbox, ['retire', original.id])).toEqual(retired);
      expect(await room(sandbox, ['show', original.id])).toEqual(retired);
      expect(await json(sandbox, ['room', 'ls'])).toEqual({ rooms: [] });
      for (const args of [
        ['room', 'show', 'Design'],
        ['room', 'send', original.id, 'No new request', '--identity', 'Alice'],
        ['room', 'broadcast', original.id, 'No new notice', '--identity', 'Alice'],
        ['room', 'join', original.id, '--identity', 'Alice'],
        ['ls', '--room', original.id],
      ]) {
        const result = await runCli(sandbox, [...args, '--json']);
        expect(result.status).toBe(3);
        expect(result.stdout).toContain('ROOM_NOT_FOUND');
      }
      const replacement = await room(sandbox, ['create', 'Design']);
      expect(replacement.id).not.toBe(retired.id);
      expect(replacement.memberIds).toEqual([]);
      expect(await room(sandbox, ['show', 'Design'])).toEqual(replacement);
    });
  });
  it('scopes direct talk to one member and rejects outsiders without fan-out', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      for (const name of ['Sender', 'Alice', 'Bob', 'Outside'])
        await json(sandbox, ['identity', 'create', name]);
      const design = await room(sandbox, ['create', 'Design']);
      for (const name of ['Alice', 'Bob'])
        await room(sandbox, ['join', design.id, '--identity', name]);
      const options = ['--room', design.id, '--inbox', '--detach', '--identity', 'Sender'];
      const accepted = await json(sandbox, [
        'talk',
        'Alice',
        'Only Alice should receive this',
        ...options,
      ]);
      const rejected = await runCli(sandbox, [
        'talk',
        'Outside',
        'No work for outsiders',
        ...options,
        '--json',
      ]);
      expect(rejected.status).toBe(1);
      expect(parseWholeStdout(rejected)).toMatchObject({
        error: { code: 'ROOM_RECIPIENT_NOT_MEMBER' },
      });
      const received = await json(sandbox, [
        'x',
        'show',
        String(accepted.requestId),
        '--incoming',
        '--identity',
        'Alice',
      ]);
      expect(received.exchange).toMatchObject({
        roomId: design.id,
        prompt: { message: 'Only Alice should receive this' },
      });
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(db.prepare('SELECT count(*) AS count FROM request_attempts').get()).toEqual({
          count: 1,
        });
        expect(db.prepare('SELECT room_id FROM request_attempts').all()).toEqual([
          { room_id: design.id },
        ]);
      } finally {
        db.close();
      }
      expect(readFileSync(tripwire, 'utf8')).toBe('\n');
    });
  });
  it.each(['leave', 'retire'])(
    'listens and replies to historical room context after %s, without consuming another room',
    async (change) => {
      await withSandbox(async (sandbox) => {
        const tripwire = await calibrateTmuxTripwire(sandbox);
        await json(sandbox, ['identity', 'create', 'Alice']);
        await json(sandbox, ['identity', 'create', 'Sender']);
        const design = await room(sandbox, ['create', 'Design']);
        const review = await room(sandbox, ['create', 'Review']);
        await room(sandbox, ['join', design.id, '--identity', 'Alice']);
        await room(sandbox, ['join', review.id, '--identity', 'Alice']);
        const sentA = (await json(sandbox, [
          'room',
          'send',
          design.id,
          'Design question',
          '--identity',
          'Sender',
        ])) as unknown as DispatchReceipt;
        const sentB = (await json(sandbox, [
          'room',
          'send',
          review.id,
          'Review question',
          '--identity',
          'Sender',
        ])) as unknown as DispatchReceipt;
        expect(sentA.items).toHaveLength(1);
        expect(sentB.items).toHaveLength(1);
        const [a] = sentA.items;
        const [b] = sentB.items;
        await room(
          sandbox,
          change === 'leave' ? ['leave', design.id, '--identity', 'Alice'] : ['retire', design.id]
        );
        const listen = [
          'x',
          'listen',
          '--identity',
          'Alice',
          '--timeout',
          '100ms',
          '--debounce',
          '1ms',
        ];
        const incoming = await json(sandbox, [...listen, '--room', design.id]);
        expect(incoming).toMatchObject({
          reason: 'messages',
          items: [{ requestId: a.requestId, roomId: design.id }],
        });
        expect(incoming.items).toHaveLength(1);
        const detail = await json(sandbox, [
          'x',
          'show',
          String(a.requestId),
          '--incoming',
          '--identity',
          'Alice',
        ]);
        const exchange = detail.exchange as {
          roomId: string;
          revision: number;
          reply: { receipt: string };
        };
        expect(exchange.roomId).toBe(design.id);
        await json(sandbox, [
          'reply',
          String(a.requestId),
          '--receipt',
          exchange.reply.receipt,
          '--message',
          'Done after leaving',
        ]);
        expect(await json(sandbox, ['result', a.requestId])).toMatchObject({
          response: 'Done after leaving',
        });
        const senderAttention = await json(sandbox, ['x', '--identity', 'Sender']);
        expect(senderAttention.items).toEqual(
          expect.arrayContaining([expect.objectContaining({ requestId: a.requestId })])
        );
        await json(sandbox, [
          'x',
          'ack',
          String(a.requestId),
          '--incoming',
          '--identity',
          'Alice',
          '--revision',
          String(exchange.revision),
        ]);
        const quiet = await json(sandbox, [
          ...listen,
          '--room',
          change === 'retire' ? design.id : 'Design',
        ]);
        expect(quiet).toMatchObject({ reason: 'timeout', items: [] });
        const remaining = await json(sandbox, [...listen, '--room', 'Review']);
        expect(remaining).toMatchObject({
          reason: 'messages',
          items: [{ requestId: b.requestId, roomId: review.id }],
        });
        expect(remaining.items).toHaveLength(1);
        expect(readFileSync(tripwire, 'utf8')).toBe('\n');
      });
    }
  );

  it('queues a frozen audience, safely replays, and distinguishes no-reply broadcasts', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      for (const name of ['Sender', 'Alice', 'Bob'])
        await json(sandbox, ['identity', 'create', name]);
      const design = await room(sandbox, ['create', 'Design']);
      const send = [
        'room',
        'send',
        design.id,
        'Review together',
        '--identity',
        'Sender',
        '--operation-id',
        '11111111-1111-4111-8111-111111111111',
      ];
      const empty = await runCli(sandbox, [...send, '--json']);
      expect(empty.status).toBe(1);
      expect(parseWholeStdout(empty)).toMatchObject({ error: { code: 'ROOM_EMPTY' } });
      for (const name of ['Alice', 'Bob'])
        await room(sandbox, ['join', design.id, '--identity', name]);
      const accepted = (await json(sandbox, send)) as unknown as DispatchReceipt;
      expect(accepted.items).toHaveLength(2);
      expect(accepted.items.every((item) => item.acceptance === 'queued')).toBe(true);
      expect(await json(sandbox, send)).toEqual(accepted);
      await room(sandbox, ['leave', design.id, '--identity', 'Bob']);
      const changed = await runCli(sandbox, [...send, '--json']);
      expect(changed.status).toBe(1);
      expect(parseWholeStdout(changed)).toMatchObject({
        error: { code: 'DISPATCH_IDEMPOTENCY_CONFLICT' },
      });
      const announcement = (await json(sandbox, [
        'room',
        'broadcast',
        design.id,
        'Review starts now',
        '--identity',
        'Sender',
      ])) as unknown as DispatchReceipt;
      expect(announcement.items).toHaveLength(1);
      const announcementId = announcement.items[0].requestId;
      expect(await json(sandbox, ['result', announcementId])).toMatchObject({
        status: 'not_required',
      });
      const received = await json(sandbox, [
        'x',
        'show',
        announcementId,
        '--incoming',
        '--identity',
        'Alice',
      ]);
      expect(received.exchange).toMatchObject({
        roomId: design.id,
        final: { status: 'not_required' },
      });
      expect(received.exchange).not.toHaveProperty('reply');
      const activity = await json(sandbox, [
        'x',
        'listen',
        '--identity',
        'Alice',
        '--room',
        design.id,
        '--timeout',
        '100ms',
        '--debounce',
        '1ms',
      ]);
      expect(activity.items).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            requestId: announcementId,
            roomId: design.id,
            kind: 'announcement',
          }),
        ])
      );
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(db.prepare('SELECT count(*) AS count FROM request_attempts').get()).toEqual({
          count: 3,
        });
        expect(db.prepare('SELECT DISTINCT room_id FROM request_attempts').all()).toEqual([
          { room_id: design.id },
        ]);
        expect(db.prepare('SELECT DISTINCT originator_kind FROM request_attempts').all()).toEqual([
          { originator_kind: 'explicit' },
        ]);
      } finally {
        db.close();
      }
      expect(readFileSync(tripwire, 'utf8')).toBe('\n');
    });
  });

  it('persists independent memberships and filters the existing identity listing', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      const alice = (await json(sandbox, ['identity', 'create', 'Alice'])).identity as {
        id: string;
      };
      const bob = (await json(sandbox, ['identity', 'create', 'Bob'])).identity as { id: string };
      const design = await room(sandbox, ['create', 'Design']);
      const review = await room(sandbox, ['create', 'Review']);
      expect(design).toMatchObject({ name: 'Design', revision: 1, memberIds: [] });
      const joined = await room(sandbox, ['join', 'Design', '--identity', 'Alice']);
      expect(joined).toMatchObject({ revision: 2, memberIds: [alice.id] });
      expect(await room(sandbox, ['join', design.id, '--identity', 'Alice'])).toEqual(joined);
      await room(sandbox, ['join', review.id, '--identity', 'Alice']);
      await room(sandbox, ['join', review.id, '--identity', 'Bob']);
      expect((await json(sandbox, ['ls', '--room', design.id])).identities).toMatchObject([
        { id: alice.id, presence: 'offline' },
      ]);
      expect((await json(sandbox, ['ls', '--room', review.id])).identities).toHaveLength(2);
      const left = await room(sandbox, ['leave', 'Design', '--identity', 'Alice']);
      expect(left).toMatchObject({ revision: 3, memberIds: [] });
      expect(await room(sandbox, ['leave', design.id, '--identity', 'Alice'])).toEqual(left);
      expect((await room(sandbox, ['show', 'Review'])).memberIds).toEqual(
        [alice.id, bob.id].sort()
      );
      expect((await json(sandbox, ['ls', '--room', 'Design'])).identities).toEqual([]);
      expect((await json(sandbox, ['ls'])).identities).toHaveLength(2);
      // Each CLI invocation reopened storage. Independent SQL proves no Office setup occurred.
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(
          db
            .prepare('SELECT room_id,identity_id FROM office_meeting_members ORDER BY identity_id')
            .all()
        ).toEqual([alice.id, bob.id].sort().map((id) => ({ room_id: review.id, identity_id: id })));
      } finally {
        db.close();
      }
      expect(readFileSync(tripwire, 'utf8')).toBe('\n');
    });
  });

  it('rejects ambiguous labels, missing identities and malformed commands without roster changes', async () => {
    await withSandbox(async (sandbox) => {
      const tripwire = await calibrateTmuxTripwire(sandbox);
      const first = await room(sandbox, ['create', 'Design']);
      const second = await room(sandbox, ['create', 'Design']);
      const ambiguous = await runCli(sandbox, ['room', 'show', 'Design', '--json']);
      expect(ambiguous.status).toBe(1);
      expect(ambiguous.stdout).toContain('ROOM_AMBIGUOUS');
      expect(ambiguous.stdout).toContain(first.id);
      expect(ambiguous.stdout).toContain(second.id);
      const missing = await runCli(sandbox, [
        'room',
        'join',
        first.id,
        '--identity',
        'Missing',
        '--json',
      ]);
      expect(missing.status).toBe(3);
      expect(missing.stdout).toContain('NAME_NOT_FOUND');
      const invalid = await runCli(sandbox, ['ls', 'Alice', '--room', first.id, '--json']);
      expect(invalid.status).toBe(1);
      expect(invalid.stdout).toContain('USAGE_ERROR');
      expect(await room(sandbox, ['show', first.id])).toEqual(first);
      expect(await room(sandbox, ['show', second.id])).toEqual(second);
      expect(readFileSync(tripwire, 'utf8')).toBe('\n');
    });
  });
});
