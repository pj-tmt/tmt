import fs from 'node:fs';
import { describe, expect, it } from 'vitest';
import { durableIdentity, durableState } from './identity-state-oracle.js';
import { withE2EFixture, type CliResult, type E2EFixture } from './harness.js';
import { readRealTmuxCli, releaseRealTmuxCli, spawnRealTmuxCli } from './real-tmux-caller.js';
import { installTmuxTrace } from './tmux-trace.js';

interface BindingResult {
  bound: true;
  id: string;
  name: string;
  pane: string;
  lifetime: 'temporary' | 'saved';
}

interface WhoamiResult {
  bound: boolean;
  id?: string;
  name?: string;
  pane: string;
  lifetime?: 'temporary' | 'saved';
}

interface IdentityListItem {
  id: string;
  name: string;
  canonicalName: string;
  lifetime: 'temporary' | 'saved';
  pane: string;
  target?: string;
  cwd?: string;
  command: string;
}

interface ListResult {
  identities: IdentityListItem[];
}

interface FocusedListResult {
  target: string;
  identity: {
    id: string;
    name: string;
    canonicalName: string;
    lifetime: 'temporary' | 'saved';
  } | null;
  pane: { id: string; target?: string; cwd?: string; command: string };
}

interface TalkResult {
  status: string;
  requestId: string;
  response: string;
}

interface CommandError {
  error: { code: string; message: string };
}

interface PaneTargetRow {
  id: string;
  target: string;
  sessionAttached: number;
}

function json<T>(result: CliResult<T>): T {
  expect(result.code, result.stderr || result.stdout).toBe(0);
  expect(result.stderr).toBe('');
  expect(result.stdout.trim()).not.toBe('');
  expect(result.json).toBeDefined();
  return result.json as T;
}

function listPaneTargets(fixture: E2EFixture): PaneTargetRow[] {
  return fixture
    .tmux([
      'list-panes',
      '-a',
      '-F',
      '#{pane_id}|#{session_name}:#{window_index}.#{pane_index}|#{session_attached}',
    ])
    .trim()
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      const [id = '', target = '', attachedText = '0'] = line.split('|');
      return { id, target, sessionAttached: Number(attachedText) };
    });
}

function expectRepeatedPaneTargets(fixture: E2EFixture, paneId: string): PaneTargetRow[] {
  const repeated = listPaneTargets(fixture).filter((row) => row.id === paneId);
  expect(repeated.length).toBeGreaterThanOrEqual(2);
  expect(new Set(repeated.map((row) => row.target)).size).toBeGreaterThan(1);
  return repeated;
}

function processIsRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function withObservableCleanup(
  callback: (fixture: E2EFixture) => Promise<void>
): Promise<void> {
  const cleanup = { root: '', socketRoot: '', serverPid: 0 };
  await withE2EFixture(async (fixture) => {
    cleanup.root = fixture.root;
    cleanup.socketRoot = fixture.socketRoot;
    cleanup.serverPid = fixture.serverPid;
    await callback(fixture);
  });
  expect(cleanup.root).not.toBe('');
  expect(cleanup.socketRoot).not.toBe('');
  expect(fs.existsSync(cleanup.root)).toBe(false);
  expect(fs.existsSync(cleanup.socketRoot)).toBe(false);
  expect(processIsRunning(cleanup.serverPid)).toBe(false);
}

async function runRealName(
  fixture: E2EFixture,
  name: string,
  options: {
    stripTmux?: boolean;
    stripPane?: boolean;
    linkToSession?: string;
  }
): Promise<{ pane: string; pid: number }> {
  const { linkToSession, ...callerOptions } = options;
  const real = await spawnRealTmuxCli(fixture, ['name', name], {
    name: `real-${name.toLowerCase()}`,
    ...callerOptions,
  });
  if (linkToSession) {
    const windowTarget = fixture.paneTarget(real.pane).replace(/\.\d+$/, '');
    fixture.tmux(['link-window', '-s', windowTarget, '-t', `${linkToSession}:`]);
  }
  expectRepeatedPaneTargets(fixture, real.pane);
  await releaseRealTmuxCli(fixture, real);
  const identity = durableIdentity(fixture, name);
  expect(readRealTmuxCli<BindingResult>(real)).toEqual({
    code: 0,
    stdout: {
      bound: true,
      id: identity.id,
      name,
      pane: real.pane,
      lifetime: 'temporary',
    },
    stderr: '',
  });
  return { pane: real.pane, pid: real.panePid };
}

