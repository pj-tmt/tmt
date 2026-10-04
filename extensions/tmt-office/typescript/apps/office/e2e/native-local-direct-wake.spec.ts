import { randomUUID } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { expect, test } from '@playwright/test';
import { withSandbox } from '../../../../../../typescript/test/support/cli-process.js';
import { withE2EFixture } from '../../../../../../typescript/test/e2e/harness.js';
import { installNativeOffice, unusedLoopbackPort } from './native-office-fixture.js';
import type { DispatchReceipt } from '../src/local/dispatch-contract.js';

test('a direct Office request wakes only the verified recipient once while inbox acceptance remains durable', async ({
  request,
}) => {
  await withSandbox(async (sandbox) => {
    const prefix = await installNativeOffice(sandbox);
    await withE2EFixture(
      async (fixture) => {
        const cli = async <T>(args: string[], outsideTmux = false): Promise<T> => {
          const result = await fixture.runJsonCli<T>(args, { outsideTmux });
          expect(result.code, result.stdout + result.stderr).toBe(0);
          return result.json!;
        };
        const pane = await fixture.createMockPane('recipient');
        const alice = await cli<{ id: string }>(['add', pane.pane, 'Alice', '-s']);
        const offline = await cli<{ identity: { id: string } }>(['identity', 'create', 'Offline']);
        const room = (await cli<{ room: { id: string } }>(['room', 'create', 'Design'])).room;
        await cli(['room', 'join', room.id, '--identity', 'Alice']);
        const started = await cli<{ url: string }>(
          ['office', '--prefix', prefix, 'start', '--port', String(await unusedLoopbackPort())],
          true
        );
        let address = new URL(started.url);
        let token = new URLSearchParams(address.hash.slice(1)).get('token');
        expect(token).toBeTruthy();
        const post = async (body: object) => {
          const response = await request.post(`${address.origin}/api/v1/local/dispatch`, {
            headers: { Authorization: `Bearer ${token}`, Origin: address.origin },
            data: body,
          });
          expect(response.status(), await response.text()).toBe(200);
          return (await response.json()) as DispatchReceipt;
        };
        try {
          const body = {
            operationId: randomUUID(),
            recipientIds: [alice.id],
            room: { kind: 'direct', roomId: room.id },
            // Exceed the 48-character preview so the full body stays in the inbox.
            message: 'private body must remain in the inbox beyond this short preview',
          };
          const accepted = await post(body);
          const requestId = accepted.items[0]!.requestId;
          expect(accepted.items).toEqual([
            { recipientId: alice.id, requestId, acceptance: 'queued' },
          ]);
          expect(accepted.wake).toEqual({
            status: 'sent',
            paneAttempted: true,
            agentProcessed: null,
          });
          const input = await fixture.waitForEvent(
            (event) => event.event === 'input' && event.line?.includes(requestId) === true
          );
          expect(input.line).toBe(
            `▚ ◆ anonymous · ${body.message.slice(0, 48)}… · tmt x show ${requestId} --incoming --identity ${alice.id} --json`
          );
          await fixture.waitForEvent(
            (event) => event.event === 'input' && event.pid === input.pid && event.line === ''
          );
          const wakeEvents = fixture.events().filter((event) => event.event === 'input');
          expect(
            fixture
              .events()
              .filter((event) => event.event === 'input' && event.line?.includes(requestId))
          ).toHaveLength(1);
          expect(fixture.events().some((event) => event.line?.includes(body.message))).toBe(false);
          const replay = await post(body);
          expect(replay.items).toEqual(accepted.items);
          expect(replay.wake).toBeUndefined();
          expect(
            fixture
              .events()
              .filter((event) => event.event === 'input' && event.line?.includes(requestId))
          ).toHaveLength(1);
          const incoming = await cli<{ exchange: { roomId: string; prompt: { message: string } } }>(
            ['x', 'show', requestId, '--incoming', '--identity', alice.id],
            true
          );
          expect(incoming.exchange.roomId).toBe(room.id);
          expect(incoming.exchange.prompt.message).toBe(body.message);

          const absent = await post({
            operationId: randomUUID(),
            recipientIds: [offline.identity.id],
            message: 'offline request',
          });
          expect(absent.items[0]!.acceptance).toBe('queued');
          expect(absent.wake).toEqual({
            status: 'unavailable',
            paneAttempted: false,
            agentProcessed: null,
          });
          expect(fixture.events().filter((event) => event.event === 'input')).toEqual(wakeEvents);

          // A live pane whose owned command ended is a shell, not a recipient.
          const shell = fixture.createShellPane('ended-recipient');
          const done = path.join(fixture.root, 'ended-run.json');
          const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;
          const command = [
            'env',
            `TMUX_TEAM_HOME=${fixture.globalDir}`,
            fixture.executables.cli.executable,
            'run',
            '-s',
            'Ended',
            '/bin/true',
          ]
            .map(quote)
            .join(' ');
          fixture.tmux([
            'send-keys',
            '-t',
            shell.pane,
            '-l',
            `${command}; printf done > ${quote(done)}`,
          ]);
          fixture.tmux(['send-keys', '-t', shell.pane, 'Enter']);
          await fixture.waitFor(() => fs.existsSync(done), 5000, 'owned command returned to shell');
          const ended = await cli<{ identity: { id: string } }>(['identity', 'show', 'Ended']);
          const readSql = (sql: string) =>
            JSON.parse(
              execFileSync(
                'sqlite3',
                ['-readonly', '-json', path.join(fixture.globalDir, 'tmux-team.db'), sql],
                { encoding: 'utf8' }
              )
            );
          expect(
            readSql(`SELECT runtime_state FROM bindings WHERE identity_id = '${ended.identity.id}'`)
          ).toEqual([{ runtime_state: 'ended' }]);
          const endedBody = {
            operationId: randomUUID(),
            recipientIds: [ended.identity.id],
            message: 'Keep this request durable without typing into the ended shell.',
          };
          const queued = await post(endedBody);
          expect(queued.items[0]!.acceptance).toBe('queued');
          expect(queued.wake).toEqual({
            status: 'unavailable',
            paneAttempted: false,
            agentProcessed: null,
          });
          expect(fixture.tmux(['capture-pane', '-p', '-t', shell.pane])).not.toContain(
            queued.items[0]!.requestId
          );
          expect(
            readSql(
              `SELECT wake_state FROM request_attempts WHERE request_id = '${queued.items[0]!.requestId}'`
            )
          ).toEqual([{ wake_state: 'unavailable' }]);
          expect((await post(endedBody)).wake).toBeUndefined();
          expect(fixture.tmux(['capture-pane', '-p', '-t', shell.pane])).not.toContain(
            queued.items[0]!.requestId
          );
          const kept = await cli<{ exchange: { prompt: { message: string } } }>(
            [
              'x',
              'show',
              queued.items[0]!.requestId,
              '--incoming',
              '--identity',
              ended.identity.id,
            ],
            true
          );
          expect(kept.exchange.prompt.message).toBe(endedBody.message);

          await cli(['office', '--prefix', prefix, 'stop'], true);
          const faulted = await fixture.runJsonCli<{ url: string }>(
            ['office', '--prefix', prefix, 'start', '--port', String(await unusedLoopbackPort())],
            { outsideTmux: true, transportFault: { stage: 'paste' } }
          );
          expect(faulted.code, faulted.stdout + faulted.stderr).toBe(0);
          address = new URL(faulted.json!.url);
          token = new URLSearchParams(address.hash.slice(1)).get('token');
          const uncertainBody = {
            operationId: randomUUID(),
            recipientIds: [alice.id],
            message: 'Paste may already have reached the pane.',
          };
          const uncertain = await post(uncertainBody);
          expect(uncertain.items[0]!.acceptance).toBe('queued');
          expect(uncertain.wake).toEqual({
            status: 'uncertain',
            paneAttempted: true,
            agentProcessed: null,
          });
          const afterFault = fixture.transportTrace();
          expect(
            afterFault.filter((line) => line.startsWith('paste-buffer.fault-after'))
          ).toHaveLength(1);
          expect((await post(uncertainBody)).wake).toBeUndefined();
          expect(fixture.transportTrace()).toEqual(afterFault);
        } finally {
          const stopped = await fixture.runJsonCli(['office', '--prefix', prefix, 'stop'], {
            outsideTmux: true,
          });
          expect(stopped.code, stopped.stdout + stopped.stderr).toBe(0);
        }
      },
      { globalDir: sandbox.globalDir, mode: 'input-log' }
    );
  });
});
