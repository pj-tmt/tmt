import Database from 'better-sqlite3';
import { randomUUID } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { withE2EFixture } from './harness.js';
import { expectJsonResult } from './cli-assertions.js';

describe('non-Office extension API delivery', { concurrent: false }, () => {
  it.each(['live', 'uncertain'] as const)(
    'keeps %s wake one-shot across extension retries',
    async (mode) => {
      await withE2EFixture(
        async (fixture) => {
          expectJsonResult(await fixture.runJsonCli(['name', 'Receiver']));
          const receiver = expectJsonResult(
            await fixture.runJsonCli<{ identity: { id: string } }>(['identity', 'show', 'Receiver'])
          ).identity.id;
          expectJsonResult(await fixture.runJsonCli(['identity', 'create', 'Sender']));
          const request = {
            version: 1,
            operation: 'dispatch.create',
            identity: 'Sender',
            input: {
              operationId: randomUUID(),
              recipientIds: [receiver],
              message: 'Exact extension work!',
            },
          };
          const input = path.join(fixture.root, 'request.json');
          fs.writeFileSync(input, JSON.stringify(request));
          const extension = path.join(fixture.wrapperDir, 'tmt-teamchat');
          // The fixture owns this path; all argument/data handling is done by TMT.
          fs.writeFileSync(extension, `#!/bin/sh\nexec "$TMT_EXECUTABLE" api < '${input}'\n`, {
            mode: 0o700,
          });
          const first = expectJsonResult(
            await fixture.runCli<{
              operationId: string;
              items: { requestId: string }[];
              wake: { status: string };
            }>(['teamchat'], mode === 'uncertain' ? { transportFault: { stage: 'paste' } } : {})
          );
          expect(first.wake.status).toBe(mode === 'live' ? 'sent' : 'uncertain');
          const id = first.items[0]!.requestId;
          if (mode === 'live') {
            // Agent-produced event, not terminal echo, proves the input was received.
            await fixture.waitForEvent(
              (event) => event.event === 'input' && event.line?.includes(id) === true
            );
          }
          const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
          try {
            const before = db
              .prepare('SELECT * FROM request_attempts WHERE request_id = ?')
              .get(id);
            expect(before).toMatchObject({
              message_text: request.input.message,
              originator_kind: 'explicit',
              wake_state: mode === 'live' ? 'sent' : 'uncertain',
            });
            const replay = expectJsonResult(await fixture.runCli(['teamchat']));
            const { wake: _wake, ...receipt } = first;
            expect(replay).toEqual(receipt);
            expect(
              db.prepare('SELECT * FROM request_attempts WHERE request_id = ?').get(id)
            ).toEqual(before);
            const trace = fs.existsSync(fixture.transportTracePath)
              ? fs.readFileSync(fixture.transportTracePath, 'utf8')
              : '';
            // A second replay with tracing must not execute any tmux delivery stage.
            expectJsonResult(await fixture.runCli(['teamchat'], { transportTrace: true }));
            expect(
              fs.existsSync(fixture.transportTracePath)
                ? fs.readFileSync(fixture.transportTracePath, 'utf8')
                : ''
            ).toBe(trace);
          } finally {
            db.close();
          }
        },
        { mode: 'input-log' }
      );
    }
  );
});