async function exerciseBoundMockPane(
  fixture: E2EFixture,
  identityName: string,
  pane: { pane: string; pid: number },
  peer: { name: string; pane: string },
  trace: ReturnType<typeof installTmuxTrace>,
  messagePrefix: string
): Promise<void> {
  const opaqueMetadata = {
    version: 1,
    unrelated: 'preserve grouped-session evidence',
    nested: { source: 'e2e', enabled: true },
  };
  fixture.tmux([
    'set-option',
    '-p',
    '-t',
    pane.pane,
    '@tmux-team.agent',
    JSON.stringify(opaqueMetadata),
  ]);
  const peerMetadataBeforeConflict = fixture.paneMetadata(peer.pane);

  const added = await fixture.runJsonCli<BindingResult>(['add', pane.pane, identityName]);
  const identity = durableIdentity(fixture, identityName);
  const peerIdentity = durableIdentity(fixture, peer.name);
  expect(json(added)).toEqual({
    bound: true,
    id: identity.id,
    name: identityName,
    pane: pane.pane,
    lifetime: 'temporary',
  });
  const metadataAfterAdd = fixture.paneMetadata(pane.pane);
  expect(JSON.parse(metadataAfterAdd)).toMatchObject(opaqueMetadata);

  trace.clear();
  const whoami = await fixture.runJsonCli<WhoamiResult>(['whoami'], { pane: pane.pane });
  expect(json(whoami)).toEqual({
    interfaceKind: 'container',
    sessionState: 'unknown',
    bound: true,
    id: identity.id,
    name: identityName,
    pane: pane.pane,
    lifetime: 'temporary',
  });
  const scopedInvocations = trace.invocations();
  const paneQueries = scopedInvocations.filter((invocation) =>
    invocation.startsWith('list-panes\t')
  );
  expect(paneQueries).toHaveLength(1);
  expect(paneQueries[0]).toContain(' -f ');
  expect(
    scopedInvocations.filter(
      (invocation) => invocation.startsWith('show-options\t') && invocation.includes(' -p ')
    )
  ).toEqual([]);

  const listed = json(await fixture.runJsonCli<ListResult>(['list']));
  expect(listed.identities).toHaveLength(2);
  expect(new Set(listed.identities.map((identity) => identity.name)).size).toBe(
    listed.identities.length
  );
  expect(listed.identities).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        id: identity.id,
        name: identityName,
        canonicalName: identityName.toLowerCase(),
        lifetime: 'temporary',
        pane: pane.pane,
      }),
      expect.objectContaining({
        id: peerIdentity.id,
        name: peer.name,
        lifetime: 'temporary',
        pane: peer.pane,
      }),
    ])
  );

  const target = fixture.paneTarget(pane.pane);
  const focused = json(await fixture.runJsonCli<FocusedListResult>(['list', target]));
  const observedTargets = new Set(
    listPaneTargets(fixture)
      .filter((row) => row.id === pane.pane)
      .map((row) => row.target)
  );
  expect(focused).toMatchObject({
    target,
    identity: {
      id: identity.id,
      name: identityName,
      canonicalName: identityName.toLowerCase(),
      lifetime: 'temporary',
    },
    pane: { id: pane.pane },
  });
  expect(focused.pane.target).toBeDefined();
  expect(observedTargets).toContain(focused.pane.target);

  const stateBeforeConflict = durableState(fixture);
  const metadataBeforeConflict = fixture.paneMetadata(pane.pane);
  const conflict = await fixture.runJsonCli<CommandError>(['add', pane.pane, 'ConflictingName']);
  expect(conflict.code).toBe(5);
  expect(conflict.json).toEqual({
    error: { code: 'PANE_ALREADY_BOUND', message: 'Pane is already bound to another name.' },
  });
  expect(fixture.paneMetadata(pane.pane)).toBe(metadataBeforeConflict);
  expect(fixture.paneMetadata(peer.pane)).toBe(peerMetadataBeforeConflict);
  const stateAfterConflict = durableState(fixture);
  expect(stateAfterConflict.bindings).toEqual(stateBeforeConflict.bindings);
  expect(stateAfterConflict.profiles).toEqual(stateBeforeConflict.profiles);
  expect(metadataAfterAdd).toBe(metadataBeforeConflict);

  const message = `${messagePrefix} exact durable reply`;
  const talk = json(
    await fixture.runJsonCli<TalkResult>([
      'talk',
      identityName,
      message,
      '--no-preamble',
      '--timeout',
      '10',
    ])
  );
  expect(talk).toMatchObject({
    status: 'completed',
    requestId: expect.stringMatching(/^req_[0-9a-f-]+$/),
    response: `mock-agent response: ${message}`,
  });
  const submitted = await fixture.waitForEvent(
    (event) =>
      event.event === 'submitted' &&
      event.requestId === talk.requestId &&
      event.pid === pane.pid &&
      event.message === message
  );
  expect(submitted).toMatchObject({
    requestId: talk.requestId,
    pid: pane.pid,
    message,
    body: `mock-agent response: ${message}`,
  });
  const durable = json(
    await fixture.runJsonCli<TalkResult>(['result', talk.requestId], { withoutTmux: true })
  );
  expect(durable).toMatchObject({
    status: 'completed',
    requestId: talk.requestId,
    response: `mock-agent response: ${message}`,
  });
}

