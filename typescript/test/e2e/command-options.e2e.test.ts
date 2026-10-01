import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { withE2EFixture } from './harness.js';

describe('command option ownership', { concurrent: false }, () => {
  it('rejects an irrelevant delivery flag without binding a pane or opening storage', async () => {
    await withE2EFixture(async (fixture) => {
      const metadata = fixture.paneMetadata(fixture.pane);
      const result = await fixture.runJsonCli(['name', 'NeverBound', '--timeout', '2'], {
        withoutTmux: true,
      });
      expect(result).toMatchObject({ code: 1, json: { error: { code: 'USAGE_ERROR' } } });
      expect(result.stderr).toBe('');
      expect(fs.existsSync(path.join(fixture.globalDir, 'tmux-team.db'))).toBe(false);
      expect(fs.existsSync(fixture.forbiddenTmuxLogPath)).toBe(false);
      expect(fixture.paneMetadata(fixture.pane)).toBe(metadata);
    });
  });

  it('preserves root options and literal payloads through send, reply, and read', async () => {
    await withE2EFixture(async (fixture) => {
      expect((await fixture.runJsonCli(['this', 'GrammarPeer'])).code).toBe(0);
      expect(
        (await fixture.runJsonCli(['preamble', 'set', 'GrammarPeer', 'MUST NOT BE INJECTED'])).code
      ).toBe(0);
      const metadata = fixture.paneMetadata(fixture.pane);
      const message = '--json --timeout=0 literal payload';
      const sent = await fixture.runJsonCli<{ requestId: string; response: string }>([
        '--timeout=10',
        '--delay=0',
        '--no-preamble',
        'send',
        'GrammarPeer',
        '--',
        message,
      ]);
      expect(sent.code, sent.stderr || sent.stdout).toBe(0);
      expect(sent.json?.response).toBe(`mock-agent response: ${message}`);
      const requestId = sent.json?.requestId;
      expect(requestId).toMatch(/^req_[0-9a-f-]+$/);
      const request = await fixture.waitForEvent(
        (event) => event.event === 'request' && event.requestId === requestId
      );
      expect(request.message).toBe(message);
      await fixture.waitForEvent(
        (event) => event.event === 'summary' && event.requestId === requestId
      );
      const captured = await fixture.runJsonCli<{ lines: number; output: string }>([
        '--lines=0',
        'read',
        'GrammarPeer',
      ]);
      expect(captured.code).toBe(0);
      expect(captured.json?.lines).toBe(0);
      expect(captured.json?.output).toContain(`mock-agent summary: ${message}`);
      expect(fixture.paneMetadata(fixture.pane)).toBe(metadata);
    });
  });
});
