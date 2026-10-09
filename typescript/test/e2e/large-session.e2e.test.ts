import fs from 'node:fs';
import { performance } from 'node:perf_hooks';
import { describe, expect, it } from 'vite-plus/test';
import { durableIdentity, durableState } from './identity-state-oracle.js';
import { withE2EFixture, type CliResult, type E2EFixture, type MockEvent } from './harness.js';
import { readRealTmuxCli, releaseRealTmuxCli, spawnRealTmuxCli } from './real-tmux-caller.js';
import { installTmuxTrace } from './tmux-trace.js';

const UNBOUND_PANE_COUNT = 200;

interface CommandError {
  error: { code: string; message: string };
}

interface WhoamiResult {
  bound: boolean;
  id?: string;
  name?: string;
  pane: string;
  lifetime?: 'temporary' | 'saved';
}

interface CheckResult {
  target: string;
  pane: string;
  identity?: { name: string; canonicalName: string };
  lines: number;
  output: string;
}

interface TalkResult {
  status: string;
  target: string;
  pane: string;
  response: string;
  requestId: string;
}

interface BindingResult {
  bound: true;
  id: string;
  name: string;
  pane: string;
  lifetime: 'temporary' | 'saved';
}

function json<T>(result: CliResult<T>): T {
  expect(result.stderr).toBe('');
  expect(result.stdout.trim()).not.toBe('');
  expect(result.json).toBeDefined();
  return result.json as T;
}

function commandCount(commands: string[], command: string): number {
  return commands.filter((entry) => entry === command).length;
}

function invocationArgs(invocation: string): string {
  const separator = invocation.indexOf('\t');
  return separator < 0 ? invocation : invocation.slice(separator + 1);
}

function processIsRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function createUnboundSleepingPanes(fixture: E2EFixture): Promise<string[]> {
  const panes: string[] = [];
  for (let index = 0; index < UNBOUND_PANE_COUNT; index += 1) {
    const pane = fixture
      .tmux([
        'new-window',
        '-d',
        '-P',
        '-F',
        '#{pane_id}',
        '-t',
        'e2e',
        '-n',
        `idle-${String(index).padStart(3, '0')}`,
        '-c',
        fixture.workspace,
        'sleep 300',
      ])
      .trim();
    if (!pane) throw new Error(`Could not create sleeping pane ${index}.`);
    panes.push(pane);
  }
  return panes;
}

async function waitForDurableReply(
  fixture: E2EFixture,
  result: TalkResult,
  pid: number,
  message: string
): Promise<MockEvent> {
  expect(result.status).toBe('completed');
  expect(result.response).toBe(`mock-agent response: ${message}`);
  return fixture.waitForEvent(
    (event) =>
      event.event === 'submitted' &&
      event.requestId === result.requestId &&
      event.pid === pid &&
      event.body === `mock-agent response: ${message}`
  );
}