describe('grouped and linked tmux pane identity evidence', { concurrent: false }, () => {
  it('deduplicates grouped-session caller evidence for a cold stripped descendant', async () => {
    await withObservableCleanup(async (fixture) => {
      const trace = installTmuxTrace(fixture);
      fixture.tmux(['new-session', '-d', '-t', 'e2e', '-s', 'grouped']);
      await fixture.attachSessionClient('e2e');
      expectRepeatedPaneTargets(fixture, fixture.pane);

      const descendant = await runRealName(fixture, 'GroupedDescendant', {
        stripTmux: true,
        stripPane: true,
      });
      const explicit = await fixture.createMockPane('grouped-explicit');
      expectRepeatedPaneTargets(fixture, explicit.pane);
      await exerciseBoundMockPane(
        fixture,
        'GroupedExplicit',
        explicit,
        { name: 'GroupedDescendant', pane: descendant.pane },
        trace,
        'grouped'
      );
      const attachedTargets = expectRepeatedPaneTargets(fixture, explicit.pane).filter(
        (row) => row.sessionAttached > 0
      );
      expect(attachedTargets).toHaveLength(1);
      const focused = json(
        await fixture.runJsonCli<FocusedListResult>(['list', fixture.paneTarget(explicit.pane)])
      );
      expect(focused.pane.target).toBe(attachedTargets[0]?.target);
      const listed = json(await fixture.runJsonCli<ListResult>(['list']));
      expect(
        listed.identities.find((identity) => identity.name === 'GroupedExplicit')?.target
      ).toBe(attachedTargets[0]?.target);
      expect(descendant.pane).not.toBe(explicit.pane);
    });
  }, 45_000);

  it('keeps linked-window duplicate rows stable with complete caller context', async () => {
    await withObservableCleanup(async (fixture) => {
      const trace = installTmuxTrace(fixture);
      fixture.tmux([
        'new-session',
        '-d',
        '-s',
        'independent',
        '-n',
        'placeholder',
        '-c',
        fixture.workspace,
        'sleep',
        '300',
      ]);
      fixture.tmux(['link-window', '-s', 'e2e:0', '-t', 'independent:']);
      await fixture.attachSessionClient('independent');
      const initialRows = expectRepeatedPaneTargets(fixture, fixture.pane);
      expect(initialRows[0]?.sessionAttached).toBe(0);

      const descendant = await runRealName(fixture, 'LinkedDescendant', {
        linkToSession: 'independent',
      });
      const explicit = { pane: fixture.pane, pid: fixture.panePid };
      await exerciseBoundMockPane(
        fixture,
        'LinkedExplicit',
        explicit,
        { name: 'LinkedDescendant', pane: descendant.pane },
        trace,
        'linked'
      );

      const attachedTargets = expectRepeatedPaneTargets(fixture, explicit.pane).filter(
        (row) => row.sessionAttached > 0
      );
      expect(attachedTargets).toHaveLength(1);
      const focused = json(
        await fixture.runJsonCli<FocusedListResult>(['list', fixture.paneTarget(explicit.pane)])
      );
      expect(focused.pane.target).toBe(attachedTargets[0]?.target);
      const listed = json(await fixture.runJsonCli<ListResult>(['list']));
      expect(listed.identities.find((identity) => identity.name === 'LinkedExplicit')?.target).toBe(
        attachedTargets[0]?.target
      );
      expect(descendant.pane).not.toBe(explicit.pane);
      const duplicateRows = expectRepeatedPaneTargets(fixture, explicit.pane);
      expect(new Set(duplicateRows.map((row) => row.target)).size).toBeGreaterThan(1);
    });
  }, 45_000);
});
