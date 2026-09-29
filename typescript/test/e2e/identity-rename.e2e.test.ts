import Database from 'better-sqlite3';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';

const BADGE_OPTION = '@tmux-team.badge';
const SESSION = '7c41e9d2-77aa-4c3d-9f10-3b2a1c0d9e8f';

interface PublicIdentity {
  id: string;
  name: string;
  canonicalName: string;
  lifetime: string;
}

interface Renamed {
  renamed: boolean;
  previousName: string;
  identity: PublicIdentity;
  pane: { id: string; updated: boolean } | null;
}

function badge(fixture: E2EFixture, pane: string): string {
  return fixture.tmux(['-u', 'show-options', '-p', '-qv', '-t', pane, BADGE_OPTION]).trim();
}

function marker(fixture: E2EFixture, pane: string): { name: string; identityId: string } {
  return JSON.parse(fixture.paneMetadata(pane)).globalIdentity;
}

function rememberSession(fixture: E2EFixture, identityId: string): void {
  const db = new Database(path.join(fixture.globalDir, 'tmux-team.db'));
  try {
    db.prepare(
      `INSERT OR REPLACE INTO identity_session_preferences
         (identity_id, preferred_harness, remembered_harness, runtime_mode, provider_session_id)
       VALUES (?, 'claude', 'claude', 'default', ?)`
    ).run(identityId, SESSION);
  } finally {
    db.close();
  }
}

describe.sequential('identity rename', () => {
  it('keeps the UUID, binding, pending work and remembered session while the pane shows the new name', async () => {
    await withE2EFixture(
      async (fixture) => {
        expectJsonResult(
          await fixture.runJsonCli(['config', 'set', 'ui.paneBadge', 'on', '--global'])
        );
        const agent = await fixture.createMockPane('rename-agent');
        const bound = expectJsonResult(
          await fixture.runJsonCli<{ id: string }>(['add', agent.pane, 'opus-tmt-peer-2', '-s'])
        );
        rememberSession(fixture, bound.id);
        expectJsonResult(await fixture.runJsonCli(['room', 'create', 'crew']));
        expectJsonResult(
          await fixture.runJsonCli(['room', 'join', 'crew', '--identity', 'opus-tmt-peer-2'])
        );
        expectJsonResult(
          await fixture.runJsonCli([
            'identity',
            'meta',
            'set',
            'team',
            'core',
            '--identity',
            'opus-tmt-peer-2',
          ])
        );
        expect(badge(fixture, agent.pane)).toBe('opus-tmt-peer-2 (tmt)');

        // A request the old name addressed, answered only after the rename.
        const pending = expectJsonResult(
          await fixture.runJsonCli<{ requestId: string }>([
            'talk',
            'opus-tmt-peer-2',
            'before the rename',
            '--no-preamble',
            '--detach',
          ])
        );
        await fixture.waitForEvent(
          (event) => event.event === 'request' && event.requestId === pending.requestId
        );

        // Concurrent reads while the name changes never detach the binding.
        const [renamed, ...listings] = await Promise.all([
          fixture.runJsonCli<Renamed>(['rename', 'opus-tmt-peer-2', 'tmt-peer-2']),
          ...Array.from({ length: 3 }, () =>
            fixture.runJsonCli<{ identities: Array<{ id: string; presence: string }> }>(['ls'])
          ),
        ]);
        expect(expectJsonResult(renamed)).toEqual({
          renamed: true,
          previousName: 'opus-tmt-peer-2',
          identity: expect.objectContaining({
            id: bound.id,
            name: 'tmt-peer-2',
            canonicalName: 'tmt-peer-2',
            lifetime: 'saved',
          }),
          pane: { id: agent.pane, updated: true },
        });
        for (const listing of listings) {
          const row = expectJsonResult(listing).identities.find((item) => item.id === bound.id);
          expect(row?.presence).toBe('active');
        }
        expect(badge(fixture, agent.pane)).toBe('tmt-peer-2 (tmt)');
        expect(marker(fixture, agent.pane)).toMatchObject({
          name: 'tmt-peer-2',
          identityId: bound.id,
        });

        // The old name is gone; everything keyed by the UUID answers to the new one.
        const old = await fixture.runJsonCli(['talk', 'opus-tmt-peer-2', 'hello', '--detach']);
        expect(old.code).toBe(3);
        expect(old.json).toMatchObject({ error: { code: 'NAME_NOT_FOUND' } });
        const shown = expectJsonResult(
          await fixture.runJsonCli<{ identity: PublicIdentity; resume?: { session: string } }>(
            ['identity', 'show', 'tmt-peer-2'],
            { withoutTmux: true }
          )
        );
        expect(shown.identity.id).toBe(bound.id);
        expect(shown.resume?.session).toBe(SESSION);
        expect(
          expectJsonResult(
            await fixture.runJsonCli<{ value: string }>([
              'identity',
              'meta',
              'get',
              'team',
              '--identity',
              'tmt-peer-2',
            ])
          ).value
        ).toBe('core');
        const room = expectJsonResult(
          await fixture.runJsonCli<{ room: { memberIds: string[] } }>(['room', 'show', 'crew'])
        );
        expect(room.room.memberIds).toEqual([bound.id]);

        fixture.releaseReplyGate(pending.requestId);
        await fixture.waitForEvent(
          (event) => event.event === 'submitted' && event.requestId === pending.requestId,
          5_000
        );
        expect(
          expectJsonResult(await fixture.runJsonCli(['result', pending.requestId]))
        ).toMatchObject({
          status: 'completed',
          response: 'mock-agent response: before the rename',
        });

        // The gate held only the pending request; open it for new ones.
        fixture.releaseReplyGate();

        const reply = expectJsonResult(
          await fixture.runJsonCli<{ response: string }>([
            'talk',
            'tmt-peer-2',
            'after the rename',
            '--no-preamble',
            '--timeout',
            '10',
          ])
        );
        expect(reply.response).toContain('mock-agent response: after the rename');
      },
      { replyGate: true }
    );
  }, 60_000);

  it('refuses a name another identity holds and renames the display case in place', async () => {
    await withE2EFixture(async (fixture) => {
      const ada = expectJsonResult(
        await fixture.runJsonCli<{ identity: PublicIdentity }>(['identity', 'create', 'Ada'], {
          withoutTmux: true,
        })
      ).identity;
      expectJsonResult(
        await fixture.runJsonCli(['identity', 'create', 'Bob'], { withoutTmux: true })
      );

      const taken = await fixture.runJsonCli(['rename', 'Ada', 'bob'], { withoutTmux: true });
      expect(taken.code).toBe(5);
      expect(taken.json).toEqual({
        error: {
          code: 'NAME_ALREADY_ACTIVE',
          message: "Another identity is already named 'bob'.",
        },
      });

      const human = await fixture.runCli(['identity', 'rename', 'ada', 'ADA'], {
        withoutTmux: true,
      });
      expect(human.code).toBe(0);
      expect(human.stdout).toBe('✓ Renamed Ada to ADA\n');
      expect(human.stderr).toBe('');
      const shown = expectJsonResult(
        await fixture.runJsonCli<{ identity: PublicIdentity }>(['identity', 'show', 'ada'], {
          withoutTmux: true,
        })
      );
      expect(shown.identity).toMatchObject({ id: ada.id, name: 'ADA', canonicalName: 'ada' });
    });
  }, 30_000);
});
