import Database from 'better-sqlite3';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { expectJsonResult } from './cli-assertions.js';

function observerPid(fixture: E2EFixture, id: string): number {
  const log = fs.readFileSync(
    path.join(fixture.globalDir, 'request-observers', `${id}.log`),
    'utf8'
  );
  const pid = Number(log.match(/^observer_pid=(\d+)$/m)?.[1]);
  expect(Number.isSafeInteger(pid) && pid > 1).toBe(true);
  return pid;
}

async function observerGone(fixture: E2EFixture, pid: number) {
  await fixture.waitFor(
    () => {
      try {
        execFileSync('ps', ['-p', String(pid), '-o', 'pid='], { stdio: 'pipe' });
        return false;
      } catch {
        return true;
      }
    },
    5000,
    'timeout observer exited and was reaped'
  );
}

function rows(fixture: E2EFixture) {
  const database = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
  try {
    return database
      .prepare(
        `SELECT a.request_id, a.route_kind, a.wake_state, a.wait_active,
      a.recipient_attention_revision AS revision, a.recipient_attention_acknowledged_revision AS ack,
      n.reply_state, n.timeout_state, n.observed FROM request_attempts a
      LEFT JOIN request_notifications n USING(request_id) ORDER BY prepared_at_ms`
      )
      .all() as Array<{
      request_id: string;
      route_kind: string;
      wake_state: string;
      wait_active: number;
      revision: number;
      ack: number;
      reply_state: string | null;
      timeout_state: string | null;
      observed: number | null;
    }>;
  } finally {
    database.close();
  }
}

