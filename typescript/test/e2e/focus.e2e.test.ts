import fs from 'node:fs';
import { describe, expect, it } from 'vite-plus/test';
import { expectJsonResult } from './cli-assertions.js';
import { withE2EFixture, type E2EFixture } from './harness.js';

interface Focused {
  focused: { pane: string };
  from: { pane: string } | null;
  client: string;
}

interface ClientView {
  client: string;
  pane: string | null;
}

function errorCode(result: { json?: unknown }): string | undefined {
  return (result.json as { error?: { code?: string } } | undefined)?.error?.code;
}

/** Every attached client's session and current pane, by client name. */
function clients(fixture: E2EFixture): Map<string, string> {
  return new Map(
    fixture
      .tmux(['list-clients', '-F', '#{client_name}|#{session_name}/#{pane_id}'])
      .trim()
      .split('\n')
      .filter(Boolean)
      .map((line) => line.split('|') as [string, string])
  );
}

function clientIn(fixture: E2EFixture, session: string): string {
  const name = [...clients(fixture)].find(([, place]) => place.startsWith(`${session}/`))?.[0];
  if (!name) throw new Error(`No client shows session ${session}.`);
  return name;
}

/** A member pane in its own session running `cat`, so any input would echo. */
function memberSession(fixture: E2EFixture, session: string): string {
  return fixture.tmux(['new-session', '-d', '-s', session, '-P', '-F', '#{pane_id}', 'cat']).trim();
}

describe('focus: show a verified pane in the invoking client', { concurrent: false }, () => {
  it('moves only the invoker client across sessions and returns with the captured pane', async () => {
    await withE2EFixture(async (fixture) => {
      const member = memberSession(fixture, 'crew');
      fixture.tmux(['new-session', '-d', '-s', 'other', 'cat']);
      expectJsonResult(await fixture.runJsonCli(['add', member, 'member']));
      await fixture.attachSessionClient('e2e');
      await fixture.attachSessionClient('other');
      const invokerClient = clientIn(fixture, 'e2e');
      const otherClient = clientIn(fixture, 'other');
      const otherBefore = clients(fixture).get(otherClient);
      const memberScreen = fixture.capture(20, member);

      const jumped = expectJsonResult<Focused>(
        await fixture.runJsonCli(['focus', 'member'], { pane: fixture.pane })
      );
      expect(jumped).toEqual({
        focused: { pane: member },
        from: { pane: fixture.pane },
        client: invokerClient,
      });
      expect(clients(fixture).get(invokerClient)).toBe(`crew/${member}`);
      expect(clients(fixture).get(otherClient)).toBe(otherBefore);

      // From where the user now is, the read-only query names the same client
      // and the pane it shows, and moves nothing.
      const before = clients(fixture);
      const view = expectJsonResult<ClientView>(
        await fixture.runJsonCli(['focus', '--client'], { pane: member })
      );
      expect(view).toEqual({ client: invokerClient, pane: member });
      expect(clients(fixture)).toEqual(before);
      // A key-binding job (run-shell, display-popup) has TMUX but no TMUX_PANE:
      // TMUX's session names the client that pressed the key.
      const keyJob = expectJsonResult<ClientView>(
        await fixture.runJsonCli(['focus', '--client'], { pane: member, caller: { pane: null } })
      );
      expect(keyJob).toEqual({ client: invokerClient, pane: member });
      const mixed = await fixture.runJsonCli(['focus', 'member', '--client'], { pane: member });
      expect(errorCode(mixed)).toBe('USAGE_ERROR');
      expect(clients(fixture)).toEqual(before);

      // Back is invoked from where the user now is (the member's session).
      const back = expectJsonResult<Focused>(
        await fixture.runJsonCli(['focus', jumped.from!.pane], { pane: member })
      );
      expect(back).toEqual({
        focused: { pane: fixture.pane },
        from: { pane: member },
        client: invokerClient,
      });
      expect(clients(fixture).get(invokerClient)).toBe(`e2e/${fixture.pane}`);
      expect(clients(fixture).get(otherClient)).toBe(otherBefore);

      // Focus changes only the view: nothing was typed into the member pane.
      expect(fixture.capture(20, member)).toBe(memberScreen);
    });
  });

  it('refuses stale evidence, a missing client or pane, without moving any client', async () => {
    await withE2EFixture(async (fixture) => {
      const member = memberSession(fixture, 'crew');
      expectJsonResult(await fixture.runJsonCli(['add', member, 'member']));

      // No client shows the invoker's session: nothing is guessed.
      const noClient = await fixture.runJsonCli(['focus', 'member'], { pane: fixture.pane });
      expect(errorCode(noClient)).toBe('HOST_UNSUPPORTED');
      const noView = await fixture.runJsonCli(['focus', '--client'], { pane: fixture.pane });
      expect(errorCode(noView)).toBe('HOST_UNSUPPORTED');

      await fixture.attachSessionClient('e2e');
      const invokerClient = clientIn(fixture, 'e2e');
      const before = clients(fixture).get(invokerClient);

      const missing = await fixture.runJsonCli(['focus', '%9999'], {
        pane: fixture.pane,
      });
      expect(errorCode(missing)).toBe('PANE_NOT_FOUND');

      // Without TMUX_PANE, TMUX's session decides; no client shows crew.
      const crewJob = await fixture.runJsonCli(['focus', 'member'], {
        pane: fixture.pane,
        caller: {
          pane: null,
          tmux: `${fixture.socketPath},${fixture.serverPid},${fixture.paneSessionId(member)}`,
        },
      });
      expect(errorCode(crewJob)).toBe('HOST_UNSUPPORTED');

      // A replaced process in the same pane no longer matches the binding.
      fixture.tmux(['respawn-pane', '-k', '-t', member, 'cat']);
      const stale = await fixture.runJsonCli(['focus', 'member'], { pane: fixture.pane });
      expect(stale.code).not.toBe(0);
      expect(['NAME_NOT_FOUND', 'RECONCILIATION_FAILED']).toContain(errorCode(stale));
      expect(clients(fixture).get(invokerClient)).toBe(before);
    });
  });

  it('never focuses an identity bound on another tmux server', async () => {
    await withE2EFixture(async (home) => {
      const member = memberSession(home, 'crew');
      expectJsonResult(await home.runJsonCli(['add', member, 'member']));
      await withE2EFixture(
        async (foreign) => {
          await foreign.attachSessionClient('e2e');
          const client = clientIn(foreign, 'e2e');
          const before = clients(foreign).get(client);
          const refused = await foreign.runJsonCli(['focus', 'member'], { pane: foreign.pane });
          expect(errorCode(refused)).toBe('NAME_NOT_FOUND');
          expect(clients(foreign).get(client)).toBe(before);
        },
        { globalDir: home.globalDir }
      );
      expect(fs.existsSync(home.globalDir)).toBe(true);
    });
  });
});
