import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { withE2EFixture } from './harness.js';
import { requestAttempts, requestResponses } from './request-state-oracle.js';

interface TalkOutput {
  status: string;
  requestId: string;
  response?: string;
  submittedAtMs?: number;
}

describe('frozen Exchange retention through the public CLI', { concurrent: false }, () => {
  it('freezes preparation policy and accepts a gated final despite invalid current config', async () => {
    await withE2EFixture(
      async (fixture) => {
        const globalFile = path.join(fixture.globalDir, 'config.json');
        const localFile = path.join(fixture.workspace, 'tmux-team.json');
        const defaults = await fixture.runJsonCli(['config', 'show'], { withoutTmux: true });
        expect(defaults).toMatchObject({
          code: 0,
          json: {
            resolved: { exchange: { retentionDays: 90 } },
            sources: { exchange: { retentionDays: 'default' } },
          },
        });
        expect(
          (
            await fixture.runJsonCli(['config', 'set', 'exchange.retentionDays', '1', '--global'], {
              withoutTmux: true,
            })
          ).code
        ).toBe(0);
        const beforeRejectedWrite = fs.readFileSync(globalFile, 'utf8');
        const localBefore = fs.existsSync(localFile) ? fs.readFileSync(localFile, 'utf8') : null;
        const localWrite = await fixture.runJsonCli([
          'config',
          'set',
          'exchange.retentionDays',
          '2',
        ]);
        expect(localWrite.code).toBe(1);
        expect(fs.readFileSync(globalFile, 'utf8')).toBe(beforeRejectedWrite);
        expect(fs.existsSync(localFile) ? fs.readFileSync(localFile, 'utf8') : null).toBe(
          localBefore
        );

        const first = await fixture.runJsonCli<TalkOutput>([
          'talk',
          fixture.pane,
          'one day frozen',
          '--no-preamble',
          '--detach',
        ]);
        expect(first.code, first.stderr || first.stdout).toBe(0);
        const firstId = first.json?.requestId ?? '';
        await fixture.waitForEvent(
          (event) => event.event === 'request' && event.requestId === firstId
        );
        expect(
          requestAttempts(fixture).find((row) => row.request_id === firstId)?.retention_days
        ).toBe(1);

        expect(
          (await fixture.runJsonCli(['config', 'set', 'exchange.retentionDays', '90', '--global']))
            .code
        ).toBe(0);
        fixture.releaseReplyGate(firstId);
        await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === firstId
        );
        const firstBody = requestResponses(fixture).find((row) => row.request_id === firstId);
        expect(firstBody).toBeDefined();
        expect(firstBody!.response_expires_at_ms - firstBody!.submitted_at_ms).toBe(86_400_000);
        expect(firstBody!.body).toBe('mock-agent response: one day frozen');
        const firstMetadata = requestAttempts(fixture).find((row) => row.request_id === firstId);
        expect(firstMetadata?.retention_days).toBe(1);

        const second = await fixture.runJsonCli<TalkOutput>([
          'talk',
          fixture.pane,
          'ninety days frozen',
          '--no-preamble',
          '--detach',
        ]);
        expect(second.code, second.stderr || second.stdout).toBe(0);
        const secondId = second.json?.requestId ?? '';
        await fixture.waitForEvent(
          (event) => event.event === 'request' && event.requestId === secondId
        );
        expect(
          requestAttempts(fixture).find((row) => row.request_id === secondId)?.retention_days
        ).toBe(90);

        // Only the disposable fixture config is invalidated after preparation.
        // The real mock peer must still submit through the public reply adapter.
        fs.writeFileSync(globalFile, '{"exchange":{"retentionDays":0}}\n');
        fixture.releaseReplyGate(secondId);
        const submitted = await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === secondId
        );
        const secondBody = requestResponses(fixture).find((row) => row.request_id === secondId);
        expect(secondBody).toBeDefined();
        expect(secondBody!.response_expires_at_ms - secondBody!.submitted_at_ms).toBe(
          90 * 86_400_000
        );
        expect(secondBody!.body).toBe('mock-agent response: ninety days frozen');
        expect(submitted.body).toBe(secondBody!.body);
        const secondMetadata = requestAttempts(fixture).find((row) => row.request_id === secondId);
        expect(secondMetadata?.retention_days).toBe(90);
        const result = await fixture.runJsonCli<TalkOutput>(['result', secondId], {
          withoutTmux: true,
        });
        expect(result).toMatchObject({
          code: 0,
          stderr: '',
          json: {
            status: 'completed',
            requestId: secondId,
            response: secondBody!.body,
            submittedAtMs: secondBody!.submitted_at_ms,
          },
        });
        expect(requestResponses(fixture).find((row) => row.request_id === firstId)).toEqual(
          firstBody
        );
        expect(requestResponses(fixture).find((row) => row.request_id === secondId)).toEqual(
          secondBody
        );
        expect(requestAttempts(fixture).find((row) => row.request_id === firstId)).toEqual(
          firstMetadata
        );
        expect(requestAttempts(fixture).find((row) => row.request_id === secondId)).toEqual(
          secondMetadata
        );
        for (const row of requestAttempts(fixture)) {
          const body = requestResponses(fixture).find(
            (response) => response.request_id === row.request_id
          );
          expect(row.retention_expires_at_ms).toBeGreaterThanOrEqual(body!.response_expires_at_ms);
        }
        expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
      },
      { replyGate: true }
    );
  });
});
