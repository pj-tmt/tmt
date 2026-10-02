import fs from 'node:fs';
import path from 'node:path';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vitest';
import { withE2EFixture, type CliResult } from './harness.js';
import { requestAttempts } from './request-state-oracle.js';

interface ErrorResult {
  error: { code: string; message: string; stage?: string };
}

function expectError(result: CliResult, exitCode: number, code: string): ErrorResult {
  expect(result.code).toBe(exitCode);
  expect(result.stderr).toBe('');
  // Parse the entire stream: an extra document or progress line must fail.
  const output = JSON.parse(result.stdout) as ErrorResult;
  expect(output.error).toMatchObject({ code, message: expect.any(String) });
  expect(output.error.message.length).toBeGreaterThan(0);
  return output;
}

describe('single JSON command error boundary', { concurrent: false }, () => {
  it('preserves missing target and binding conflict exits without extra output', async () => {
    await withE2EFixture(async (fixture) => {
      expectError(await fixture.runJsonCli(['check', 'missing']), 3, 'NAME_NOT_FOUND');
      expectError(await fixture.runJsonCli(['whoami'], { outsideTmux: true }), 3, 'PANE_NOT_FOUND');
      expect((await fixture.runJsonCli(['name', 'Receiver'])).code).toBe(0);
      expectError(await fixture.runJsonCli(['name', 'Other']), 5, 'PANE_ALREADY_BOUND');
      const current = await fixture.runJsonCli<{ name: string }>(['whoami']);
      expect(current.json?.name).toBe('Receiver');
      expect(fixture.events().filter((event) => event.event === 'request')).toEqual([]);
    });
  });

  it('reports busy storage once and recovers without changing the role', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['name', 'Receiver'])).code).toBe(0);
      expect(
        (await fixture.runJsonCli(['role', 'set', 'Keep this role', '--identity', 'Receiver'])).code
      ).toBe(0);
      const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'));
      try {
        database.exec('BEGIN IMMEDIATE');
        const request = fixture.runCliProcess([
          '--json',
          'role',
          'set',
          'Must not replace',
          '--identity',
          'Receiver',
        ]);
        let completed = false;
        void request.result.then(() => {
          completed = true;
        });
        // Existing repository initialization retries 20 times after the first
        // attempt; each SQLite open can wait 5 s. Exercise actual exhaustion,
        // not lock release that would turn this failure scenario into success.
        // Retry-policy redesign is separate from the CLI output boundary.
        await fixture.waitFor(() => completed, 115_000, 'storage retry exhaustion');
        const failed = await request.result;
        expectError(failed, 1, 'ROLE_ERROR');
        expect(database.prepare('SELECT content FROM role_profiles').get()).toEqual({
          content: 'Keep this role',
        });
      } finally {
        if (database.inTransaction) database.exec('ROLLBACK');
        database.close();
      }
      const recovered = await fixture.runJsonCli<{ role: { content: string } }>([
        'role',
        'show',
        '--identity',
        'Receiver',
      ]);
      expect(recovered.code).toBe(0);
      expect(recovered.json?.role.content).toBe('Keep this role');
    });
  }, 125_000);

  it('does not complete from pane closure and accepts a late public reply', async () => {
    await withE2EFixture(
      async (fixture) => {
        expect((await fixture.runJsonCli(['name', 'Receiver'])).code).toBe(0);
        const request = fixture.runCliProcess<{
          requestId: string;
          status: string;
          response?: string;
        }>(['--json', 'talk', 'Receiver', 'pane closure request', '--timeout', '20']);
        const event = await fixture.waitForEvent(
          (item) => item.event === 'request' && item.message === 'pane closure request',
          5_000
        );
        await fixture.waitForEvent(
          (item) => item.event === 'silent' && item.requestId === event.requestId,
          5_000
        );
        await fixture.waitFor(() => requestAttempts(fixture)[0]?.wake_state === 'sent');
        fixture.tmux(['kill-pane', '-t', fixture.pane]);
        expect(requestAttempts(fixture)[0]).toMatchObject({
          request_id: event.requestId,
          status: 'queued',
          route_kind: 'inbox',
          wake_state: 'sent',
          wait_active: 1,
        });
        const bodyPath = path.join(fixture.root, 'late-response.txt');
        fs.writeFileSync(bodyPath, 'late response after pane closure');
        const reply = await fixture.runJsonCli(
          ['reply', event.requestId ?? '', '--receipt', event.receipt ?? '', '--file', bodyPath],
          { outsideTmux: true }
        );
        expect(reply.code).toBe(0);
        const completed = await request.result;
        expect(completed.code).toBe(0);
        expect(completed.json).toMatchObject({
          status: 'completed',
          requestId: event.requestId,
          response: 'late response after pane closure',
        });
        expect(requestAttempts(fixture)).toHaveLength(1);
        expect(requestAttempts(fixture)[0]).toMatchObject({
          request_id: event.requestId,
          status: 'queued',
          route_kind: 'inbox',
          wake_state: 'sent',
          wait_active: 0,
        });
      },
      { mode: 'silent' }
    );
  });

  it('wakes a long poll on SIGINT and drains exactly one JSON error', async () => {
    await withE2EFixture(
      async (fixture) => {
        fs.writeFileSync(
          path.join(fixture.globalDir, 'config.json'),
          JSON.stringify({ defaults: { pollInterval: 60 } })
        );
        expect((await fixture.runJsonCli(['name', 'Receiver'])).code).toBe(0);
        const request = fixture.runCliProcess([
          '--json',
          'talk',
          'Receiver',
          'interrupt long poll',
          '--timeout',
          '120',
        ]);
        const event = await fixture.waitForEvent(
          (item) => item.event === 'silent' && item.message === 'interrupt long poll',
          5_000
        );
        await fixture.waitFor(() => requestAttempts(fixture)[0]?.wake_state === 'sent');
        request.kill('SIGINT');
        const result = await request.result;
        expectError(result, 1, 'INTERRUPTED');
        expect(requestAttempts(fixture)).toHaveLength(1);
        expect(requestAttempts(fixture)[0]).toMatchObject({
          request_id: event.requestId,
          status: 'queued',
          route_kind: 'inbox',
          wake_state: 'sent',
          wait_active: 0,
        });
      },
      { mode: 'silent' }
    );
  }, 12_000);
});
