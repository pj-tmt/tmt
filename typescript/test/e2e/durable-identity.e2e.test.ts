import { describe, expect, it } from 'vitest';
import { expectJsonResult, unknownActivity, withoutAddress } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';
import { durableState } from './identity-state-oracle.js';

interface PublicIdentity {
  id: string;
  name: string;
  canonicalName: string;
  lifetime: 'temporary' | 'saved';
}

interface IdentityResult {
  identity: PublicIdentity;
  created?: boolean;
}

interface ListedIdentity extends PublicIdentity {
  presence: 'active' | 'offline' | 'unknown';
  pane: string | null;
  command: string;
}

async function showStoredProfile(fixture: E2EFixture, name: string): Promise<string> {
  const result = await fixture.runJsonCli<{ identity: PublicIdentity; role: { content: string } }>(
    ['role', 'show', '--identity', name],
    { withoutTmux: true }
  );
  return expectJsonResult(result).role.content;
}

describe('durable identity lifecycle', { concurrent: false }, () => {
  it('creates offline, binds, restarts, and rebinds one identity without losing its profile', async () => {
    await withE2EFixture(async (fixture) => {
      // Fullwidth Latin and ASCII spellings are intentionally equivalent after
      // Unicode normalization, while the first spelling remains the display name.
      const name = 'Ａｌｉｃｅ';
      const equivalentName = 'alice';
      const profile = 'Keep this profile across a tmux restart: 世界.';
      const preamble = 'Use the durable profile for this pane.';

      // Creation is deliberately outside tmux: durable identity storage is not
      // contingent on a currently reachable pane.
      const created = expectJsonResult(
        await fixture.runJsonCli<IdentityResult>(['identity', 'create', name], {
          withoutTmux: true,
        })
      );
      expect(created.created).toBe(true);
      expect(created.identity).toMatchObject({
        name,
        canonicalName: 'alice',
        lifetime: 'saved',
      });
      expect(created.identity.id).toMatch(/^[0-9a-f-]{36}$/);

      expect(
        expectJsonResult(
          await fixture.runJsonCli<{ identity: PublicIdentity }>(
            ['identity', 'show', equivalentName],
            { withoutTmux: true }
          )
        )
      ).toEqual({ identity: created.identity });
      expect(
        expectJsonResult(
          await fixture.runJsonCli<{ identities: PublicIdentity[] }>(['identity', 'list'], {
            withoutTmux: true,
          })
        )
      ).toEqual({ identities: [created.identity] });

      expect(
        expectJsonResult(
          await fixture.runJsonCli(['role', 'set', profile, '--identity', name], {
            withoutTmux: true,
          })
        )
      ).toMatchObject({ identity: created.identity, role: { content: profile } });
      expect(
        expectJsonResult(
          await fixture.runJsonCli(['preamble', 'set', name, preamble], { withoutTmux: true })
        )
      ).toEqual({ agent: name, preamble, status: 'set' });

      const firstBinding = expectJsonResult<{
        bound: true;
        id: string;
        name: string;
        pane: string;
        lifetime: 'temporary' | 'saved';
      }>(await fixture.runJsonCli(['name', name]));
      expect(firstBinding).toEqual({
        bound: true,
        id: created.identity.id,
        name,
        pane: fixture.pane,
        lifetime: 'saved',
      });
      const stateBeforeRepeat = durableState(fixture);
      expect(stateBeforeRepeat.identities).toHaveLength(1);
      expect(stateBeforeRepeat.identities[0]?.id).toBe(created.identity.id);
      expect(stateBeforeRepeat.bindings).toHaveLength(1);
      expect(stateBeforeRepeat.bindings[0]?.identity_id).toBe(created.identity.id);
      expect(stateBeforeRepeat.profiles).toHaveLength(1);

      const repeated = expectJsonResult(
        await fixture.runJsonCli<IdentityResult>(['identity', 'create', equivalentName], {
          withoutTmux: true,
        })
      );
      expect(repeated).toEqual({ identity: created.identity, created: false });
      expect(durableState(fixture)).toEqual(stateBeforeRepeat);
      expect(await showStoredProfile(fixture, equivalentName)).toBe(profile);
      expect(durableState(fixture)).toEqual(stateBeforeRepeat);

      const activeBeforeRestart = expectJsonResult<{
        identities: ListedIdentity[];
      }>(withoutAddress(await fixture.runJsonCli(['list'])));
      expect(activeBeforeRestart.identities).toEqual([
        expect.objectContaining({
          id: created.identity.id,
          name,
          canonicalName: 'alice',
          lifetime: 'saved',
          presence: 'active',
          pane: fixture.pane,
        }),
      ]);

      const preambleBeforeRestart = expectJsonResult<{ agent: string; preamble: string }>(
        await fixture.runJsonCli(['preamble', 'show', equivalentName], { withoutTmux: true })
      );
      expect(preambleBeforeRestart).toEqual({ agent: name, preamble });

      expect(expectJsonResult(await fixture.runJsonCli(['unbind']))).toEqual({
        unbound: true,
        id: created.identity.id,
        name,
        pane: fixture.pane,
        lifetime: 'saved',
        retired: false,
      });
      expect(expectJsonResult(withoutAddress(await fixture.runJsonCli(['list'])))).toEqual({
        identities: [
          {
            ...created.identity,
            presence: 'offline',
            pane: null,
            command: '',
            session: { activity: unknownActivity },
          },
        ],
      });

      const restarted = await fixture.restartServer();
      const rebound = expectJsonResult<{
        bound: true;
        id: string;
        name: string;
        pane: string;
        lifetime: 'temporary' | 'saved';
      }>(await fixture.runJsonCli(['name', equivalentName]));
      expect(rebound).toEqual({
        bound: true,
        id: created.identity.id,
        name,
        pane: restarted.pane,
        lifetime: 'saved',
      });
      expect(await showStoredProfile(fixture, name)).toBe(profile);
      expect(
        expectJsonResult(
          await fixture.runJsonCli<{ agent: string; preamble: string }>(
            ['preamble', 'show', name],
            { withoutTmux: true }
          )
        )
      ).toEqual({ agent: name, preamble });
      expect(
        expectJsonResult<{ identities: ListedIdentity[] }>(
          withoutAddress(await fixture.runJsonCli(['list']))
        )
      ).toEqual({
        identities: [
          expect.objectContaining({
            id: created.identity.id,
            name,
            canonicalName: 'alice',
            lifetime: 'saved',
            presence: 'active',
            pane: restarted.pane,
          }),
        ],
      });
      const finalState = durableState(fixture);
      expect(finalState.identities).toEqual(stateBeforeRepeat.identities);
      expect(finalState.profiles).toEqual(stateBeforeRepeat.profiles);
      expect(finalState.bindings).toHaveLength(1);
      expect(finalState.bindings[0]).toMatchObject({
        identity_id: created.identity.id,
        pane_id: restarted.pane,
      });
    });
  }, 45_000);
});
