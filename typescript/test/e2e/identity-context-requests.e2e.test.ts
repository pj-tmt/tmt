import Database from 'better-sqlite3';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';
import { requestAttempts, requestResponses } from './request-state-oracle.js';

function attentionState(fixture: E2EFixture): unknown {
  const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'), { readonly: true });
  try {
    return {
      originators: db
        .prepare('SELECT * FROM request_attention_identities ORDER BY identity_id')
        .all(),
      recipients: db
        .prepare('SELECT * FROM request_recipient_attention_identities ORDER BY identity_id')
        .all(),
    };
  } finally {
    db.close();
  }
}

describe('identity context request summaries', { concurrent: false }, () => {
  it('counts unacknowledged X items without consuming attention or exposing message content', async () => {
    await withE2EFixture(
      async (fixture) => {
        const reader = expectJsonResult<{ id: string }>(
          await fixture.runJsonCli(['name', 'Reader', '-s'])
        );
        expectJsonResult(
          await fixture.runJsonCli(['identity', 'create', 'Peer'], { withoutTmux: true })
        );
        const queue = async (sender: string, recipient: string, body: string) =>
          expectJsonResult<{ requestId: string }>(
            await fixture.runJsonCli(
              ['talk', recipient, body, '--inbox', '--identity', sender, '--detach'],
              { withoutTmux: true }
            )
          ).requestId;
        const pending = await queue('Reader', 'Peer', 'private pending body');
        expectJsonResult(
          await fixture.runJsonCli(['x', 'ackall', '--identity', 'Reader'], { withoutTmux: true })
        );
        const completed = await queue('Reader', 'Peer', 'private completed body');
        const detail = expectJsonResult<{ exchange: { reply: { receipt: string } } }>(
          await fixture.runJsonCli(['x', 'show', completed, '--incoming', '--identity', 'Peer'], {
            withoutTmux: true,
          })
        );
        expectJsonResult(
          await fixture.runJsonCli(
            [
              'reply',
              completed,
              '--receipt',
              detail.exchange.reply.receipt,
              '--message',
              'private final body',
            ],
            { withoutTmux: true }
          )
        );
        const incoming = await queue('Peer', 'Reader', 'private incoming body');
        const before = {
          identities: durableState(fixture),
          attempts: requestAttempts(fixture),
          responses: requestResponses(fixture),
          attention: attentionState(fixture),
        };
        const result = await fixture.runJsonCli(['whoami', '--context']);
        expectJsonResult(result);
        expect(result.json).toMatchObject({
          bound: true,
          id: reader.id,
          originated: { count: 1, inspect: `tmt x --identity '${reader.id}' --json` },
          incoming: { count: 1, inspect: `tmt x --incoming --identity '${reader.id}' --json` },
        });
        expect(result.stdout).not.toContain('private ');
        expect(result.stdout).not.toContain(detail.exchange.reply.receipt);
        for (const id of [pending, completed, incoming]) expect(result.stdout).not.toContain(id);
        expect(Buffer.byteLength(result.stdout)).toBeLessThanOrEqual(4096);
        expect({
          identities: durableState(fixture),
          attempts: requestAttempts(fixture),
          responses: requestResponses(fixture),
          attention: attentionState(fixture),
        }).toEqual(before);
      },
      { mode: 'input-log' }
    );
  });
});
