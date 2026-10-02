import Database from 'better-sqlite3';
import { readFileSync } from 'node:fs';
import { expect, it } from 'vite-plus/test';
import {
  expectError,
  parseWholeStdout,
  runCli,
  withSandbox,
  type Sandbox,
} from '../support/cli-process.js';
import { calibrateTmuxTripwire } from './tmux-tripwire.js';

interface Status {
  activity: string;
  mood: string | null;
  updatedAtMs: number;
  expiresAtMs: number;
  stale: boolean;
}
async function json(sandbox: Sandbox, args: string[]) {
  const result = await runCli(sandbox, [...args, '--json']);
  expect(result.status, result.stdout + result.stderr).toBe(0);
  expect(result.stderr).toBe('');
  return parseWholeStdout(result);
}
function stored(file: string) {
  const database = new Database(file, { readonly: true });
  try {
    return {
      identities: database.prepare('SELECT * FROM identities ORDER BY id').all(),
      status: database.prepare('SELECT * FROM identity_status ORDER BY identity_id').all(),
      requests: database.prepare('SELECT count(*) FROM request_attempts').pluck().get(),
      profiles: database.prepare('SELECT count(*) FROM office_local_profiles').pluck().get(),
    };
  } finally {
    database.close();
  }
}

it('uses the same identity across native processes without tmux, appearance writes or implicit lifetime changes', async () => {
  await withSandbox(async (sandbox) => {
    const tripwire = await calibrateTmuxTripwire(sandbox);
    const identity = (await json(sandbox, ['identity', 'create', 'Alice'])).identity as {
      id: string;
    };
    const args = ['identity', 'status'];
    const selected = ['--identity', 'Alice'];
    expect(await json(sandbox, [...args, 'show', ...selected])).toEqual({
      identityId: identity.id,
      status: null,
    });
    const before = stored(sandbox.database);
    const first = await json(sandbox, [
      ...args,
      'set',
      'Reviewing the renderer',
      '--mood',
      'focused',
      ...selected,
    ]);
    expect(first).toEqual({
      identityId: identity.id,
      status: {
        activity: 'Reviewing the renderer',
        mood: 'focused',
        updatedAtMs: expect.any(Number),
        expiresAtMs: expect.any(Number),
        stale: false,
      },
    });
    const status = first.status as Status;
    expect(status.expiresAtMs - status.updatedAtMs).toBe(3_600_000);
    expect(await json(sandbox, [...args, 'show', ...selected])).toEqual(first);
    const after = stored(sandbox.database);
    expect(after.identities).toEqual(before.identities);
    expect(after.requests).toBe(before.requests);
    expect(after.profiles).toBe(before.profiles);
    expect(after.status).toEqual([
      {
        identity_id: identity.id,
        activity: status.activity,
        mood: status.mood,
        updated_at_ms: status.updatedAtMs,
        expires_at_ms: status.expiresAtMs,
      },
    ]);
    for (const invalid of [
      ['set', 'Bad\ntext'],
      ['set', 'ok', '--mood', 'x'.repeat(33)],
      ['set', 'ok', '--for', '999ms'],
    ]) {
      const result = await runCli(sandbox, [...args, ...invalid, ...selected, '--json']);
      expect(result.status).not.toBe(0);
      expectError(result, invalid.includes('--for') ? 'USAGE_ERROR' : 'IDENTITY_STATUS_INVALID');
      expect(stored(sandbox.database)).toEqual(after);
    }
    const renewed = (
      await json(sandbox, [...args, 'set', 'Reviewing the renderer', '--for', '90s', ...selected])
    ).status as Status;
    expect(renewed.mood).toBeNull();
    expect(renewed.updatedAtMs).toBeGreaterThanOrEqual(status.updatedAtMs);
    expect(renewed.expiresAtMs - renewed.updatedAtMs).toBe(90_000);
    const human = await runCli(sandbox, [...args, 'show', ...selected]);
    expect(human.status).toBe(0);
    expect(human.stdout).toContain('Self-reported status: Reviewing the renderer');
    expect(human.stdout).toContain('expires:');
    for (const removed of [true, false])
      expect(await json(sandbox, [...args, 'clear', ...selected])).toEqual({
        identityId: identity.id,
        removed,
      });
    expect(await json(sandbox, [...args, 'show', ...selected])).toEqual({
      identityId: identity.id,
      status: null,
    });
    const missing = await runCli(sandbox, [...args, 'show', '--identity', 'Unknown', '--json']);
    expect(missing.status).toBe(3);
    expectError(missing, 'NAME_NOT_FOUND');
    expect(readFileSync(tripwire, 'utf8')).toBe('\n');
  });
});

it('keeps expired status inspectable without renewing it and rejects missing inferred identity', async () => {
  await withSandbox(async (sandbox) => {
    await json(sandbox, ['identity', 'create', 'Alice']);
    await json(sandbox, ['identity', 'status', 'set', 'Finished earlier', '--identity', 'Alice']);
    const database = new Database(sandbox.database);
    try {
      // Fixed historical timestamps exercise the read boundary without a wall-clock sleep.
      database
        .prepare('UPDATE identity_status SET updated_at_ms = 1000, expires_at_ms = 2000')
        .run();
    } finally {
      database.close();
    }
    const before = stored(sandbox.database);
    const shown = await json(sandbox, ['identity', 'status', 'show', '--identity', 'Alice']);
    expect(shown.status).toEqual({
      activity: 'Finished earlier',
      mood: null,
      updatedAtMs: 1000,
      expiresAtMs: 2000,
      stale: true,
    });
    const human = await runCli(sandbox, ['identity', 'status', 'show', '--identity', 'Alice']);
    expect(human.status).toBe(0);
    expect(human.stdout).toContain('Stale status: Finished earlier');
    expect(stored(sandbox.database)).toEqual(before);
    const implicit = await runCli(sandbox, ['identity', 'status', 'set', 'Not mine', '--json']);
    expect(implicit.status).not.toBe(0);
    expectError(implicit, 'IDENTITY_REQUIRED');
    expect(stored(sandbox.database)).toEqual(before);
    for (const help of ['--help', '-h']) {
      const result = await runCli(sandbox, ['identity', 'status', 'set', help]);
      expect(result.status).toBe(0);
      for (const option of ['--identity', '--mood', '--for'])
        expect(result.stdout).toContain(option);
    }
  });
});
