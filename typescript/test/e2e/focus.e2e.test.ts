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

  for (const direction of ['left', 'right', 'up', 'down'] as const) {
    it(`swaps ${direction} without moving the caller or killing any pane`, async () => {
      await withE2EFixture(async (fixture) => {
        const horizontal = direction === 'left' || direction === 'right';
        const before = direction === 'left' || direction === 'up';
        const neighbour = fixture
          .tmux([
            'split-window',
            horizontal ? '-h' : '-v',
            ...(before ? ['-b'] : []),
            '-t',
            fixture.pane,
            '-P',
            '-F',
            '#{pane_id}',
            'cat',
          ])
          .trim();
        const target = memberSession(fixture, 'swap-target');
        expectJsonResult(await fixture.runJsonCli(['add', target, 'swap-worker']));
        fixture.tmux(['select-pane', '-t', fixture.pane]);
        await fixture.attachSessionClient('e2e');
        const views = clients(fixture);
        const frame = (pane: string): string =>
          fixture
            .tmux([
              'display-message',
              '-p',
              '-t',
              pane,
              '#{window_id}|#{pane_left}|#{pane_top}|#{pane_width}|#{pane_height}',
            ])
            .trim();
        const all = (): string[] =>
          fixture
            .tmux(['list-panes', '-a', '-F', '#{pane_id}|#{pane_pid}'])
            .trim()
            .split('\n')
            .sort();
        const panes = all();
        const callerFrame = frame(fixture.pane);
        const neighbourFrame = frame(neighbour);
        const targetFrame = frame(target);
        const swapped = expectJsonResult(
          await fixture.runJsonCli(['focus', 'swap-worker', '--into', direction], {
            pane: fixture.pane,
          })
        );
        expect(swapped).toEqual({ swapped: { pane: target }, with: { pane: neighbour } });
        expect(frame(target)).toBe(neighbourFrame);
        expect(frame(neighbour)).toBe(targetFrame);
        expect(frame(fixture.pane)).toBe(callerFrame);
        expect(clients(fixture)).toEqual(views);
        expect(all()).toEqual(panes);
        const already = expectJsonResult(
          await fixture.runJsonCli(['focus', 'swap-worker', '--into', direction], {
            pane: fixture.pane,
          })
        );
        expect(already).toEqual({ swapped: null, with: { pane: target }, already: true });
        expect(frame(target)).toBe(neighbourFrame);
        expect(all()).toEqual(panes);
        expect(clients(fixture)).toEqual(views);
      });
    });
  }

  it('refuses missing neighbours, zoom, no caller, itself and gone targets without effects', async () => {
    await withE2EFixture(async (fixture) => {
      const target = memberSession(fixture, 'swap-refusals');
      await fixture.attachSessionClient('e2e');
      const views = clients(fixture);
      const panes = fixture.tmux(['list-panes', '-a', '-F', '#{pane_id}|#{window_id}|#{pane_pid}']);
      for (const args of [
        ['focus', target, '--into', 'left'],
        ['focus', '%99999', '--into', 'right'],
      ]) {
        expect(errorCode(await fixture.runJsonCli(args, { pane: fixture.pane }))).toBe(
          'PANE_NOT_FOUND'
        );
      }
      expect(
        errorCode(
          await fixture.runJsonCli(['focus', fixture.pane, '--into', 'right'], {
            pane: fixture.pane,
          })
        )
      ).toBe('HOST_UNSUPPORTED');
      expect(
        errorCode(
          await fixture.runJsonCli(['focus', target, '--into', 'right'], {
            pane: fixture.pane,
            caller: { pane: null },
          })
        )
      ).toBe('HOST_UNSUPPORTED');
      expect(clients(fixture)).toEqual(views);
      expect(fixture.tmux(['list-panes', '-a', '-F', '#{pane_id}|#{window_id}|#{pane_pid}'])).toBe(
        panes
      );
      fixture.tmux(['split-window', '-h', '-t', fixture.pane, 'cat']);
      fixture.tmux(['select-pane', '-t', fixture.pane]);
      fixture.tmux(['resize-pane', '-Z', '-t', fixture.pane]);
      expect(
        errorCode(
          await fixture.runJsonCli(['focus', target, '--into', 'right'], { pane: fixture.pane })
        )
      ).toBe('PANE_NOT_FOUND');
      expect(
        fixture.tmux(['display-message', '-p', '-t', fixture.pane, '#{window_zoomed_flag}']).trim()
      ).toBe('1');
      expect(clients(fixture)).toEqual(views);
      fixture.tmux(['kill-pane', '-t', target]);
      expect(
        errorCode(
          await fixture.runJsonCli(['focus', target, '--into', 'right'], { pane: fixture.pane })
        )
      ).toBe('PANE_NOT_FOUND');
      expect(clients(fixture)).toEqual(views);
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
      const staleSwap = await fixture.runJsonCli(['focus', 'member', '--into', 'left'], {
        pane: fixture.pane,
      });
      expect(staleSwap.code).not.toBe(0);
      expect(['NAME_NOT_FOUND', 'RECONCILIATION_FAILED']).toContain(errorCode(staleSwap));
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
          const swapRefused = await foreign.runJsonCli(['focus', 'member', '--into', 'left'], {
            pane: foreign.pane,
          });
          expect(errorCode(swapRefused)).toBe('NAME_NOT_FOUND');
          expect(clients(foreign).get(client)).toBe(before);
        },
        { globalDir: home.globalDir }
      );
      expect(fs.existsSync(home.globalDir)).toBe(true);
    });
  });
});