describe('scoped identity operations in a large tmux session', { concurrent: false }, () => {
  it(
    'keeps fresh binding, routing, conflicts, and inspection cost scoped to selected panes',
    { timeout: 60_000 },
    async () => {
      const cleanup = { root: '', socketRoot: '', serverPid: 0 };
      await withE2EFixture(async (fixture) => {
        cleanup.root = fixture.root;
        cleanup.socketRoot = fixture.socketRoot;
        cleanup.serverPid = fixture.serverPid;

        const trace = installTmuxTrace(fixture);
        // Initialize server/storage through the public CLI before comparing
        // steady-state costs. No identity is bound and no pane metadata seeded.
        const initialized = await fixture.runJsonCli<WhoamiResult>(['whoami']);
        expect(initialized.code).toBe(0);
        expect(json(initialized)).toEqual({
          bound: false,
          pane: fixture.pane,
          interfaceKind: 'container',
          sessionState: 'unknown',
        });
        trace.clear();
        const smallStartedAt = performance.now();
        const smallWhoami = await fixture.runJsonCli<WhoamiResult>(['whoami']);
        const smallDurationMs = performance.now() - smallStartedAt;
        expect(smallWhoami.code).toBe(0);
        expect(json(smallWhoami)).toEqual({
          bound: false,
          pane: fixture.pane,
          interfaceKind: 'container',
          sessionState: 'unknown',
        });
        const smallCommands = trace.commands();
        const smallInvocations = trace.invocations();
        expect(smallCommands).toContain('list-panes');
        const smallMetadataReads = commandCount(smallCommands, 'show-options');
        expect(smallMetadataReads).toBeGreaterThan(0);

        const idlePanes = await createUnboundSleepingPanes(fixture);
        expect(idlePanes).toHaveLength(UNBOUND_PANE_COUNT);
        const idleMetadata = idlePanes.map((pane) => fixture.paneMetadata(pane));
        expect(idleMetadata.every((metadata) => metadata === '')).toBe(true);

        trace.clear();
        const largeStartedAt = performance.now();
        const largeWhoami = await fixture.runJsonCli<WhoamiResult>(['whoami']);
        const largeDurationMs = performance.now() - largeStartedAt;
        expect(largeWhoami.code).toBe(0);
        expect(json(largeWhoami)).toEqual({
          bound: false,
          pane: fixture.pane,
          interfaceKind: 'container',
          sessionState: 'unknown',
        });
        const largeCommands = trace.commands();
        const largeInvocations = trace.invocations();
        expect(largeCommands).toContain('list-panes');
        // Durations are diagnostic evidence only; subprocess counts are the
        // deterministic regression gate for the former O(panes) fallback.
        console.info('Large-session whoami diagnostics', {
          smallDurationMs: Math.round(smallDurationMs),
          largeDurationMs: Math.round(largeDurationMs),
          smallSubprocesses: smallInvocations.length,
          largeSubprocesses: largeInvocations.length,
        });
        expect(largeInvocations.length).toBe(smallInvocations.length);
        expect(commandCount(largeCommands, 'show-options')).toBe(smallMetadataReads);

        const opaqueMetadata = {
          version: 1,
          unrelated: 'preserve-me',
          nested: { owner: 'fixture', enabled: true },
        };
        fixture.tmux([
          'set-option',
          '-p',
          '-t',
          fixture.pane,
          '@tmux-team.agent',
          JSON.stringify(opaqueMetadata),
        ]);

        // This is intentionally the first binding operation after the large
        // session exists. It exercises the cold explicit-target publication
        // path without creating metadata in every idle pane.
        trace.clear();
        const named = await fixture.runJsonCli<BindingResult>([
          'add',
          fixture.pane,
          'LargeSession',
        ]);
        expect(named.code).toBe(0);
        const namedIdentity = durableIdentity(fixture, 'LargeSession');
        expect(namedIdentity.lifetime).toBe('temporary');
        expect(json(named)).toEqual({
          bound: true,
          id: namedIdentity.id,
          name: 'LargeSession',
          pane: fixture.pane,
          lifetime: 'temporary',
        });
        const addListInvocations = trace
          .invocations()
          .map(invocationArgs)
          .filter((args) => args.includes('list-panes'));
        // Snapshot capture intentionally reads the entire remembered layout;
        // binding/routing evidence still selects only its requested panes.
        const topologyReads = addListInvocations.filter(
          (args) =>
            args.includes('#{window_visible_layout}') &&
            args.includes('@tmux-team.workspace-command')
        );
        expect(topologyReads).toHaveLength(1);
        expect(topologyReads[0]).toContain(' -a ');
        const bindingReads = addListInvocations.filter((args) => !topologyReads.includes(args));
        expect(bindingReads.length).toBeGreaterThan(0);
        expect(bindingReads.every((args) => args.includes(' -f '))).toBe(true);

        const metadataAfterName = fixture.paneMetadata();
        expect(JSON.parse(metadataAfterName)).toMatchObject(opaqueMetadata);
        expect(JSON.parse(metadataAfterName)).toMatchObject({
          globalIdentity: { name: 'LargeSession', canonicalName: 'largesession' },
        });
        const repeated = await fixture.runJsonCli<BindingResult>(['this', 'LargeSession']);
        expect(repeated.code).toBe(0);
        expect(json(repeated)).toEqual(json(named));

        // Caller discovery is exercised from a real descendant of a tmux pane,
        // while preserving the default tmux environment for this positive path.
        const realName = await spawnRealTmuxCli(fixture, ['name', 'DescendantAlias'], {
          name: 'real-name',
        });
        await releaseRealTmuxCli(fixture, realName);
        const realNameResult = readRealTmuxCli<BindingResult>(realName);
        const descendantIdentity = durableIdentity(fixture, 'DescendantAlias');
        expect(descendantIdentity.lifetime).toBe('temporary');
        expect(realNameResult).toEqual({
          code: 0,
          stdout: {
            bound: true,
            id: descendantIdentity.id,
            name: 'DescendantAlias',
            pane: realName.pane,
            lifetime: 'temporary',
          },
          stderr: '',
        });

        const whoami = await fixture.runJsonCli<WhoamiResult>(['whoami']);
        expect(whoami.code).toBe(0);
        expect(json(whoami)).toEqual({
          interfaceKind: 'container',
          sessionState: 'unknown',
          bound: true,
          id: namedIdentity.id,
          name: 'LargeSession',
          pane: fixture.pane,
          lifetime: 'temporary',
        });

        const namedCheck = await fixture.runJsonCli<CheckResult>([
          'check',
          'LargeSession',
          '--lines=20',
        ]);
        expect(namedCheck.code).toBe(0);
        expect(json(namedCheck)).toMatchObject({
          target: 'LargeSession',
          pane: fixture.pane,
          identity: { name: 'LargeSession', canonicalName: 'largesession' },
          lines: 20,
        });

        const directCheck = await fixture.runJsonCli<CheckResult>([
          'check',
          fixture.pane,
          '--lines=20',
        ]);
        expect(directCheck.code).toBe(0);
        expect(json(directCheck)).toMatchObject({
          target: fixture.pane,
          pane: fixture.pane,
          identity: { name: 'LargeSession', canonicalName: 'largesession' },
          lines: 20,
        });

        const namedTalk = await fixture.runJsonCli<TalkResult>([
          'talk',
          'LargeSession',
          'named large-session request',
          '--no-preamble',
          '--timeout',
          '10',
        ]);
        expect(namedTalk.code).toBe(0);
        await waitForDurableReply(
          fixture,
          json(namedTalk),
          fixture.panePid,
          'named large-session request'
        );

        const directTalk = await fixture.runJsonCli<TalkResult>([
          'talk',
          fixture.pane,
          'direct large-session request',
          '--no-preamble',
          '--timeout',
          '10',
        ]);
        expect(directTalk.code).toBe(0);
        await waitForDurableReply(
          fixture,
          json(directTalk),
          fixture.panePid,
          'direct large-session request'
        );

        const stateBeforeConflict = durableState(fixture);
        const metadataBeforeConflict = fixture.paneMetadata();
        const conflict = await fixture.runJsonCli<CommandError>([
          'add',
          fixture.pane,
          'ConflictingName',
        ]);
        expect(conflict.code).toBe(5);
        expect(json(conflict)).toEqual({
          error: { code: 'PANE_ALREADY_BOUND', message: 'Pane is already bound to another name.' },
        });
        expect(fixture.paneMetadata()).toBe(metadataBeforeConflict);
        const stateAfterConflict = durableState(fixture);
        expect(stateAfterConflict.bindings).toEqual(stateBeforeConflict.bindings);
        expect(stateAfterConflict.profiles).toEqual(stateBeforeConflict.profiles);
        expect(idlePanes.map((pane) => fixture.paneMetadata(pane))).toEqual(idleMetadata);
      });

      expect(cleanup.root).not.toBe('');
      expect(cleanup.socketRoot).not.toBe('');
      expect(fs.existsSync(cleanup.root)).toBe(false);
      expect(fs.existsSync(cleanup.socketRoot)).toBe(false);
      expect(processIsRunning(cleanup.serverPid)).toBe(false);
    }
  );
});
