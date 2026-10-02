import { existsSync } from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import { parseWholeStdout, runCli, withSandbox } from '../support/cli-process.js';
import { installTmuxTripwire } from './tmux-tripwire.js';

async function json(sandbox: Parameters<typeof runCli>[0], args: string[]) {
  const result = await runCli(sandbox, [...args, '--json'], { deadlineMs: 5_000 });
  expect(result.status).toBe(0);
  return parseWholeStdout(result);
}

describe('durable local identity inbox', () => {
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
