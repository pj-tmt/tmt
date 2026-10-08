import { existsSync, writeFileSync } from 'node:fs';
import Database from 'better-sqlite3';
import { describe, expect, it } from 'vite-plus/test';
import { parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { installTmuxTripwire } from './tmux-tripwire.js';

async function json(sandbox: Parameters<typeof runCli>[0], args: string[]) {
  const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 5_000 });
  expect(result.status).toBe(0);
  return parseWholeStdout(result);
}

describe('durable local identity inbox', () => {
  it('waits by default for a listener-pulled reply without any host driver', async () => {
    await withSandbox(async (sandbox) => {
      const tmuxLog = installTmuxTripwire(sandbox);
      await json(sandbox, ['identity', 'create', 'Sender']);
      const receiver = await json(sandbox, ['identity', 'create', 'Receiver']);
      const recipient = (receiver.identity as { id: string }).id;
      writeFileSync(
        sandbox.globalConfig,
        JSON.stringify({ defaults: { timeout: 5, pollInterval: 0.01 } })
      );
      const listening = json(sandbox, [
        'x',
        'listen',
        '--identity',
        recipient,
        '--timeout',
        '4s',
        '--debounce',
        '1ms',
      ]);
      // Omit both --inbox and --timeout: ordinary talk must use the configured
      // foreground waiter instead of immediately returning queued/offline.
      const sending = runCli(
        sandbox,
        ['talk', 'Receiver', 'reply through inbox', '--identity', 'Sender', '--json'],
        { deadlineMs: 10_000 }
      );
      const incoming = await listening;
      expect(incoming).toMatchObject({
        reason: 'messages',
        items: [{ delivery: 'queued', kind: 'request' }],
      });
      const requestId = (incoming.items as { requestId: string }[])[0]!.requestId;
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(
          db
            .prepare(
              'SELECT wait_active, status, wake_state FROM request_attempts WHERE request_id = ?'
            )
            .get(requestId)
        ).toEqual({ wait_active: 1, status: 'queued', wake_state: 'not_attempted' });
      } finally {
        db.close();
      }
      const detail = await json(sandbox, [
        'x',
        'show',
        requestId,
        '--incoming',
        '--identity',
        recipient,
      ]);
      const receipt = (detail.exchange as { reply: { receipt: string } }).reply.receipt;
      await json(sandbox, ['reply', requestId, '--receipt', receipt, '--message', 'exact reply\n']);
      const sent = await sending;
      expect(sent.status).toBe(0);
      expect(sent.stderr).toBe('');
      expect(parseWholeStdout(sent)).toMatchObject({
        status: 'completed',
        requestId,
        response: 'exact reply\n',
        recipientIdentityId: recipient,
      });
      expect(await json(sandbox, ['result', requestId])).toMatchObject({
        status: 'completed',
        response: 'exact reply\n',
      });
      await json(sandbox, [
        'x',
        'ack',
        requestId,
        '--incoming',
        '--revision',
        '1',
        '--identity',
        recipient,
      ]);
      // A receiver can re-arm after handling the exact revision without getting
      // the same request again. Neither listen nor inspection acknowledged it.
      expect(
        await json(sandbox, [
          'x',
          'listen',
          '--identity',
          recipient,
          '--timeout',
          '20ms',
          '--debounce',
          '1ms',
        ])
      ).toMatchObject({ reason: 'timeout', items: [] });
      expect(existsSync(`${sandbox.globalDir}/request-observers`)).toBe(false);
      expect(existsSync(tmuxLog)).toBe(false);
    });
  });

  it('times out without a driver, gives runnable inspection commands and accepts a late reply', async () => {
    await withSandbox(async (sandbox) => {
      const tmuxLog = installTmuxTripwire(sandbox);
      await json(sandbox, ['identity', 'create', 'Sender']);
      const receiver = await json(sandbox, ['identity', 'create', 'Receiver']);
      const recipient = (receiver.identity as { id: string }).id;
      const result = await runCli(sandbox, [
        'talk',
        'Receiver',
        'answer after timeout',
        '--identity',
        'Sender',
        '--timeout',
        '20ms',
        '--json',
      ]);
      expect(result.status).toBe(4);
      expect(result.stderr).toBe('');
      const timed = parseWholeStdout(result);
      expect(timed).toMatchObject({
        status: 'timeout',
        target: 'Receiver',
        identity: { name: 'Receiver', canonicalName: 'receiver' },
        error: {
          code: 'TIMEOUT',
          message: expect.stringContaining('has not responded within 0.02s'),
        },
      });
      expect(timed).not.toHaveProperty('pane');
      const requestId = timed.requestId as string;
      const suggestion = (timed.error as { suggestion: string }).suggestion;
      expect(suggestion).toContain(`tmt result ${requestId} --json`);
      expect(suggestion).toContain(`tmt inbox --identity '${recipient}' --json`);
      expect(suggestion).toContain(
        `tmt x show ${requestId} --incoming --identity '${recipient}' --json`
      );
      expect(suggestion).not.toContain('tmt resume');
      expect(await json(sandbox, ['inbox', '--identity', recipient])).toMatchObject({
        items: [{ requestId, delivery: 'queued' }],
      });
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(
          db
            .prepare(
              'SELECT wait_active, status, wake_state FROM request_attempts WHERE request_id = ?'
            )
            .get(requestId)
        ).toEqual({ wait_active: 0, status: 'queued', wake_state: 'not_attempted' });
        expect(db.prepare('SELECT COUNT(*) AS count FROM request_attempts').get()).toEqual({
          count: 1,
        });
      } finally {
        db.close();
      }
      const detail = await json(sandbox, [
        'x',
        'show',
        requestId,
        '--incoming',
        '--identity',
        recipient,
      ]);
      const receipt = (detail.exchange as { reply: { receipt: string } }).reply.receipt;
      await json(sandbox, ['reply', requestId, '--receipt', receipt, '--message', 'late reply']);
      expect(await json(sandbox, ['result', requestId])).toMatchObject({
        status: 'completed',
        requestId,
        response: 'late reply',
      });
      expect(await json(sandbox, ['inbox', '--identity', recipient])).toMatchObject({ items: [] });
      expect(existsSync(`${sandbox.globalDir}/request-observers`)).toBe(false);
      expect(existsSync(tmuxLog)).toBe(false);
    });
  });

  it('keeps detach immediate and rejects missing or retired identities without another request', async () => {
    await withSandbox(async (sandbox) => {
      const tmuxLog = installTmuxTripwire(sandbox);
      await json(sandbox, ['identity', 'create', 'Sender']);
      const receiver = await json(sandbox, ['identity', 'create', 'Receiver']);
      const recipient = (receiver.identity as { id: string }).id;
      const queued = await json(sandbox, [
        'talk',
        'Receiver',
        'detached inbox request',
        '--identity',
        'Sender',
        '--detach',
      ]);
      expect(queued).toMatchObject({
        status: 'queued',
        recipientIdentityId: recipient,
        notification: 'not_attempted',
        waitingFor: 'recipient_inbox_pull',
      });
      expect(queued).not.toHaveProperty('pane');
      await json(sandbox, ['rm', 'Receiver', '--force']);
      for (const target of ['Receiver', 'Missing']) {
        const refused = await runCli(sandbox, [
          'talk',
          target,
          'no request',
          '--identity',
          'Sender',
          '--timeout',
          '20ms',
          '--json',
        ]);
        expect(refused.status).toBe(3);
        expect(parseWholeStdout(refused)).toMatchObject({ error: { code: 'NAME_NOT_FOUND' } });
      }
      const db = new Database(sandbox.database, { readonly: true });
      try {
        expect(db.prepare('SELECT COUNT(*) AS count FROM request_attempts').get()).toEqual({
          count: 1,
        });
        expect(db.prepare('SELECT wait_active FROM request_attempts').get()).toEqual({
          wait_active: 0,
        });
      } finally {
        db.close();
      }
      expect(existsSync(tmuxLog)).toBe(false);
    });
  });

  it('explains intentional queue-only delivery and gives recipient pull and correlated inspection', async () => {
    await withSandbox(async (sandbox) => {
      const tmuxLog = installTmuxTripwire(sandbox);
      await json(sandbox, ['identity', 'create', 'Sender']);
      const receiver = await json(sandbox, ['identity', 'create', 'Receiver']);
      const recipient = (receiver.identity as { id: string }).id;
      const result = await runCli(sandbox, [
        'talk',
        'Receiver',
        'pull this request',
        '--inbox',
        '--identity',
        'Sender',
        '--detach',
      ]);
      expect(result.status).toBe(0);
      expect(result.stderr).toBe('');
      const requestId = result.stdout.match(/Queued request (req_[\w-]+) for Receiver/)?.[1];
      expect(requestId).toBeDefined();
      expect(result.stdout).toContain(
        "no live notification was attempted; waiting for the recipient's inbox pull"
      );
      expect(result.stdout).toContain(`tmt inbox --identity '${recipient}' --json`);
      expect(result.stdout).toContain(
        `tmt x show ${requestId} --incoming --identity '${recipient}' --json`
      );
      expect(result.stdout).toContain(`tmt result ${requestId}`);
      // Follow exactly the advertised recipient pull and correlated inspect paths.
      expect(await json(sandbox, ['inbox', '--identity', recipient])).toMatchObject({
        items: [{ requestId, delivery: 'queued' }],
      });
      expect(
        await json(sandbox, ['x', 'show', requestId!, '--incoming', '--identity', recipient])
      ).toMatchObject({
        exchange: { requestId, prompt: { message: 'pull this request' } },
      });
      expect(existsSync(tmuxLog)).toBe(false);
    });
  });

  it('queues, listens, inspects, replies and preserves participant-scoped attention', async () => {
    await withSandbox(async (sandbox) => {
      const tmuxLog = installTmuxTripwire(sandbox);
      const sender = await json(sandbox, ['identity', 'create', 'Sender']);
      const receiver = await json(sandbox, ['identity', 'create', 'Receiver']);
      const queued = await json(sandbox, [
        'talk',
        'receiver',
        'review the durable request',
        '--inbox',
        '--identity',
        'sender',
        '--detach',
      ]);
      expect(queued).toMatchObject({ status: 'queued', target: 'receiver' });
      expect(queued).toMatchObject({
        notification: 'not_attempted',
        waitingFor: 'recipient_inbox_pull',
      });
      expect(queued).not.toHaveProperty('pane');
      const requestId = queued.requestId as string;

      const incoming = await json(sandbox, [
        'x',
        'listen',
        '--identity',
        'receiver',
        '--timeout',
        '1s',
        '--debounce',
        '1ms',
      ]);
      expect(incoming).toMatchObject({
        reason: 'messages',
        identityId: (receiver.identity as { id: string }).id,
        items: [
          {
            requestId,
            revision: 1,
            kind: 'request',
            direction: 'incoming',
            delivery: 'queued',
            acknowledged: false,
            settled: false,
            inspectCommand: `tmt x show ${requestId} --incoming --identity receiver`,
            ackCommand: `tmt x ack ${requestId} --incoming --revision 1 --identity receiver`,
            sender: {
              identityId: (sender.identity as { id: string }).id,
              name: 'Sender',
              canonicalName: 'sender',
            },
            recipient: {
              identityId: (receiver.identity as { id: string }).id,
              name: 'Receiver',
              canonicalName: 'receiver',
            },
          },
        ],
      });
      expect(JSON.stringify(incoming)).not.toContain('receipt');
      expect(JSON.stringify(incoming)).not.toContain('review the durable request');

      const detail = await json(sandbox, [
        'x',
        'show',
        requestId,
        '--incoming',
        '--identity',
        'receiver',
      ]);
      expect(detail).toMatchObject({
        exchange: {
          requestId,
          prompt: { status: 'retained', message: 'review the durable request' },
          reply: { receipt: expect.stringMatching(/^v2_/) },
        },
      });
      const receipt = (detail.exchange as { reply: { receipt: string } }).reply.receipt;
      await json(sandbox, [
        'reply',
        requestId,
        '--receipt',
        receipt,
        '--message',
        'review complete',
      ]);
      expect(await json(sandbox, ['result', requestId])).toMatchObject({
        status: 'completed',
        requestId,
        response: 'review complete',
      });

      const resultNotice = await json(sandbox, [
        'x',
        'listen',
        '--identity',
        'sender',
        '--timeout',
        '1s',
        '--debounce',
        '1ms',
      ]);
      expect(resultNotice).toMatchObject({
        reason: 'messages',
        identityId: (sender.identity as { id: string }).id,
        items: [
          {
            requestId,
            revision: 2,
            kind: 'response',
            direction: 'incoming',
            inspectCommand: `tmt x show ${requestId} --identity sender`,
            ackCommand: `tmt x ack ${requestId} --revision 2 --identity sender`,
          },
        ],
      });
      expect(JSON.stringify(resultNotice)).not.toContain('review complete');
      expect(JSON.stringify(resultNotice)).not.toContain('receipt');
      await json(sandbox, [
        'x',
        'ack',
        requestId,
        '--incoming',
        '--identity',
        'receiver',
        '--revision',
        '1',
      ]);
      // Recipient acknowledgement cannot consume the originator's response attention.
      expect(await json(sandbox, ['x', 'list', '--identity', 'sender'])).toMatchObject({
        items: [{ requestId, revision: 2, acknowledged: false }],
      });
      const idle = await json(sandbox, [
        'x',
        'listen',
        '--identity',
        'receiver',
        '--timeout',
        '20ms',
        '--debounce',
        '1ms',
      ]);
      expect(idle).toMatchObject({ reason: 'timeout', items: [], nextAfter: null });

      const timed = await runCli(
        sandbox,
        [
          'talk',
          'receiver',
          'keep this queued after foreground timeout',
          '--inbox',
          '--identity',
          'sender',
          '--timeout',
          '20ms',
          '--json',
        ],
        { deadlineMs: 5_000 }
      );
      expect(timed.status).toBe(4);
      const timedDocument = parseWholeStdout(timed);
      expect(timedDocument).toMatchObject({ status: 'timeout', error: { code: 'TIMEOUT' } });
      expect(timedDocument).not.toHaveProperty('pane');
      const timedRequestId = timedDocument.requestId as string;
      expect(
        await json(sandbox, [
          'x',
          'listen',
          '--identity',
          'receiver',
          '--timeout',
          '1s',
          '--debounce',
          '1ms',
        ])
      ).toMatchObject({
        reason: 'messages',
        items: [{ requestId: timedRequestId, kind: 'request', delivery: 'queued' }],
      });
      expect(existsSync(`${sandbox.globalDir}/office/service.json`)).toBe(false);
      expect(existsSync(tmuxLog)).toBe(false);
    });
  });
});
