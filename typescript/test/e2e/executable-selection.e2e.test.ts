import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { describe, expect, it } from 'vite-plus/test';
import { resolveCliExecutables } from '../support/cli-executable.mjs';
import { createCliProbe } from '../support/cli-probe.js';
import { E2EFixture, withE2EFixture } from './harness.js';
import { spawnRealTmuxCli, releaseRealTmuxCli, readRealTmuxCli } from './real-tmux-caller.js';

describe('CLI executable selection', () => {
  it.each(['default', 'explicit', 'shared probe', 'separate peer'])(
    'preserves durable talk/result and real descendant behavior with %s selection',
    async (selection) => {
      const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-cli-selection-'));
      try {
        const { cli } = resolveCliExecutables({});
        const primary = createCliProbe(root, cli);
        const peerRoot = path.join(root, 'peer');
        fs.mkdirSync(peerRoot);
        const peer = createCliProbe(peerRoot, cli);
        const executableEnv: NodeJS.ProcessEnv = {};
        if (selection !== 'default') {
          executableEnv.TMT_TEST_CLI = JSON.stringify(
            selection === 'explicit' ? cli : primary.descriptor
          );
        }
        if (selection === 'separate peer')
          executableEnv.TMT_TEST_PEER_CLI = JSON.stringify(peer.descriptor);
        let fixtureRoot = '';
        let socketRoot = '';
        await withE2EFixture(
          async (fixture) => {
            fixtureRoot = fixture.root;
            socketRoot = fixture.socketRoot;
            const descendant = await spawnRealTmuxCli(fixture, ['name', "Reader's identity"], {
              name: 'selected',
              stripTmux: true,
              stripPane: true,
            });
            await releaseRealTmuxCli(fixture, descendant);
            expect(readRealTmuxCli(descendant)).toMatchObject({
              code: 0,
              stderr: '',
              stdout: { bound: true, name: "Reader's identity", pane: descendant.pane },
            });
            expect(JSON.parse(fixture.paneMetadata(descendant.pane))).toMatchObject({
              globalIdentity: { panePid: descendant.panePid },
            });
            const talk = await fixture.runJsonCli<{ requestId: string; response: string }>([
              'talk',
              fixture.pane,
              "request with 'quotes' and spaces",
              '--no-preamble',
              '--timeout',
              '8',
            ]);
            expect(talk.code, talk.stderr || talk.stdout).toBe(0);
            const requestId = talk.json!.requestId;
            const body = "mock-agent response: request with 'quotes' and spaces";
            expect(talk.json?.response).toBe(body);
            await fixture.waitForEvent(
              (event) =>
                event.event === 'submitted' && event.requestId === requestId && event.body === body
            );
            const result = await fixture.runJsonCli(['result', requestId], { withoutTmux: true });
            expect(result).toMatchObject({
              code: 0,
              stderr: '',
              json: { requestId, status: 'completed', response: body },
            });
            if (selection.includes('probe') || selection === 'separate peer') {
              const calls = primary.invocations();
              expect(calls.some((args) => args.includes("Reader's identity"))).toBe(true);
              expect(calls.some((args) => args.includes('talk'))).toBe(true);
              expect(
                calls.some((args) => args.includes('result') && args.includes(requestId))
              ).toBe(true);
              const replies = (selection === 'separate peer' ? peer : primary)
                .invocations()
                .filter((args) => args.includes('reply'));
              expect(replies).toHaveLength(1);
              expect(replies[0]).toContain(requestId);
              if (selection === 'separate peer')
                expect(calls.some((args) => args.includes('reply'))).toBe(false);
            }
          },
          { executableEnv }
        );
        expect(fs.existsSync(fixtureRoot)).toBe(false);
        expect(fs.existsSync(socketRoot)).toBe(false);
      } finally {
        fs.rmSync(root, { recursive: true, force: true });
      }
    },
    20_000
  );

  it('rejects invalid explicit peer selection before allocating a private server or files', () => {
    const roots = () =>
      fs
        .readdirSync(os.tmpdir())
        .filter((name) => name.startsWith('tmux-team-e2e-') || name.startsWith('te2e-'))
        .sort();
    const before = roots();
    expect(() => new E2EFixture({ executableEnv: { TMT_TEST_PEER_CLI: '' } })).toThrow(
      'TMT_TEST_PEER_CLI'
    );
    expect(roots()).toEqual(before);
  });

  it('cleans partial startup if the selected peer disappears after initial validation', async () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'tmt-disappearing-peer-'));
    let fixture: E2EFixture | undefined;
    try {
      const peer = createCliProbe(root, resolveCliExecutables({}).cli);
      fixture = new E2EFixture({
        executableEnv: { TMT_TEST_PEER_CLI: JSON.stringify(peer.descriptor) },
      });
      fs.unlinkSync(peer.descriptor.executable);
      await expect(fixture.start()).rejects.toThrow('failed to start its private tmux server');
      expect(fixture.serverIsRunning()).toBe(false);
      expect(fixture.mockProcessIsRunning()).toBe(false);
      expect(fs.existsSync(fixture.root)).toBe(false);
      expect(fs.existsSync(fixture.socketRoot)).toBe(false);
    } finally {
      await fixture?.stop();
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  it('propagates a selected peer failure without a fallback reply or false completion', async () => {
    let stopped: E2EFixture | undefined;
    await withE2EFixture(
      async (fixture) => {
        stopped = fixture;
        const talk = fixture.runCliProcess<{ requestId: string; status: string }>([
          '--json',
          'talk',
          fixture.pane,
          'peer must fail',
          '--no-preamble',
          '--timeout',
          '1',
        ]);
        const failure = await fixture.waitForEvent(
          (event) => event.event === 'failure' && event.stage === 'reply'
        );
        expect(failure.exitCode).toBe(23);
        const result = await talk.result;
        expect(result).toMatchObject({ code: 4, json: { status: 'timeout' } });
        expect(
          fixture.events().some((event) => event.event === 'submitted' || event.event === 'summary')
        ).toBe(false);
        expect(
          await fixture.runJsonCli(['result', result.json!.requestId], { withoutTmux: true })
        ).toMatchObject({ code: 3, json: { error: { code: 'RESPONSE_NOT_AVAILABLE' } } });
      },
      {
        executableEnv: {
          TMT_TEST_PEER_CLI: JSON.stringify({ executable: '/bin/sh', args: ['-c', 'exit 23'] }),
        },
      }
    );
    expect(stopped?.serverIsRunning()).toBe(false);
    expect(stopped?.mockProcessIsRunning()).toBe(false);
    expect(fs.existsSync(stopped!.root)).toBe(false);
    expect(fs.existsSync(stopped!.socketRoot)).toBe(false);
  });
});