describe.sequential('session-aware durable routing', () => {
  it('losing the detached timeout observer does not resend or suppress a later reply callback', async () => {
    await withE2EFixture(
      async (fixture) => {
        expectJsonResult(await fixture.runJsonCli(['name', 'receiver']));
        const sender = await fixture.createMockPane('sender');
        expectJsonResult(await fixture.runJsonCli(['name', 'sender'], { pane: sender.pane }));
        const sent = expectJsonResult(
          await fixture.runJsonCli(['talk', 'receiver', 'worker crash', '--detach'], {
            pane: sender.pane,
          })
        );
        const id = String(sent.requestId);
        const pid = observerPid(fixture, id);
        const args = execFileSync('ps', ['-p', String(pid), '-o', 'args='], {
          encoding: 'utf8',
        }).trim();
        expect(args.endsWith(`__request-observer ${id}`)).toBe(true);
        const group = execFileSync('ps', ['-p', String(pid), '-o', 'pgid='], {
          encoding: 'utf8',
        }).trim();
        expect(Number(group)).toBe(pid);
        process.kill(-pid, 'SIGKILL');
        await observerGone(fixture, pid);
        fixture.releaseReplyGate(id);
        await fixture.waitForEvent(
          (event) =>
            event.event === 'input' &&
            event.line === `[tmt] reply from receiver to ${id}: tmt result ${id}`
        );
        await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === id
        );
        expect(rows(fixture)[0]).toMatchObject({
          timeout_state: 'not_attempted',
          reply_state: 'sent',
        });
        expect(
          fixture.events().filter((event) => event.event === 'request' && event.requestId === id)
        ).toHaveLength(1);
      },
      { replyGate: true }
    );
  });
  it.each(['rebound', 'offline', 'retired-name-reused'] as const)(
    'routes a reply by original originator UUID after %s, not the captured pane or name',
    async (transition) => {
      await withE2EFixture(
        async (fixture) => {
          expectJsonResult(await fixture.runJsonCli(['name', 'receiver']));
          const sender = await fixture.createMockPane('sender');
          expectJsonResult(
            await fixture.runJsonCli(['name', 'sender', '-s'], { pane: sender.pane })
          );
          const sent = expectJsonResult(
            await fixture.runJsonCli(['talk', 'receiver', 'held reply', '--detach'], {
              pane: sender.pane,
            })
          );
          const id = String(sent.requestId);
          await fixture.waitForEvent(
            (event) => event.event === 'request' && event.requestId === id
          );
          expectJsonResult(await fixture.runJsonCli(['unbind'], { pane: sender.pane }));
          const replacement = await fixture.createMockPane('replacement');
          if (transition === 'retired-name-reused') {
            expectJsonResult(await fixture.runJsonCli(['rm', 'sender', '--force']));
          }
          if (transition !== 'offline') {
            expectJsonResult(
              await fixture.runJsonCli(['name', 'sender', '-s'], { pane: replacement.pane })
            );
          }
          fixture.releaseReplyGate(id);
          await fixture.waitForEvent(
            (event) => event.event === 'submitted' && event.requestId === id
          );
          const inputs = fixture
            .events()
            .filter((event) => event.event === 'input' && event.line?.includes(id));
          expect(inputs.filter((event) => event.pid === sender.pid)).toEqual([]);
          if (transition === 'rebound') {
            const hint = await fixture.waitForEvent(
              (event) => event.event === 'input' && event.line?.includes(id) === true
            );
            expect(hint.pid).toBe(replacement.pid);
            expect(rows(fixture)[0]?.reply_state).toBe('sent');
          } else {
            expect(inputs).toEqual([]);
            expect(rows(fixture)[0]?.reply_state).toBe('unavailable');
          }
          expect(expectJsonResult(await fixture.runJsonCli(['result', id]))).toMatchObject({
            status: 'completed',
            response: 'mock-agent response: held reply',
          });
        },
        { replyGate: true }
      );
    }
  );

  it('a detached timeout is not a final and a late reply still wakes the originator once', async () => {
    await withE2EFixture(
      async (fixture) => {
        expectJsonResult(await fixture.runJsonCli(['name', 'receiver']));
        fs.writeFileSync(
          path.join(fixture.globalDir, 'config.json'),
          JSON.stringify({ defaults: { timeout: 1 } })
        );
        const sender = await fixture.createMockPane('sender');
        expectJsonResult(await fixture.runJsonCli(['name', 'sender'], { pane: sender.pane }));
        const sent = expectJsonResult(
          await fixture.runJsonCli(['talk', 'receiver', 'late', '--detach'], { pane: sender.pane })
        );
        const id = String(sent.requestId);
        await fixture.waitForEvent(
          (event) =>
            event.event === 'input' &&
            event.line?.startsWith(`[tmt] no reply yet from receiver to ${id}`) === true
        );
        expect((await fixture.runJsonCli(['result', id])).code).toBe(3);
        await observerGone(fixture, observerPid(fixture, id));
        fixture.releaseReplyGate(id);
        await fixture.waitForEvent(
          (event) =>
            event.event === 'input' &&
            event.line === `[tmt] reply from receiver to ${id}: tmt result ${id}`
        );
        await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === id
        );
        expect(rows(fixture)[0]).toMatchObject({ timeout_state: 'sent', reply_state: 'sent' });
        expect(expectJsonResult(await fixture.runJsonCli(['result', id]))).toMatchObject({
          response: 'mock-agent response: late',
        });
      },
      { replyGate: true }
    );
  });

  it('does not paste into an ended shell and recovers a fresh plain runtime without replaying old work', async () => {
    await withE2EFixture(async (fixture) => {
      expectJsonResult(await fixture.runJsonCli(['name', 'sender']));
      const recipient = fixture.createShellPane('restart');
      const scenario = path.join(fixture.root, 'ended.json');
      const report = path.join(fixture.root, 'ended-report.json');
      const session = '33333333-3333-4333-8333-333333333333';
      fs.writeFileSync(
        scenario,
        JSON.stringify([
          { args: ['name', 'receiver', '-s', '--json'] },
          {
            args: ['__hook', 'codex'],
            input: { hook_event_name: 'SessionStart', source: 'startup', session_id: session },
          },
          {
            args: ['__hook', 'codex'],
            input: { hook_event_name: 'SessionEnd', reason: 'other', session_id: session },
          },
        ])
      );
      const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
      const launch = (file: string, output: string, listen: boolean) => {
        const args = [
          'env',
          `TMUX_TEAM_HOME=${fixture.globalDir}`,
          '/opt/tmt-tests/hook-runtime/codex',
          fixture.executables.cli.executable,
          file,
          output,
          ...(listen ? ['--listen'] : []),
        ];
        fixture.tmux(['send-keys', '-t', recipient.pane, '-l', args.map(quote).join(' ')]);
        fixture.tmux(['send-keys', '-t', recipient.pane, 'Enter']);
      };
      launch(scenario, report, false);
      await fixture.waitFor(() => fs.existsSync(report), 10000, 'ended fixture');
      const results = JSON.parse(fs.readFileSync(report, 'utf8'));
      expect(results.every((value: { code: number }) => value.code === 0)).toBe(true);
      const offline = expectJsonResult(
        await fixture.runJsonCli(['talk', 'receiver', 'DO_NOT_PASTE_OLD', '--timeout', '1'])
      );
      expect(offline).toMatchObject({ status: 'queued', offline: true });
      expect(fixture.tmux(['capture-pane', '-p', '-t', recipient.pane])).not.toContain(
        'DO_NOT_PASTE_OLD'
      );
      fs.writeFileSync(scenario, '[]');
      const restarted = path.join(fixture.root, 'restarted.json');
      launch(scenario, restarted, true);
      await fixture.waitFor(() => fs.existsSync(restarted), 5000, 'fresh runtime readiness');
      const live = expectJsonResult(
        await fixture.runJsonCli(['talk', 'receiver', 'FRESH_ONLY', '--detach'])
      );
      expect(live).toMatchObject({ status: 'sent', pane: recipient.pane });
      const input = path.join(fixture.root, 'restarted.input.json');
      await fixture.waitFor(() => fs.existsSync(input), 5000, 'fresh runtime consumed paste');
      const lines = JSON.parse(fs.readFileSync(input, 'utf8')) as string[];
      expect(lines).toContain('FRESH_ONLY');
      expect(lines.join('\n')).not.toContain('DO_NOT_PASTE_OLD');
      expect(rows(fixture).map((row) => row.wake_state)).toEqual(['unavailable', 'sent']);
    });
  });
  it('keeps live talk output and delivers once without incoming attention or a second waiter hint', async () => {
    await withE2EFixture(async (fixture) => {
      expectJsonResult(await fixture.runJsonCli(['name', 'receiver']));
      const sender = await fixture.createMockPane('sender');
      expectJsonResult(await fixture.runJsonCli(['name', 'sender'], { pane: sender.pane }));
      const result = expectJsonResult(
        await fixture.runJsonCli(['talk', 'receiver', 'one live request!', '--timeout', '8'], {
          pane: sender.pane,
        })
      );
      expect(result).toMatchObject({
        status: 'completed',
        target: 'receiver',
        pane: fixture.pane,
        identity: { name: 'receiver', canonicalName: 'receiver' },
        response: 'mock-agent response: one live request！',
      });
      const [state] = rows(fixture);
      expect(state).toMatchObject({
        route_kind: 'inbox',
        wake_state: 'sent',
        wait_active: 0,
        reply_state: 'not_attempted',
        observed: 1,
      });
      expect(state!.revision).toBe(state!.ack);
      const incoming = expectJsonResult(
        await fixture.runJsonCli([
          'x',
          'listen',
          '--identity',
          'receiver',
          '--timeout',
          '100ms',
          '--debounce',
          '1ms',
        ])
      );
      expect(incoming.items).toEqual([]);
      expect(expectJsonResult(await fixture.runJsonCli(['whoami', '--context']))).toMatchObject({
        bound: true,
        incoming: { count: 0 },
      });
      expect(fixture.events().filter((event) => event.event === 'request')).toHaveLength(1);
    });
  });

  it('wakes a detached originator once and does not notify again for an identical reply', async () => {
    await withE2EFixture(
      async (fixture) => {
        expectJsonResult(await fixture.runJsonCli(['name', 'receiver']));
        const sender = await fixture.createMockPane('sender');
        expectJsonResult(await fixture.runJsonCli(['name', 'sender'], { pane: sender.pane }));
        const sent = expectJsonResult(
          await fixture.runJsonCli(['talk', 'receiver', 'detached', '--detach'], {
            pane: sender.pane,
          })
        );
        expect(sent).toMatchObject({ status: 'sent', target: 'receiver', pane: fixture.pane });
        const id = String(sent.requestId);
        const request = await fixture.waitForEvent(
          (event) => event.event === 'request' && event.requestId === id
        );
        fixture.releaseReplyGate(id);
        await fixture.waitFor(
          () => rows(fixture)[0]?.reply_state === 'sent',
          5000,
          'originator reply hint'
        );
        expectJsonResult(
          await fixture.runJsonCli([
            'reply',
            id,
            '--receipt',
            request.receipt!,
            '--message',
            'mock-agent response: detached',
          ])
        );
        expect(rows(fixture)[0]).toMatchObject({
          reply_state: 'sent',
          timeout_state: 'not_attempted',
        });
        const hint = `[tmt] reply from receiver to ${id}: tmt result ${id}`;
        await fixture.waitForEvent((event) => event.event === 'input' && event.line === hint);
        expect(
          fixture.events().filter((event) => event.event === 'input' && event.line === hint)
        ).toHaveLength(1);
        expect(fixture.events().filter((event) => event.event === 'request')).toHaveLength(1);
        await observerGone(fixture, observerPid(fixture, id));
      },
      { replyGate: true }
    );
  });

  it('queues an offline recipient, emits a bounded timeout hint and never re-wakes after binding', async () => {
    await withE2EFixture(async (fixture) => {
      expectJsonResult(await fixture.runJsonCli(['name', 'sender']));
      expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'offline']));
      const result = expectJsonResult(
        await fixture.runJsonCli(['talk', 'offline', 'kept offline', '--timeout', '1'])
      );
      expect(result).toMatchObject({ status: 'queued', offline: true, target: 'offline' });
      await fixture.waitFor(() => rows(fixture)[0]?.timeout_state === 'sent', 5000, 'timeout hint');
      await fixture.waitForEvent(
        (event) =>
          event.event === 'input' &&
          event.line ===
            `[tmt] no reply yet from offline to ${result.requestId} after 1s; still pending`
      );
      const recipient = await fixture.createMockPane('offline');
      expectJsonResult(await fixture.runJsonCli(['name', 'offline'], { pane: recipient.pane }));
      const incoming = expectJsonResult(
        await fixture.runJsonCli([
          'x',
          'listen',
          '--identity',
          'offline',
          '--timeout',
          '100ms',
          '--debounce',
          '1ms',
        ])
      );
      expect(incoming.items).toHaveLength(1);
      expect(
        expectJsonResult(
          await fixture.runJsonCli(['whoami', '--context'], { pane: recipient.pane })
        )
      ).toMatchObject({
        bound: true,
        incoming: { count: 1 },
      });
      expect(rows(fixture)[0]).toMatchObject({
        wake_state: 'unavailable',
        reply_state: 'not_attempted',
      });
      expect(fixture.events().filter((event) => event.event === 'request')).toEqual([]);
    });
  });
});
