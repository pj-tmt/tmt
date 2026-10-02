import { unknownActivity, withoutAddress } from './cli-assertions.js';
import { describe, expect, it } from 'vite-plus/test';
import { withE2EFixture } from './harness.js';

interface IdentitySummary {
  name: string;
  canonicalName: string;
}

interface IdentityListItem extends IdentitySummary {
  id: string;
  lifetime: 'temporary' | 'saved';
  presence: 'active' | 'offline' | 'unknown';
  pane: string;
  target?: string;
  cwd?: string;
  command?: string;
}

interface CommandError {
  error: {
    code: string;
    message: string;
    suggestion?: string;
  };
}

interface BoundResult {
  bound: true;
  id: string;
  name: string;
  pane: string;
  lifetime: 'temporary' | 'saved';
}

describe('global identity and runtime routing', { concurrent: false }, () => {
  it('routes names, an ordinary all identity, and direct pane targets across workspaces', async () => {
    await withE2EFixture(async (fixture) => {
      const alphaWorkspace = fixture.createWorkspace('alpha-workspace');
      const allWorkspace = fixture.createWorkspace('all-workspace');
      const callerWorkspace = fixture.createWorkspace('caller-workspace');
      const alpha = await fixture.createMockPane('alpha', alphaWorkspace);
      const all = await fixture.createMockPane('all', allWorkspace);

      const alphaBinding = await fixture.runJsonCli<BoundResult>(['add', alpha.pane, 'Alpha'], {
        cwd: alphaWorkspace,
      });
      expect(alphaBinding.code).toBe(0);
      expect(alphaBinding.json).toEqual({
        bound: true,
        id: expect.any(String),
        name: 'Alpha',
        pane: alpha.pane,
        lifetime: 'temporary',
      });
      const alphaId = alphaBinding.json!.id;
      expect(JSON.parse(fixture.paneMetadata(alpha.pane))).toMatchObject({
        globalIdentity: { name: 'Alpha', canonicalName: 'alpha' },
      });

      const allBinding = await fixture.runJsonCli<BoundResult>(['add', all.pane, 'all'], {
        cwd: allWorkspace,
      });
      expect(allBinding.code).toBe(0);
      expect(allBinding.json).toEqual({
        bound: true,
        id: expect.any(String),
        name: 'all',
        pane: all.pane,
        lifetime: 'temporary',
      });
      const allId = allBinding.json!.id;
      expect(JSON.parse(fixture.paneMetadata(all.pane))).toMatchObject({
        globalIdentity: { name: 'all', canonicalName: 'all' },
      });

      const listed = withoutAddress(
        await fixture.runJsonCli<{ identities: IdentityListItem[] }>(['list'], {
          cwd: callerWorkspace,
        })
      );
      expect(listed.code).toBe(0);
      expect(listed.json).toEqual({
        identities: [
          {
            id: allId,
            name: 'all',
            canonicalName: 'all',
            lifetime: 'temporary',
            presence: 'active',
            pane: all.pane,
            target: fixture.paneTarget(all.pane),
            cwd: allWorkspace,
            command: 'node',
            session: { activity: unknownActivity },
          },
          {
            id: alphaId,
            name: 'Alpha',
            canonicalName: 'alpha',
            lifetime: 'temporary',
            presence: 'active',
            pane: alpha.pane,
            target: fixture.paneTarget(alpha.pane),
            cwd: alphaWorkspace,
            command: 'node',
            session: { activity: unknownActivity },
          },
        ],
      });

      const normalizedTarget = ' ＡＬＰＨＡ ';
      const alphaMessage = 'route to normalized alpha';
      const alphaTalk = await fixture.runJsonCli<{
        target: string;
        pane: string;
        identity: IdentitySummary;
        status: string;
        response: string;
      }>(['talk', normalizedTarget, alphaMessage, '--timeout', '10'], {
        cwd: callerWorkspace,
      });
      expect(alphaTalk.code).toBe(0);
      expect(alphaTalk.json).toMatchObject({
        target: normalizedTarget,
        pane: alpha.pane,
        identity: {
          name: 'Alpha',
          canonicalName: 'alpha',
        },
        status: 'completed',
      });
      expect(alphaTalk.json?.response).toContain(`mock-agent response: ${alphaMessage}`);
      await fixture.waitForEvent(
        (event) =>
          event.event === 'submitted' && event.pid === alpha.pid && event.message === alphaMessage
      );
      expect(
        fixture
          .events()
          .some(
            (event) =>
              event.event === 'request' && event.pid === all.pid && event.message === alphaMessage
          )
      ).toBe(false);

      const allMessage = 'route only to identity named all';
      const allTalk = await fixture.runJsonCli<{
        target: string;
        pane: string;
        identity: IdentitySummary;
        status: string;
      }>(['talk', 'all', allMessage, '--timeout', '10'], { cwd: callerWorkspace });
      expect(allTalk).toMatchObject({
        code: 0,
        json: {
          target: 'all',
          pane: all.pane,
          identity: {
            name: 'all',
            canonicalName: 'all',
          },
          status: 'completed',
        },
      });
      await fixture.waitForEvent(
        (event) =>
          event.event === 'submitted' && event.pid === all.pid && event.message === allMessage
      );
      expect(
        fixture
          .events()
          .some(
            (event) =>
              event.event === 'request' && event.pid === alpha.pid && event.message === allMessage
          )
      ).toBe(false);

      const directTarget = fixture.paneTarget();
      const directMessage = 'route directly to unnamed pane';
      const directTalk = await fixture.runJsonCli<{
        target: string;
        pane: string;
        status: string;
      }>(['talk', directTarget, directMessage, '--timeout', '10'], {
        cwd: callerWorkspace,
      });
      expect(directTalk).toMatchObject({
        code: 0,
        json: { target: directTarget, pane: fixture.pane, status: 'completed' },
      });
      expect(directTalk.json).not.toHaveProperty('identity');
      await fixture.waitForEvent(
        (event) =>
          event.event === 'submitted' &&
          event.pid === fixture.panePid &&
          event.message === directMessage
      );

      const checked = await fixture.runJsonCli<{
        target: string;
        pane: string;
        identity: IdentitySummary;
        lines: number;
        output: string;
      }>(['check', normalizedTarget, '25'], { cwd: callerWorkspace });
      expect(checked).toMatchObject({
        code: 0,
        json: {
          target: normalizedTarget,
          pane: alpha.pane,
          identity: {
            name: 'Alpha',
            canonicalName: 'alpha',
          },
          lines: 25,
        },
      });
      expect(checked.json?.output).toContain(`mock-agent summary: ${alphaMessage}`);

      const focused = withoutAddress(
        await fixture.runJsonCli<{
          target: string;
          identity: IdentitySummary | null;
          pane: {
            id: string;
            target?: string;
            cwd?: string;
            command?: string;
          };
        }>(['list', directTarget], { cwd: callerWorkspace })
      );
      expect(focused).toMatchObject({
        code: 0,
        json: {
          target: directTarget,
          identity: null,
          pane: {
            id: fixture.pane,
            target: directTarget,
            cwd: fixture.workspace,
            command: 'node',
          },
        },
      });
    });
  }, 45_000);

  it('rejects stale, unknown, legacy-order, and team-scoped inputs without side effects', async () => {
    await withE2EFixture(async (fixture) => {
      const alpha = await fixture.createMockPane('alpha');
      const binding = await fixture.runJsonCli(['add', alpha.pane, 'Alpha']);
      expect(binding.code).toBe(0);

      const eventsBefore = fixture.events();
      const metadataBefore = fixture.paneMetadata(alpha.pane);

      const unknown = await fixture.runJsonCli<CommandError>(['talk', 'all', 'must not broadcast']);
      expect(unknown.code).toBe(3);
      expect(unknown.json).toEqual({
        error: { code: 'NAME_NOT_FOUND', message: "Identity 'all' was not found." },
      });

      const stale = await fixture.runJsonCli<CommandError>(['check', '%99999']);
      expect(stale.code).toBe(3);
      expect(stale.json).toEqual({
        error: { code: 'PANE_NOT_FOUND', message: "Pane target '%99999' was not found." },
      });

      const legacyOrder = await fixture.runJsonCli<CommandError>([
        'add',
        'legacy-name',
        alpha.pane,
      ]);
      expect(legacyOrder.code).toBe(1);
      expect(legacyOrder.json).toMatchObject({
        error: { code: 'LEGACY_ADD_ORDER' },
      });
      expect(legacyOrder.json?.error.message).toContain(
        'The v4 add argument order is no longer supported.'
      );
      expect(legacyOrder.json?.error.suggestion).toContain(`tmt add ${alpha.pane} legacy-name`);

      const teamCommand = await fixture.runJsonCli<CommandError>(['team', 'list']);
      expect(teamCommand.code).toBe(1);
      expect(teamCommand.json).toEqual({
        error: {
          code: 'UNSUPPORTED_TEAM',
          message: 'Team workflows are not supported.',
        },
      });

      const teamFlag = await fixture.runJsonCli<CommandError>([
        '--team',
        'legacy',
        'talk',
        'Alpha',
        'must not send',
      ]);
      expect(teamFlag.code).toBe(1);
      expect(teamFlag.json).toEqual({
        error: {
          code: 'UNSUPPORTED_TEAM',
          message: 'Team workflows are not supported.',
        },
      });

      expect(fixture.events()).toEqual(eventsBefore);
      expect(fixture.paneMetadata(alpha.pane)).toBe(metadataBefore);
    });
  });

  it('returns a stable empty identity list', async () => {
    await withE2EFixture(async (fixture) => {
      const listed = withoutAddress(
        await fixture.runJsonCli<{ identities: IdentityListItem[] }>(['list'])
      );
      expect(listed).toMatchObject({ code: 0, json: { identities: [] } });
    });
  });
});
