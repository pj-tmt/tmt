import Database from 'better-sqlite3';
import { randomUUID } from 'node:crypto';
import { chmodSync, existsSync, mkdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { runCli, withSandbox, type Sandbox } from '../support/cli-process.js';

async function api(sandbox: Sandbox, operation: string, input: unknown, identity?: string) {
  const result = await runCli(sandbox, ['api'], {
    stdin: JSON.stringify({ version: 1, operation, input, ...(identity ? { identity } : {}) }),
  });
  expect(result.stderr).toBe('');
  return { ...result, body: JSON.parse(result.stdout) };
}

async function identity(sandbox: Sandbox, name: string): Promise<string> {
  const result = await runCli(sandbox, ['identity', 'create', name, '--json']);
  expect(result.status).toBe(0);
  return JSON.parse(result.stdout).identity.id;
}

function snapshot(sandbox: Sandbox) {
  const db = new Database(sandbox.database, { readonly: true });
  try {
    return db.prepare('SELECT * FROM request_attempts ORDER BY request_id').all();
  } finally {
    db.close();
  }
}

// Independent oracle: every user table's rows, read without the implementation.
function everyTable(sandbox: Sandbox) {
  const db = new Database(sandbox.database, { readonly: true });
  try {
    const tables = db
      .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
      .all() as { name: string }[];
    return Object.fromEntries(
      tables.map(({ name }) => [name, db.prepare(`SELECT * FROM "${name}"`).all()])
    );
  } finally {
    db.close();
  }
}

describe('public local extension API', () => {
  it('discovers capabilities through a non-Office extension without creating state', async () => {
    await withSandbox(async (sandbox) => {
      const bin = path.join(sandbox.root, 'bin');
      mkdirSync(bin);
      const extension = path.join(bin, 'tmt-teamchat');
      writeFileSync(extension, '#!/bin/sh\nexec "$TMT_EXECUTABLE" api\n');
      chmodSync(extension, 0o700);
      sandbox.env.PATH = `${bin}${path.delimiter}${sandbox.env.PATH ?? ''}`;
      const request = JSON.stringify({ version: 1, operation: 'capabilities', input: {} });
      const result = await runCli(sandbox, ['teamchat'], { stdin: request });
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      expect(JSON.parse(result.stdout)).toMatchObject({
        version: 1,
        supported: { min: 1, max: 1 },
        operations: expect.arrayContaining(['requests.list', 'dispatch.create', 'rooms.write']),
      });
      for (const body of [
        '{',
        JSON.stringify({ version: 1, operation: 'capabilities', input: {}, extra: true }),
        '{"version":1,"version":1,"operation":"capabilities","input":{}}',
        JSON.stringify({ version: 1, operation: 'dispatch.create', input: {} }),
      ]) {
        const invalid = await runCli(sandbox, ['api'], { stdin: body });
        expect(invalid.status).toBe(1);
        expect(JSON.parse(invalid.stdout).error.code).toBe('API_INPUT_INVALID');
      }
      const unsupported = await runCli(sandbox, ['api'], {
        stdin: JSON.stringify({ version: 99, operation: 'capabilities', input: {} }),
      });
      expect(unsupported.status).toBe(1);
      expect(JSON.parse(unsupported.stdout).error).toMatchObject({
        code: 'API_VERSION_UNSUPPORTED',
        supported: { min: 1, max: 1 },
      });
      expect(existsSync(sandbox.globalDir)).toBe(false);
    });
  });

  it('shares room resources and rejects stale or unattributed writes without mutation', async () => {
    await withSandbox(async (sandbox) => {
      const alice = await identity(sandbox, 'Alice');
      const roomId = randomUUID();
      const input = { roomId, room: { expectedRevision: 0, name: 'Team', memberIds: [alice] } };
      expect((await api(sandbox, 'rooms.write', input)).status).toBe(1);
      expect((await api(sandbox, 'rooms.write', input, 'missing')).body.error.code).toBe(
        'NAME_NOT_FOUND'
      );
      const created = await api(sandbox, 'rooms.write', input, 'Alice');
      expect(created.status).toBe(0);
      expect(created.body).toEqual({
        id: roomId,
        name: 'Team',
        revision: 1,
        retired: false,
        memberIds: [alice],
      });
      const stale = await api(
        sandbox,
        'rooms.write',
        { ...input, room: { ...input.room, name: 'Wrong' } },
        alice
      );
      expect(stale.status).toBe(1);
      const shown = await runCli(sandbox, ['room', 'show', roomId, '--json']);
      expect(shown.status).toBe(0);
      expect(JSON.parse(shown.stdout).room).toEqual(created.body);
    });
  });

  it('bounds incomplete and oversized input before opening storage', async () => {
    await withSandbox(async (sandbox) => {
      const discovery = await api(sandbox, 'capabilities', {});
      const oversized = await runCli(sandbox, ['api'], {
        stdin: ' '.repeat(discovery.body.limits.inputBytes + 1),
      });
      expect(oversized.status).toBe(1);
      expect(JSON.parse(oversized.stdout).error.code).toBe('API_INPUT_INVALID');
      const unfinished = await runCli(sandbox, ['api'], {
        stdin: '{',
        closeStdin: false,
        deadlineMs: 8_000,
      });
      expect(unfinished.status).toBe(1);
      expect(JSON.parse(unfinished.stdout).error.code).toBe('API_INPUT_TIMEOUT');
      expect(existsSync(sandbox.globalDir)).toBe(false);
    });
  });

  it('recovers immutable dispatches and pages concurrent history without acknowledging work', async () => {
    await withSandbox(async (sandbox) => {
      const sender = await identity(sandbox, 'Sender');
      const recipient = await identity(sandbox, 'Recipient');
      const input = {
        operationId: randomUUID(),
        recipientIds: [recipient],
        message: 'Exact\nmessage',
      };
      const first = await api(sandbox, 'dispatch.create', input, sender);
      expect(first.status).toBe(0);
      expect(first.body.wake).toMatchObject({
        status: 'unavailable',
        paneAttempted: false,
        agentProcessed: null,
      });
      expect(first.body.items).toEqual([
        { recipientId: recipient, requestId: expect.any(String), acceptance: 'queued' },
      ]);
      const { wake: _wake, ...receipt } = first.body;
      const acceptedRows = snapshot(sandbox);
      const replay = await api(sandbox, 'dispatch.create', input, sender);
      expect(replay.status).toBe(0);
      expect(replay.body).toEqual(receipt);
      expect(snapshot(sandbox)).toEqual(acceptedRows);
      expect(
        (await api(sandbox, 'dispatch.create', { ...input, message: 'Changed' }, sender)).status
      ).toBe(1);
      expect(
        (await api(sandbox, 'dispatch.show', { operationId: input.operationId })).body
      ).toEqual(receipt);
      await api(
        sandbox,
        'dispatch.create',
        { ...input, operationId: randomUUID(), message: 'Second' },
        sender
      );
      const page = await api(sandbox, 'requests.list', { recipientId: recipient, limit: 1 });
      expect(page.status).toBe(0);
      expect(page.body.items).toHaveLength(1);
      expect(page.body.nextBefore).not.toBeNull();
      await api(
        sandbox,
        'dispatch.create',
        { ...input, operationId: randomUUID(), message: 'Newer' },
        sender
      );
      const incoming = await runCli(sandbox, [
        'x',
        'show',
        receipt.items[0].requestId,
        '--incoming',
        '--identity',
        recipient,
        '--json',
      ]);
      expect(incoming.status).toBe(0);
      const replied = await runCli(sandbox, [
        'reply',
        receipt.items[0].requestId,
        '--receipt',
        JSON.parse(incoming.stdout).exchange.reply.receipt,
        '--message',
        'Final after first page',
        '--json',
      ]);
      expect(replied.status).toBe(0);
      const before = snapshot(sandbox);
      const next = await api(sandbox, 'requests.list', {
        recipientId: recipient,
        limit: 1,
        before: page.body.nextBefore,
      });
      expect(next.body.items[0].requestId).toBe(receipt.items[0].requestId);
      const detail = await api(sandbox, 'requests.show', { requestId: receipt.items[0].requestId });
      expect(detail.status).toBe(0);
      expect(detail.body.prompt).toMatchObject({ status: 'retained', message: input.message });
      expect(detail.body.sender).toMatchObject({ identityId: sender });
      expect(detail.body.final).toMatchObject({
        status: 'retained',
        response: 'Final after first page',
      });
      expect(snapshot(sandbox)).toEqual(before);
    });
  });

  it('projects one room roster exactly like the ordinary reads, without mutation', async () => {
    await withSandbox(async (sandbox) => {
      const json = async (args: string[]) => {
        const result = await runCli(sandbox, [...args, '--json']);
        expect(result.status, args.join(' ')).toBe(0);
        return JSON.parse(result.stdout);
      };
      const alice = await identity(sandbox, 'Alice');
      const bob = await identity(sandbox, 'Bob');
      const outsider = await identity(sandbox, 'Carol');
      await json(['room', 'create', 'Squad']);
      for (const member of [alice, bob])
        await json(['room', 'join', 'Squad', '--identity', member]);
      await json(['identity', 'meta', 'set', 'squad.a.state', 'review', '--identity', alice]);
      await json(['identity', 'meta', 'set', 'squad.ab.state', 'other', '--identity', alice]);
      await json(['identity', 'meta', 'set', 'squad.a.state', 'hidden', '--identity', outsider]);
      await json(['identity', 'status', 'set', 'Reviewing', '--identity', alice]);
      await json(['identity', 'status', 'set', 'Parked', '--identity', bob, '--for', '1s']);
      // Bounded poll for the one-second minimum expiry; reads never renew status.
      const deadline = Date.now() + 5_000;
      while (!(await json(['identity', 'status', 'show', '--identity', bob])).status.stale) {
        expect(Date.now()).toBeLessThan(deadline);
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
      const before = everyTable(sandbox);

      const roster = await api(sandbox, 'rooms.roster', {
        room: 'Squad',
        metadataPrefix: 'squad.a.',
      });
      expect(roster.status).toBe(0);
      expect(roster.body.room).toEqual((await json(['room', 'show', 'Squad'])).room);
      expect(roster.body.members.map((member: { id: string }) => member.id)).toEqual(
        roster.body.room.memberIds
      );
      for (const member of roster.body.members) {
        const { metadata, status, ...summary } = member;
        expect(summary).toEqual((await json(['identity', 'show', member.id])).identity);
        const all = (await json(['identity', 'meta', 'list', '--identity', member.id])).metadata;
        expect(metadata).toEqual(
          Object.fromEntries(Object.entries(all).filter(([key]) => key.startsWith('squad.a.')))
        );
        expect(status).toEqual(
          (await json(['identity', 'status', 'show', '--identity', member.id])).status
        );
      }
      const byId = new Map(roster.body.members.map((m: { id: string }) => [m.id, m]));
      expect(byId.get(alice)).toMatchObject({ metadata: { 'squad.a.state': 'review' } });
      expect(byId.get(bob)).toMatchObject({ metadata: {}, status: { stale: true } });

      const missing = await api(sandbox, 'rooms.roster', { room: 'Absent' });
      expect(missing.status).toBe(1);
      expect(missing.body.error.code).toBe('ROOM_NOT_FOUND');
      expect(everyTable(sandbox)).toEqual(before);
    });
  });

  it('scopes identity-retirement hooks to their consumer through the public API', async () => {
    await withSandbox(async (sandbox) => {
      const ada = await identity(sandbox, 'Ada');
      const hook = (consumer: string) => ({ consumer, identityId: ada, reference: 'scope-a' });
      expect((await api(sandbox, 'identityHooks.register', hook('tmt-office'))).body).toEqual({
        state: 'registered',
      });
      expect(
        (await api(sandbox, 'identityHooks.pending', { consumer: 'tmt-office', limit: 16 })).body
      ).toEqual({ hooks: [], pending: 0 });
      const early = await api(sandbox, 'identityHooks.attempt', hook('tmt-office'));
      expect(early.status).toBe(1);
      expect(early.body.error.code).toBe('HOOK_NOT_PENDING');

      const retire = await runCli(sandbox, ['rm', 'Ada', '--force', '--json']);
      expect(retire.status, retire.stdout + retire.stderr).toBe(0);
      // Registration after retirement is pending at once.
      expect((await api(sandbox, 'identityHooks.register', hook('other'))).body).toEqual({
        state: 'pending',
      });
      const page = await api(sandbox, 'identityHooks.pending', {
        consumer: 'tmt-office',
        limit: 16,
      });
      expect(page.body).toEqual({
        hooks: [{ identityId: ada, reference: 'scope-a', attemptCount: 0 }],
        pending: 1,
      });
      const foreign = await api(sandbox, 'identityHooks.ack', hook('third'));
      expect(foreign.body.error.code).toBe('HOOK_NOT_FOUND');
      expect((await api(sandbox, 'identityHooks.attempt', hook('tmt-office'))).body).toEqual({
        recorded: true,
      });
      expect((await api(sandbox, 'identityHooks.ack', hook('tmt-office'))).body).toEqual({
        acknowledged: true,
      });
      expect((await api(sandbox, 'identityHooks.ack', hook('tmt-office'))).body).toEqual({
        acknowledged: false,
      });
      expect(
        (await api(sandbox, 'identityHooks.pending', { consumer: 'other', limit: 16 })).body.pending
      ).toBe(1);
    });
  });

  it('reads only saved-identity notebooks without creating missing files', async () => {
    await withSandbox(async (sandbox) => {
      const owner = await identity(sandbox, 'Writer');
      const file = path.join(sandbox.globalDir, 'notes', owner, 'notes.md');
      expect((await api(sandbox, 'notes.read', { identityId: owner })).body.error.code).toBe(
        'NOTEBOOK_NOT_FOUND'
      );
      expect(existsSync(file)).toBe(false);
      const initialized = await runCli(sandbox, ['notes', 'path', '--identity', owner, '--json']);
      expect(initialized.status).toBe(0);
      const content = 'Exact notes\r\nsecond line\n';
      writeFileSync(file, content);
      const note = await api(sandbox, 'notes.read', { identityId: owner });
      expect(note.status).toBe(0);
      expect(note.body).toEqual({ identityId: owner, name: 'Writer', content });
      expect((await api(sandbox, 'notes.read', { identityId: '../notes.md' })).status).toBe(1);
      writeFileSync(file, 'x'.repeat(1_048_577));
      expect((await api(sandbox, 'notes.read', { identityId: owner })).body.error.code).toBe(
        'NOTEBOOK_TOO_LARGE'
      );
    });
  });
});
